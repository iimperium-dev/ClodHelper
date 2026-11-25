import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import {
  bridgeExeLegacy,
  bridgeFileIssue,
  bridgeHealth,
  bridgeTaskStatus,
  bridgeVersions,
  globalProxyEnv,
  proxiesPing,
  proxyAlertMute,
  proxyAlertsMutedGet,
  shimStatus,
  systemPacStatus,
  vpnStatus,
} from "./api";
import type {
  BridgeFileIssue,
  BridgeHealth,
  BridgeTaskStatus,
  BridgeVersions,
  GlobalProxyEnv,
  ProxyPing,
  ShimStatus,
  SystemPacStatus,
  VpnStatus,
} from "./types";

const POLL_INTERVAL_MS = 5_000;
/** Автонастройку сайтов проверяем каждый 3-й тик (~15 с). */
const PAC_CHECK_EVERY_TICKS = 3;
/** Сэмплов истории трафика для sparkline (~3.3 мин при 5с-тике). */
const HISTORY_LEN = 40;
/** Пинг прокси-пула: раз в 5 минут (лёгкий TCP-connect, но не спамим). */
const POOL_PING_INTERVAL_MS = 5 * 60_000;
/** Не ответивший прокси переизмеряем раньше - через 20 с. */
const POOL_RECHECK_MS = 20_000;
/** Скан процессов (shimStatus) при живом /healthz - раз в N тиков (~30с):
    healthz и так доказывает, что мост жив, а скан - самый дорогой вызов. */
const PROCESS_SCAN_EVERY_TICKS = 6;
/** VPN рядом (адаптеры, маршруты, процессы) - раз в N тиков (~30с). */
const VPN_CHECK_EVERY_TICKS = 6;
/** Окно оценки стабильности прокси: последние N тиков (~1 мин). */
const QUALITY_WINDOW = 12;
/** «Нестабилен»: доля отказов в окне выше порога при ≥2 отказах. */
const UNSTABLE_FAIL_RATE = 0.1;

/** Состояние одного прокси для UI: ok / нестабилен / не отвечает. */
export type UpstreamState = "ok" | "unstable" | "down";

export interface UpstreamQuality {
  state: UpstreamState;
  /** Успешных/неудачных подключений за окно (~1 мин). */
  recentOk: number;
  recentFail: number;
}

export type OrbState = "loading" | "ok" | "warn" | "down";

/**
 * Одна правда о каждом прокси пула - для «Обзора», «Прокси», сайдбара,
 * трея и окна настроек приложения. Источники: настоящий запрос через
 * прокси (proxies_ping), TCP-пинг и живые счётчики моста.
 *   ok       - работает;
 *   slow     - работает, но долго подключается;
 *   unstable - часть подключений моста за минуту не прошла;
 *   down     - не работает (reason - почему);
 *   unknown  - ещё не проверяли.
 */
export type ProxyLive = "ok" | "slow" | "unstable" | "down" | "unknown";

export interface ProxyState {
  url: string;
  host: string;
  /** TCP-пинг, мс (null - порт не ответил, undefined - не мерили). */
  ms?: number | null;
  state: ProxyLive;
  reason?: string;
  /** Резерв какого прокси пула (индекс); undefined - сам прокси пула. */
  backupOf?: number;
  /** Прокси пула не работает, но его подменяет этот резерв (индекс). */
  coveredBy?: number;
  /** Человек скрыл предупреждение о поломке этого прокси (знает причину,
      например, не оплатил). Проверки идут как раньше, но «Обзор», сайдбар
      и мигание орба его не считают. Заработает - метка снимется сама. */
  muted?: boolean;
}

/** Прокси пула (без резервов) - с исходными индексами. */
export const poolOnly = (proxies: ProxyState[]) => proxies.filter((p) => p.backupOf === undefined);

/** Медленно - дольше этого подключение к прокси. */
export const SLOW_MS = 1500;

