import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import {
  bridgeExeLegacy,
  bridgeHealth,
  bridgeTaskStatus,
  bridgeVersions,
  globalProxyEnv,
  proxiesPing,
  shimStatus,
} from "./api";
import type {
  BridgeHealth,
  BridgeTaskStatus,
  BridgeVersions,
  GlobalProxyEnv,
  ProxyPing,
  ShimStatus,
} from "./types";

const POLL_INTERVAL_MS = 5_000;
/** Сэмплов истории трафика для sparkline (~3.3 мин при 5с-тике). */
const HISTORY_LEN = 40;
/** Пинг прокси-пула: раз в 5 минут (лёгкий TCP-connect, но не спамим). */
const POOL_PING_INTERVAL_MS = 5 * 60_000;
/** Не ответивший прокси переизмеряем раньше — через 20 с. */
const POOL_RECHECK_MS = 20_000;
/** Скан процессов (shimStatus) при живом /healthz — раз в N тиков (~30с):
    healthz и так доказывает, что мост жив, а скан — самый дорогой вызов. */
const PROCESS_SCAN_EVERY_TICKS = 6;
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
 * Одна правда о каждом прокси пула — для «Обзора», «Прокси», сайдбара,
 * трея и окна настроек приложения. Источники: настоящий запрос через
 * прокси (proxies_ping), TCP-пинг и живые счётчики моста.
 *   ok       — работает;
 *   slow     — работает, но долго подключается;
 *   unstable — часть подключений моста за минуту не прошла;
 *   down     — не работает (reason — почему);
 *   unknown  — ещё не проверяли.
 */
export type ProxyLive = "ok" | "slow" | "unstable" | "down" | "unknown";

export interface ProxyState {
  url: string;
  host: string;
  /** TCP-пинг, мс (null — порт не ответил, undefined — не мерили). */
  ms?: number | null;
  state: ProxyLive;
  reason?: string;
  /** Резерв какого прокси пула (индекс); undefined — сам прокси пула. */
  backupOf?: number;
  /** Прокси пула не работает, но его подменяет этот резерв (индекс). */
  coveredBy?: number;
}

/** Прокси пула (без резервов) — с исходными индексами. */
export const poolOnly = (proxies: ProxyState[]) => proxies.filter((p) => p.backupOf === undefined);

/** Медленно — дольше этого подключение к прокси. */
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
  // Чей резерв — из проверки пула, а пока её нет — из /healthz моста.
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
    // Настоящий запрос через прокси не прошёл (или порт закрыт) — не работает,
    // какой бы красивый ни был TCP-пинг.
    if (p && (p.alive === false || p.ms === null)) {
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
    // Своей проверки ещё нет — судим по мосту.
    if (q?.state === "down") return { ...base, state: "down" as const, reason: "Мост не может через него подключиться" };
    if (q?.state === "unstable") {
      return {
        ...base,
        state: "unstable" as const,
        reason: `${q.recentFail} из ${q.recentOk + q.recentFail} подключений за минуту не прошли`,
      };
    }
    return { ...base, state: q ? ("ok" as const) : ("unknown" as const) };
  });
  // Упавший прокси с живым резервом: трафик идёт через резерв (мост так
  // и делает — первый живой по порядку).
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
  /** Версии моста (null — ещё не узнали). */
  versions: BridgeVersions | null;
  /** Качество каждого прокси пула (по индексу health.upstreams). */
  quality: UpstreamQuality[];
  /** Состояние каждого прокси пула (по индексу) — одно на всё UI. */
  proxies: ProxyState[];
  /** Итоговое состояние для орба/логотипа/сайдбара/трея — одно на всё UI. */
  orb: OrbState;
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
}

const StatusContext = createContext<StatusContextValue | null>(null);

