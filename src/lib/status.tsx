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
  const refreshPoolPing = useCallback(async () => {
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
  }, []);
  const refreshPoolPingRef = useRef(refreshPoolPing);
  refreshPoolPingRef.current = refreshPoolPing;

  /** Пересчитать качество прокси по дельтам счётчиков /healthz. */
  const updateQuality = useCallback((h: BridgeHealth) => {
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

  const orb = orbStateOf(status, health, quality);

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
    warn — прокси не отвечает ИЛИ нестабилен (часть подключений за минуту
    не прошла), хотя формально ещё «healthy». */
export function orbStateOf(
  status: ShimStatus | null,
  health: BridgeHealth | null,
  quality: UpstreamQuality[] = [],
): OrbState {
  if (status === null) return "loading";
  if (!status.running) return "down";
  if (health && health.upstreams.some((u) => !u.healthy)) return "warn";
  if (quality.some((q) => q.state !== "ok")) return "warn";
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