/** Работает ли прокси (для выбора выхода и счёта запасных). */
export const isUsable = (s: ProxyLive) => s !== "down";

function hostOf(url: string): string {
  const rest = url.includes("://") ? url.split("://")[1] : url;
  const hp = rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest;
  const colon = hp.lastIndexOf(":");
  return colon > 0 ? hp.slice(0, colon) : hp;
}

/** Состояния прокси пула по порядку (индекс = индекс в пуле и в мосте). */
export function proxyStatesOf(
  poolPing: ProxyPing[],
  health: BridgeHealth | null,
  quality: UpstreamQuality[],
): ProxyState[] {
  const n = Math.max(poolPing.length, health?.upstreams.length ?? 0);
  // Чей резерв - из проверки пула, а пока её нет - из /healthz моста.
  const parentOf = (i: number): number | undefined => {
    const fromPing = poolPing[i]?.backup_of;
    if (typeof fromPing === "number") return fromPing;
    if (poolPing[i]) return undefined;
    const k = health?.upstreams.findIndex((u) => u.backups?.includes(i)) ?? -1;
    return k >= 0 ? k : undefined;
  };
  const states = Array.from({ length: n }, (_, i): ProxyState => {
    const p = poolPing[i];
    const q = quality[i];
    const url = p?.url ?? health?.upstreams[i]?.url ?? "";
    const base = { url, host: p?.host || hostOf(url), ms: p?.ms, backupOf: parentOf(i) };
    // Мост прямо сейчас ходит через этот прокси без сбоев - он работает,
    // что бы ни показала наша отдельная проверка (её тестовые адреса бывают
    // недоступны из страны пользователя или через конкретный прокси).
    const u = health?.upstreams[i];
    const bridgeProves = !!u && u.healthy && u.ok > 0 && (u.fail_streak ?? 0) === 0 && q?.state !== "down";
    // Настоящий запрос через прокси не прошёл (или порт закрыт) - не работает,
    // какой бы красивый ни был TCP-пинг.
    if (p && (p.alive === false || p.ms === null) && !bridgeProves) {
      return { ...base, state: "down" as const, reason: p.error ?? "Прокси не отвечает" };
    }
    if (p?.alive === true) {
      if (q && q.state !== "ok") {
        return {
          ...base,
          state: "unstable" as const,
          reason: `${q.recentFail} из ${q.recentOk + q.recentFail} подключений за минуту не прошли`,
        };
      }
      return { ...base, state: p.ms !== null && p.ms !== undefined && p.ms > SLOW_MS ? ("slow" as const) : ("ok" as const) };
    }
    // Своей проверки ещё нет - судим по мосту.
    if (q?.state === "down") return { ...base, state: "down" as const, reason: "Мост не может через него подключиться" };
    if (q?.state === "unstable") {
      return {
        ...base,
        state: "unstable" as const,
        reason: `${q.recentFail} из ${q.recentOk + q.recentFail} подключений за минуту не прошли`,
      };
    }
    return { ...base, state: q || bridgeProves ? ("ok" as const) : ("unknown" as const) };
  });
  // Упавший прокси с живым резервом: трафик идёт через резерв (мост так
  // и делает - первый живой по порядку).
  states.forEach((s, i) => {
    if (s.backupOf !== undefined || s.state !== "down") return;
    const j = states.findIndex((b) => b.backupOf === i && (b.state === "ok" || b.state === "slow"));
    if (j >= 0) s.coveredBy = j;
  });
  return states;
}

export interface TrafficSample {
  /** Обработано соединений за тик (дельта via_upstream+via_direct). */
  delta: number;
  /** Активных соединений в момент сэмпла. */
  active: number;
}

