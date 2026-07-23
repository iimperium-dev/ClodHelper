import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  bridgeExeLegacy,
  bridgeHealth,
  bridgeTaskStatus,
  globalProxyEnv,
  proxiesPing,
  shimStatus,
} from "./api";
import type {
  BridgeHealth,
  BridgeTaskStatus,
  GlobalProxyEnv,
  ProxyPing,
  ShimStatus,
} from "./types";

const POLL_INTERVAL_MS = 5_000;
/** Сэмплов истории трафика для sparkline (~3.3 мин при 5с-тике). */
const HISTORY_LEN = 40;
/** Пинг прокси-пула: раз в 5 минут (лёгкий TCP-connect, но не спамим). */
const POOL_PING_INTERVAL_MS = 5 * 60_000;

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
  const [history, setHistory] = useState<TrafficSample[]>([]);
  const [poolPing, setPoolPing] = useState<ProxyPing[]>([]);
  const lastTotal = useRef<number | null>(null);
  /** Поколение refresh: устаревший ответ (интервальный тик, обогнанный ручным
      refresh после действия) не должен перезатирать более свежее состояние —
      иначе UI на 5с «откатывается» и sparkline рисует ложные всплески. */
  const gen = useRef(0);

  const refreshPoolPing = useCallback(async () => {
    try {
      setPoolPing(await proxiesPing());
    } catch {
      /* пул может быть пуст / команда недоступна — не страшно */
    }
  }, []);

  const refresh = useCallback(async () => {
    const g = ++gen.current;
    const fresh = () => gen.current === g;
    // Каждый статус независим: медленный вызов не держит остальные.
    await Promise.all([
      shimStatus().then((v) => fresh() && setStatus(v)).catch(() => {}),
      bridgeTaskStatus().then((v) => fresh() && setTask(v)).catch(() => {}),
      globalProxyEnv().then((v) => fresh() && setEnv(v)).catch(() => {}),
      bridgeExeLegacy().then((v) => fresh() && setExeLegacy(v)).catch(() => {}),
      bridgeHealth()
        .then((h) => {
          if (!fresh()) return;
          setHealth(h);
          setHealthErr(null);
          const total = h.via_upstream + h.via_direct;
          const prev = lastTotal.current;
          lastTotal.current = total;
          // Первый сэмпл (или рестарт моста → счётчики упали) — дельту не пишем.
          if (prev !== null && total >= prev) {
            setHistory((old) =>
              [...old, { delta: total - prev, active: h.active }].slice(-HISTORY_LEN),
            );
          }
        })
        .catch((err) => {
          if (!fresh()) return;
          setHealth(null);
          setHealthErr(String(err));
          lastTotal.current = null;
        }),
    ]);
  }, []);

  useEffect(() => {
    void refresh();
    void refreshPoolPing();
    // Скрытое окно не поллит: спрятанный трей-попап живёт вечно, и без этой
    // паузы он параллельно с главным окном дёргал бы мост каждые 5с и пинговал
    // весь пул прокси каждые 5 минут. При показе — обновляемся сразу.
    const id = window.setInterval(() => {
      if (!document.hidden) void refresh();
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

  return (
    <StatusContext.Provider
      value={{
        status,
        health,
        healthErr,
        task,
        env,
        exeLegacy,
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

/** Единая логика статуса для орба/логотипа/трея: loading/down/warn/ok. */
export function orbStateOf(
  status: ShimStatus | null,
  health: BridgeHealth | null,
): "loading" | "ok" | "warn" | "down" {
  if (status === null) return "loading";
  if (!status.running) return "down";
  if (health && health.upstreams.some((u) => !u.healthy)) return "warn";
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