/*
  Один общий поллинг статусов на всё приложение (5с) вместо поллинга в каждой
  странице: сайдбар и трей-попап всегда видят живой статус, а страницы при
  переключении не начинают с нуля («Проверяю…» при каждом заходе).
*/
export function StatusProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<ShimStatus | null>(null);
  const [health, setHealth] = useState<BridgeHealth | null>(null);
  const [healthErr, setHealthErr] = useState<string | null>(null);
  const [task, setTask] = useState<BridgeTaskStatus | null>(null);
  const [env, setEnv] = useState<GlobalProxyEnv | null>(null);
  const [exeLegacy, setExeLegacy] = useState(false);
  const [versions, setVersions] = useState<BridgeVersions | null>(null);
  const [quality, setQuality] = useState<UpstreamQuality[]>([]);
  /** Окно дельт ok/fail по каждому прокси + прошлые абсолютные счётчики. */
  const qualityWindow = useRef<{ ok: number; fail: number }[][]>([]);
  const lastCounters = useRef<{ ok: number; fail: number }[] | null>(null);
  const tick = useRef(0);
  /** Когда пришло последнее событие bridge-health от монитора (мс). */
  const lastEvent = useRef(0);
  const [history, setHistory] = useState<TrafficSample[]>([]);
  const [poolPing, setPoolPing] = useState<ProxyPing[]>([]);
  const lastTotal = useRef<number | null>(null);
  /** Поколение refresh: устаревший ответ (интервальный тик, обогнанный ручным
      refresh после действия) не должен перезатирать более свежее состояние —
      иначе UI на 5с «откатывается» и sparkline рисует ложные всплески. */
  const gen = useRef(0);

  /** Повторный замер, если какой-то прокси не ответил: короткий сбой у
      провайдера не должен висеть красным «нет связи» до следующего круга. */
  const recheck = useRef<number | null>(null);
  // Проверка идёт до ~10 с (настоящий запрос через каждый прокси): вызовы,
  // пришедшие пока она идёт, ждут её же, а не запускают ещё одну.
  const inflight = useRef<Promise<void> | null>(null);
  const refreshPoolPing = useCallback(() => {
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
      if (recheck.current !== null) window.clearTimeout(recheck.current);
      recheck.current = list.some((p) => p.ms === null)
        ? window.setTimeout(() => {
            if (!document.hidden) void refreshPoolPingRef.current();
          }, POOL_RECHECK_MS)
        : null;
    } catch {
      /* пул может быть пуст / команда недоступна — не страшно */
    }
  };
  const refreshPoolPingRef = useRef(refreshPoolPing);
  refreshPoolPingRef.current = refreshPoolPing;

  /** Пересчитать качество прокси по дельтам счётчиков /healthz. */
  const wasHealthy = useRef<boolean[]>([]);
  const updateQuality = useCallback((h: BridgeHealth) => {
    // Мост только что перестал достукиваться до прокси — сразу проверяем его
    // по-настоящему, а не ждём планового круга (до 5 минут).
    const nowHealthy = h.upstreams.map((u) => u.healthy);
    if (nowHealthy.some((ok, i) => !ok && wasHealthy.current[i] !== false)) {
      void refreshPoolPingRef.current();
    }
    wasHealthy.current = nowHealthy;
    const prev = lastCounters.current;
    lastCounters.current = h.upstreams.map((u) => ({ ok: u.ok, fail: u.fail }));
    // Первый сэмпл / рестарт моста (счётчики упали) / сменился пул — окно с нуля.
    const reset =
      prev === null ||
      prev.length !== h.upstreams.length ||
      h.upstreams.some((u, i) => u.ok < prev[i].ok || u.fail < prev[i].fail);
    if (reset) qualityWindow.current = h.upstreams.map(() => []);
    else {
      h.upstreams.forEach((u, i) => {
        const w = qualityWindow.current[i];
        w.push({ ok: u.ok - prev[i].ok, fail: u.fail - prev[i].fail });
        if (w.length > QUALITY_WINDOW) w.shift();
      });
    }
    setQuality(
      h.upstreams.map((u, i) => {
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
      }),
    );
  }, []);

  /** Новый снимок /healthz → состояние, качество прокси, история трафика. */
  const applyHealth = useCallback(
    (h: BridgeHealth) => {
      setHealth(h);
      setHealthErr(null);
      updateQuality(h);
      const total = h.via_upstream + h.via_direct;
      const prev = lastTotal.current;
      lastTotal.current = total;
      // Первый сэмпл (или рестарт моста → счётчики упали) — дельту не пишем.
      if (prev !== null && total >= prev) {
        setHistory((old) =>
          [...old, { delta: total - prev, active: h.active }].slice(-HISTORY_LEN),
        );
      }
    },
    [updateQuality],
  );

  useEffect(() => {
    const un = listen<BridgeHealth>("bridge-health", (e) => {
      lastEvent.current = Date.now();
      // Без проверки document.hidden: WebKit иногда считает видимое окно
      // скрытым, и тогда событие терялось, а свой опрос тоже молчал —
      // данные в окне застывали на часы. Применить снимок дёшево.
      applyHealth(e.payload);
    });
    return () => {
      void un.then((f) => f());
    };
  }, [applyHealth]);

  /**
   * full=true — всё (при открытии окна и после действий пользователя);
   * full=false — интервальный тик: только здоровье моста и, редко, процесс.
   * Автозапуск, переменные окружения и версии сами по себе не меняются —
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
    // Монитор трея присылает /healthz событием каждые 5 с — сами опрашиваем
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
        setHealth(null);
        setHealthErr(String(err));
        lastTotal.current = null;
        lastCounters.current = null;
        setQuality([]);
        return false;
      });
    // Мост не ответил — нужен честный статус процесса прямо сейчас.
    if (full || !healthOk || t % PROCESS_SCAN_EVERY_TICKS === 0) {
      slow.push(shimStatus().then((v) => fresh() && setStatus(v)).catch(() => {}));
    }
    await Promise.all(slow);
  }, [applyHealth]);

  useEffect(() => {
    void refresh();
    void refreshPoolPing();
    // Скрытое окно не поллит: спрятанный трей-попап живёт вечно, и без этой
    // паузы он параллельно с главным окном дёргал бы мост каждые 5с и пинговал
    // весь пул прокси каждые 5 минут. При показе — обновляемся сразу.
    const id = window.setInterval(() => {
      if (!document.hidden) void refresh(false);
    }, POLL_INTERVAL_MS);
    const pingId = window.setInterval(() => {
      if (!document.hidden) void refreshPoolPing();
    }, POOL_PING_INTERVAL_MS);
    const onVisible = () => {
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(id);
      window.clearInterval(pingId);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refresh, refreshPoolPing]);

  const proxies = proxyStatesOf(poolPing, health, quality);
  const orb = orbStateOf(status, proxies);

  return (
    <StatusContext.Provider
      value={{
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
        history,
        poolPing,
        refresh,
        refreshPoolPing,
        setEnv,
      }}
    >
      {children}
    </StatusContext.Provider>
  );
}

export function useStatus(): StatusContextValue {
  const ctx = useContext(StatusContext);
  if (!ctx) throw new Error("useStatus must be used within StatusProvider");
  return ctx;
}

/** Единая логика статуса для орба/логотипа/трея: loading/down/warn/ok.
    warn — какой-то прокси пула не работает или нестабилен: мост жив, но
    часть трафика (закреплённая за этим прокси) может стоять. */
export function orbStateOf(status: ShimStatus | null, proxies: ProxyState[] = []): OrbState {
  if (status === null) return "loading";
  if (!status.running) return "down";
  if (proxies.some((p) => p.state === "down" || p.state === "unstable")) return "warn";
  return "ok";
}

export function formatUptime(sec?: number): string {
  if (!sec || sec < 0) return "—";
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  if (h > 0) return `${h}ч ${m}м`;
  if (m > 0) return `${m}м ${s}с`;
  return `${s}с`;
}