interface StatusContextValue {
  status: ShimStatus | null;
  health: BridgeHealth | null;
  healthErr: string | null;
  task: BridgeTaskStatus | null;
  env: GlobalProxyEnv | null;
  exeLegacy: boolean;
  /** Версии моста (null - ещё не узнали). */
  versions: BridgeVersions | null;
  /** Качество каждого прокси пула (по индексу health.upstreams). */
  quality: UpstreamQuality[];
  /** Состояние каждого прокси пула (по индексу) - одно на всё UI. */
  proxies: ProxyState[];
  /** Итоговое состояние для орба/логотипа/сайдбара/трея - одно на всё UI. */
  orb: OrbState;
  /** Мост работает, но часть прокси с проблемой: орб зелёный и раз в
      столько секунд мигает жёлтым (10 - один прокси, 5 - два и больше). */
  orbPulse: number | null;
  /** История трафика текущей сессии GUI (для sparkline). */
  history: TrafficSample[];
  /** Последний TCP-пинг пула прокси (обновляется раз в 5 минут). */
  poolPing: ProxyPing[];
  /** Принудительное обновление после действий (start/stop/toggle). */
  refresh: () => Promise<void>;
  /** Пингануть пул сейчас (после изменения пула / по кнопке). */
  refreshPoolPing: () => Promise<void>;
  /** Оптимистичная подмена env-статуса (после set/clear). */
  setEnv: (v: GlobalProxyEnv) => void;
  /** Файл моста пропал (антивирус): восстановлен / удаляется снова / нет копии. */
  bridgeFile: BridgeFileIssue | null;
  setBridgeFile: (v: BridgeFileIssue | null) => void;
  /** «Сайты во всех браузерах»: состояние системной автонастройки. */
  pac: SystemPacStatus | null;
  setPac: (v: SystemPacStatus | null) => void;
  /** Автонастройку включили, а другая программа её сбросила - сайты не работают. */
  sitesBroken: boolean;
  /** С какого момента (мс) не работает ни один прокси пула; null - работают. */
  poolDownSince: number | null;
  /** VPN рядом с мостом (null - ещё не проверяли). */
  vpn: VpnStatus | null;
  /** Скрыть или вернуть предупреждение о неработающем прокси (по URL). */
  setAlertMuted: (url: string, mute: boolean) => Promise<void>;
}

/** Хранилище статуса: провайдер кладёт сюда свежее значение, подписчики
    перерисовываются, только если изменилось то, что они читают. */
interface StatusStore {
  get: () => StatusContextValue;
  subscribe: (cb: () => void) => () => void;
}

const StatusContext = createContext<StatusStore | null>(null);

/** Одинаковы ли данные (снимки из Rust - простой JSON, без функций). */
function sameData(a: unknown, b: unknown): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}

/** Сеттер, который не трогает состояние, если пришло то же самое: окна
    не перерисовываются, когда цифры не изменились. */
function useSameSkippingState<T>(initial: T): [T, (v: T) => void] {
  const [v, set] = useState<T>(initial);
  const setIfChanged = useCallback((next: T) => set((old) => (sameData(old, next) ? old : next)), []);
  return [v, setIfChanged];
}

/** Окно скрыто (свёрнуто в трей, попап закрыт). Фокус - на случай WebKit,
    который иногда считает видимое окно скрытым. */
const windowHidden = () => document.hidden && !document.hasFocus();

/** Перерыв в снимках дольше этого (скрытым окнам монитор снимки не шлёт,
    компьютер спал) - снимок считаем первым: дельту за перерыв в историю не
    пишем (был бы один огромный столбик), окно качества - с нуля (иначе
    прокси мог бы мигнуть «нестабильным»). Обычный шаг - 5 с. */
const SAMPLE_GAP_MS = 15_000;
/** Скрытое окно применяет отложенный снимок не реже этого - если браузер
    ошибся со «скрыто», данные не застынут надолго. */
const HIDDEN_FLUSH_MS = 30_000;

/*
  Один общий поллинг статусов на всё приложение (5с) вместо поллинга в каждой
  странице: сайдбар и трей-попап всегда видят живой статус, а страницы при
  переключении не начинают с нуля («Проверяю…» при каждом заходе).

  Перерисовки: одинаковые ответы состояние не меняют; каждый useStatus()
  перерисовывается, только когда меняется то, что он сам читает (например,
  «Сайты» не трогает новый снимок счётчиков моста). Скрытое окно копит
  снимки без перерисовки и показывает их, когда его откроют.
*/
export function StatusProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useSameSkippingState<ShimStatus | null>(null);
  const [health, setHealth] = useState<BridgeHealth | null>(null);
  const [healthErr, setHealthErr] = useState<string | null>(null);
  const [task, setTask] = useSameSkippingState<BridgeTaskStatus | null>(null);
  const [env, setEnv] = useSameSkippingState<GlobalProxyEnv | null>(null);
  const [exeLegacy, setExeLegacy] = useState(false);
  const [versions, setVersions] = useSameSkippingState<BridgeVersions | null>(null);
  const [quality, setQuality] = useSameSkippingState<UpstreamQuality[]>([]);
  /** Окно дельт ok/fail по каждому прокси + прошлые абсолютные счётчики. */
  const qualityWindow = useRef<{ ok: number; fail: number }[][]>([]);
  const lastCounters = useRef<{ ok: number; fail: number }[] | null>(null);
  const tick = useRef(0);
  /** Когда пришло последнее событие bridge-health от монитора (мс). */
  const lastEvent = useRef(0);
  const [history, setHistory] = useState<TrafficSample[]>([]);
  /** История копится здесь и в скрытом окне; в состояние - при показе. */
  const historyBuf = useRef<TrafficSample[]>([]);
  const [poolPing, setPoolPing] = useSameSkippingState<ProxyPing[]>([]);
  /** URL прокси со скрытым предупреждением о поломке. */
  const [mutedUrls, setMutedUrls] = useSameSkippingState<string[]>([]);
  const setAlertMuted = useCallback(async (url: string, mute: boolean) => {
    setMutedUrls(await proxyAlertMute(url, mute));
  }, [setMutedUrls]);
  const [bridgeFile, setBridgeFile] = useSameSkippingState<BridgeFileIssue | null>(null);
  const [pac, setPac] = useSameSkippingState<SystemPacStatus | null>(null);
  const [vpn, setVpn] = useSameSkippingState<VpnStatus | null>(null);
  const lastTotal = useRef<number | null>(null);
  /** Когда приняли последний снимок (мс) - для перерыва SAMPLE_GAP_MS. */
  const lastSampleAt = useRef(0);
  /** Поколение refresh: устаревший ответ (интервальный тик, обогнанный ручным
      refresh после действия) не должен перезатирать более свежее состояние -
      иначе UI на 5с «откатывается» и sparkline рисует ложные всплески. */
  const gen = useRef(0);

  /** Повторный замер, если какой-то прокси не ответил: короткий сбой у
      провайдера не должен висеть красным «нет связи» до следующего круга. */
  const recheck = useRef<number | null>(null);
  /** Проверку пула попросили, пока окно скрыто: сделаем при показе. Скрытый
      трей-попап не проверяет пул параллельно с главным окном. */
  const pingWanted = useRef(false);
  // Проверка идёт до ~10 с (настоящий запрос через каждый прокси): вызовы,
  // пришедшие пока она идёт, ждут её же, а не запускают ещё одну.
  const inflight = useRef<Promise<void> | null>(null);
  const refreshPoolPing = useCallback(() => {
    pingWanted.current = false;
    if (!inflight.current) {
      inflight.current = runPoolPing().finally(() => {
        inflight.current = null;
      });
    }
    return inflight.current;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const runPoolPing = async () => {
    try {
      const list = await proxiesPing();
      setPoolPing(list);
      // После проверки: заработавший прокси Rust уже снял со скрытых.
      proxyAlertsMutedGet().then(setMutedUrls, () => {});
      if (recheck.current !== null) window.clearTimeout(recheck.current);
      recheck.current = list.some((p) => p.ms === null)
        ? window.setTimeout(() => {
            if (!document.hidden) void refreshPoolPingRef.current();
          }, POOL_RECHECK_MS)
        : null;
    } catch {
      /* пул может быть пуст / команда недоступна - не страшно */
    }
  };
  const refreshPoolPingRef = useRef(refreshPoolPing);
  refreshPoolPingRef.current = refreshPoolPing;
  /** Проверить пул сейчас, а в скрытом окне - когда его покажут. */
  const wantPoolPing = useCallback(() => {
    if (windowHidden()) pingWanted.current = true;
    else void refreshPoolPingRef.current();
  }, []);

  /** Пересчитать качество прокси по дельтам счётчиков /healthz. */
  const wasHealthy = useRef<boolean[]>([]);
  const qualityOf = useCallback((h: BridgeHealth, gap: boolean): UpstreamQuality[] => {
    // Мост только что перестал достукиваться до прокси - сразу проверяем его
    // по-настоящему, а не ждём планового круга (до 5 минут).
    const nowHealthy = h.upstreams.map((u) => u.healthy);
    if (nowHealthy.some((ok, i) => !ok && wasHealthy.current[i] !== false)) wantPoolPing();
    wasHealthy.current = nowHealthy;
    const prev = lastCounters.current;
    lastCounters.current = h.upstreams.map((u) => ({ ok: u.ok, fail: u.fail }));
    // Первый сэмпл / рестарт моста (счётчики упали) / сменился пул / долгий
    // перерыв - окно с нуля.
    const reset =
      gap ||
      prev === null ||
      prev.length !== h.upstreams.length ||
      h.upstreams.some((u, i) => u.ok < prev[i].ok || u.fail < prev[i].fail);
    if (reset || !prev) qualityWindow.current = h.upstreams.map(() => []);
    else {
      h.upstreams.forEach((u, i) => {
        const w = qualityWindow.current[i];
        w.push({ ok: u.ok - prev[i].ok, fail: u.fail - prev[i].fail });
        if (w.length > QUALITY_WINDOW) w.shift();
      });
    }
    return h.upstreams.map((u, i) => {
      const w = qualityWindow.current[i] ?? [];
      const recentOk = w.reduce((a, d) => a + d.ok, 0);
      const recentFail = w.reduce((a, d) => a + d.fail, 0);
      const rate = recentFail / Math.max(1, recentOk + recentFail);
      const state: UpstreamState = !u.healthy
        ? "down"
        : recentFail >= 2 && rate > UNSTABLE_FAIL_RATE
          ? "unstable"
          : "ok";
      return { state, recentOk, recentFail };
    });
  }, [wantPoolPing]);

  /** Снимок, принятый, но ещё не показанный (окно скрыто). */
  const pending = useRef<{ health: BridgeHealth; quality: UpstreamQuality[] } | null>(null);
  const flushTimer = useRef<number | null>(null);
  /** Последний принятый снимок: тот же самый ещё раз (повтор события,
      опрос сразу после события) ничего не меняет и не перерисовывает. */
  const lastHealthJson = useRef("");
  /** Показать накопленное: снимок, качество, историю. */
  const flush = useCallback(() => {
    if (flushTimer.current !== null) {
      window.clearTimeout(flushTimer.current);
      flushTimer.current = null;
    }
    const p = pending.current;
    if (!p) return;
    pending.current = null;
    setHealth(p.health);
    setQuality(p.quality);
    setHistory(historyBuf.current.slice());
    setHealthErr(null);
  }, [setQuality]);

  /** Новый снимок /healthz → качество прокси, история трафика; на экран -
      сразу, а в скрытом окне - когда его покажут. */
  const applyHealth = useCallback(
    (h: BridgeHealth) => {
      const json = JSON.stringify(h);
      if (json === lastHealthJson.current) return;
      lastHealthJson.current = json;
      const now = Date.now();
      const gap = lastSampleAt.current > 0 && now - lastSampleAt.current > SAMPLE_GAP_MS;
      lastSampleAt.current = now;
      const q = qualityOf(h, gap);
      const total = h.via_upstream + h.via_direct;
      const prev = lastTotal.current;
      lastTotal.current = total;
      // Первый сэмпл, рестарт моста (счётчики упали) или долгий перерыв -
      // дельту не пишем.
      if (prev !== null && total >= prev && !gap) {
        historyBuf.current = [...historyBuf.current, { delta: total - prev, active: h.active }].slice(-HISTORY_LEN);
      }
      pending.current = { health: h, quality: q };
      if (!windowHidden()) flush();
      else if (flushTimer.current === null) flushTimer.current = window.setTimeout(flush, HIDDEN_FLUSH_MS);
    },
    [qualityOf, flush],
  );

  // Программа восстановила мост или антивирус удаляет его снова - сразу в UI.
  useEffect(() => {
    const un = listen<BridgeFileIssue | null>("bridge-file", (e) => setBridgeFile(e.payload));
    return () => {
      void un.then((f) => f());
    };
  }, [setBridgeFile]);

  useEffect(() => {
    const un = listen<BridgeHealth>("bridge-health", (e) => {
      lastEvent.current = Date.now();
      // Снимок принимаем всегда (история и качество не рвутся), а в скрытом
      // окне откладываем только перерисовку. WebKit иногда считает видимое
      // окно скрытым - тогда показ догонит фокус, мышь или таймер.
      applyHealth(e.payload);
    });
    return () => {
      void un.then((f) => f());
    };
  }, [applyHealth]);

  /**
   * full=true - всё (при открытии окна и после действий пользователя);
   * full=false - интервальный тик: только здоровье моста и, редко, процесс.
   * Автозапуск, переменные окружения и версии сами по себе не меняются -
   * раньше на них каждые 5с уходило 3 запуска launchctl.
   */
  const refresh = useCallback(async (full = true) => {
    const g = ++gen.current;
    const fresh = () => gen.current === g;
    const t = tick.current++;
    const slow: Promise<unknown>[] = full
      ? [
          bridgeTaskStatus().then((v) => fresh() && setTask(v)).catch(() => {}),
          globalProxyEnv().then((v) => fresh() && setEnv(v)).catch(() => {}),
          bridgeExeLegacy().then((v) => fresh() && setExeLegacy(v)).catch(() => {}),
          bridgeVersions().then((v) => fresh() && setVersions(v)).catch(() => {}),
        ]
      : [];
    // Монитор трея присылает /healthz событием каждые 5 с - сами опрашиваем
    // мост, только если событий давно не было (монитор не запущен и т.п.).
    const viaEvents = !full && Date.now() - lastEvent.current < 8_000;
    const healthOk = viaEvents
      ? true
      : await bridgeHealth()
      .then((h) => {
        if (!fresh()) return true;
        applyHealth(h);
        return true;
      })
      .catch((err) => {
        if (!fresh()) return false;
        pending.current = null;
        lastHealthJson.current = "";
        setHealth(null);
        setHealthErr(String(err));
        lastTotal.current = null;
        lastCounters.current = null;
        setQuality([]);
        return false;
      });
    // Мост не ответил - нужен честный статус процесса прямо сейчас.
    if (full || !healthOk || t % PROCESS_SCAN_EVERY_TICKS === 0) {
      slow.push(shimStatus().then((v) => fresh() && setStatus(v)).catch(() => {}));
    }
    // Автонастройку сайтов может молча сбросить другая программа (VPN,
    // клиент прокси) - проверяем сами, а не только при заходе на «Сайты».
    if (full || t % PAC_CHECK_EVERY_TICKS === 0) {
      slow.push(systemPacStatus().then((v) => fresh() && setPac(v)).catch(() => {}));
    }
    // VPN включают и выключают в любой момент - смотрим раз в ~30 с.
    if (full || t % VPN_CHECK_EVERY_TICKS === 0) {
      slow.push(vpnStatus().then((v) => fresh() && setVpn(v)).catch(() => {}));
    }
    // Мост не отвечает - не удалил ли его антивирус (событие могло прийти,
    // пока окна не было).
    if (full || !healthOk) {
      slow.push(bridgeFileIssue().then((v) => fresh() && setBridgeFile(v)).catch(() => {}));
    }
    await Promise.all(slow);
  }, [applyHealth, setTask, setEnv, setVersions, setStatus, setPac, setVpn, setBridgeFile, setQuality]);

  useEffect(() => {
    void refresh();
    // Скрытые предупреждения - сразу, не дожидаясь проверки пула.
    proxyAlertsMutedGet().then(setMutedUrls, () => {});
    // Скрытое окно (трей-попап живёт спрятанным, --tray) проверяет пул,
    // только когда его покажут: главное окно делает это и так.
    wantPoolPing();
    // Скрытое окно не поллит: спрятанный трей-попап живёт вечно, и без этой
    // паузы он параллельно с главным окном дёргал бы мост каждые 5с и пинговал
    // весь пул прокси каждые 5 минут. При показе - обновляемся сразу.
    const id = window.setInterval(() => {
      if (!document.hidden) void refresh(false);
    }, POLL_INTERVAL_MS);
    const pingId = window.setInterval(() => {
      if (!document.hidden) void refreshPoolPing();
    }, POOL_PING_INTERVAL_MS);
    const onShown = () => {
      if (windowHidden()) return;
      flush();
      if (pingWanted.current) void refreshPoolPing();
    };
    const onVisible = () => {
      onShown();
      if (!document.hidden) void refresh();
    };
    // Мышь над окном - оно точно на виду (страховка от ошибки WebKit).
    const onPointer = () => {
      if (pending.current) flush();
      if (pingWanted.current) void refreshPoolPing();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onShown);
    window.addEventListener("pointermove", onPointer, { passive: true });
    return () => {
      window.clearInterval(id);
      window.clearInterval(pingId);
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onShown);
      window.removeEventListener("pointermove", onPointer);
    };
  }, [refresh, refreshPoolPing, wantPoolPing, flush, setMutedUrls]);

  // Состояния прокси: новый массив, только если что-то правда поменялось -
  // иначе «Прокси» и «Сайты» перерисовывались бы на каждый снимок моста.
  const prevProxies = useRef<ProxyState[]>([]);
  const proxies = useMemo(() => {
    const next = proxyStatesOf(poolPing, health, quality).map((p) =>
      mutedUrls.includes(p.url) ? { ...p, muted: true } : p,
    );
    if (sameData(next, prevProxies.current)) return prevProxies.current;
    prevProxies.current = next;
    return next;
  }, [poolPing, health, quality, mutedUrls]);
  const orb = orbStateOf(status, proxies);
  const orbPulse = orbPulseOf(orb, proxies);
  // Отсчёт «прокси не работает уже N минут» - для баннера по ситуации.
  const [poolDownSince, setPoolDownSince] = useState<number | null>(null);
  const poolDown = orb === "warn";
  useEffect(() => {
    setPoolDownSince((t) => (poolDown ? t ?? Date.now() : null));
  }, [poolDown]);
  const sitesBroken = !!pac?.supported && pac.enabled && !pac.active;

  const value = useMemo<StatusContextValue>(
    () => ({
      status,
      health,
      healthErr,
      task,
      env,
      exeLegacy,
      versions,
      quality,
      proxies,
      orb,
      orbPulse,
      history,
      poolPing,
      refresh,
      refreshPoolPing,
      setEnv,
      bridgeFile,
      setBridgeFile,
      pac,
      setPac,
      sitesBroken,
      poolDownSince,
      vpn,
      setAlertMuted,
    }),
    [
      status, health, healthErr, task, env, exeLegacy, versions, quality, proxies, orb, orbPulse,
      history, poolPing, refresh, refreshPoolPing, setEnv, bridgeFile, setBridgeFile, pac, setPac,
      sitesBroken, poolDownSince, vpn, setAlertMuted,
    ],
  );

  // Хранилище одно на провайдер; значение кладём при рендере (дети, которые
  // рендерятся в этом же проходе, видят свежее), подписчикам говорим после.
  const store = useRef<{ v: StatusContextValue; subs: Set<() => void>; api: StatusStore } | null>(null);
  if (store.current === null) {
    const s = { v: value, subs: new Set<() => void>() } as { v: StatusContextValue; subs: Set<() => void>; api: StatusStore };
    s.api = {
      get: () => s.v,
      subscribe: (cb) => {
        s.subs.add(cb);
        return () => s.subs.delete(cb);
      },
    };
    store.current = s;
  }
  store.current.v = value;
  useLayoutEffect(() => {
    for (const cb of store.current!.subs) cb();
  }, [value]);

  return <StatusContext.Provider value={store.current.api}>{children}</StatusContext.Provider>;
}

/**
 * Статус для компонента. Компонент перерисовывается, только когда меняется
 * одно из полей, которые он читает (запоминаем их при чтении): например,
 * «Сайты» читают только pac - снимок счётчиков моста их не трогает.
 */
export function useStatus(): StatusContextValue {
  const store = useContext(StatusContext);
  if (!store) throw new Error("useStatus must be used within StatusProvider");
  const used = useRef<Set<keyof StatusContextValue> | null>(null);
  if (used.current === null) used.current = new Set();
  const keys = used.current;
  const last = useRef<StatusContextValue | null>(null);
  const snapshot = useCallback(() => {
    const cur = store.get();
    const prev = last.current;
    if (prev !== null && prev !== cur) {
      let same = true;
      for (const k of keys) {
        if (prev[k] !== cur[k]) {
          same = false;
          break;
        }
      }
      if (same) return prev;
    }
    last.current = cur;
    return cur;
  }, [store, keys]);
  const snap = useSyncExternalStore(store.subscribe, snapshot, snapshot);
  return useMemo(
    () =>
      new Proxy(snap, {
        get(_t, k) {
          if (typeof k === "string") keys.add(k as keyof StatusContextValue);
          // Всегда свежее значение: поле могли впервые прочитать позже рендера.
          return store.get()[k as keyof StatusContextValue];
        },
      }),
    [snap, store, keys],
  );
}

/** Сколько прокси с проблемой: не отвечает или нестабилен (резервы тоже).
    Скрытые человеком не в счёт. */
export function proxyProblems(proxies: ProxyState[]): number {
  return proxies.filter((p) => !p.muted && (p.state === "down" || p.state === "unstable")).length;
}

/** Единая логика статуса для орба/логотипа/трея: loading/down/warn/ok.
    down - мост остановлен. warn - не работает ни один прокси пула (и
    резерв не подменил). Если не работает только часть прокси, это ok:
    трафик идёт, о проблеме говорит редкое жёлтое мигание (orbPulseOf). */
export function orbStateOf(status: ShimStatus | null, proxies: ProxyState[] = []): OrbState {
  if (status === null) return "loading";
  if (!status.running) return "down";
  const pool = poolOnly(proxies);
  if (pool.length > 0 && pool.every((p) => p.state === "down" && p.coveredBy === undefined)) return "warn";
  return "ok";
}

/** Период жёлтого мигания зелёного орба, с: null - проблем нет. */
export function orbPulseOf(orb: OrbState, proxies: ProxyState[]): number | null {
  if (orb !== "ok") return null;
  const n = proxyProblems(proxies);
  return n === 0 ? null : n === 1 ? 10 : 5;
}

export function formatUptime(sec?: number): string {
  if (!sec || sec < 0) return "-";
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  if (h > 0) return `${h}ч ${m}м`;
  if (m > 0) return `${m}м ${s}с`;
  return `${s}с`;
}
