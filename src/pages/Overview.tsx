import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  ChevronDown,
  ChevronRight,
  Loader2,
  Pause,
  Play,
  RotateCw,
} from "lucide-react";
import { Button } from "../components/Button";
import { Toggle } from "../components/Toggle";
import { StatusOrb } from "../components/StatusOrb";
import { NumberTicker } from "../components/NumberTicker";
import { Sparkline } from "../components/Sparkline";
import {
  bridgeSetRouteMode,
  bridgeTaskRegister,
  bridgeUpdate,
  clearGlobalProxyEnv,
  setGlobalProxyEnv,
  shimRestart,
  shimStart,
  shimStop,
  shimTest,
} from "../lib/api";
import { formatUptime, useStatus } from "../lib/status";
import type { ShimTestResult } from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { cn } from "../lib/cn";

interface Props {
  /** Переход в раздел «Прокси» (клик по строке пула). */
  onOpenProxies: () => void;
}

/** host:port из masked-URL пула ("http://user:****@host:port"). */
function chipLabel(masked: string): string {
  const rest = masked.includes("://") ? masked.split("://")[1] : masked;
  return rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest;
}

/** "N сек/мин/ч назад" для списка ошибок (t — секунды-от-старта моста). */
function agoLabel(uptimeSec: number, t: number): string {
  const d = Math.max(0, uptimeSec - t);
  if (d < 60) return `${d} сек назад`;
  if (d < 3600) return `${Math.floor(d / 60)} мин назад`;
  return `${Math.floor(d / 3600)} ч назад`;
}

/** Техническую строку моста — в короткую человеческую причину. */
function humanError(msg: string): string {
  if (/unreachable|No route|os error 51|os error 65/i.test(msg)) return "Не было интернета на Mac";
  if (/логин\/пароль|407/.test(msg)) return "Прокси не принял логин или пароль";
  if (/молчит на CONNECT|нет ответа на TCP|не отвечает/.test(msg)) return "Прокси не отвечал";
  if (/ответил 5\d\d/.test(msg)) return "Прокси не смог открыть сайт";
  if (/прямое соединение/.test(msg)) return "Сайт не открылся напрямую";
  if (/лимит/.test(msg)) return "Слишком много соединений сразу";
  return msg.length > 60 ? `${msg.slice(0, 57)}…` : msg;
}

/** Сгруппировать одинаковые причины: свежие — первыми, с количеством. */
function errorGroups(
  errors: { t: number; msg: string }[],
  uptimeSec: number,
): { text: string; count: number; ago: string; fresh: boolean }[] {
  const map = new Map<string, { text: string; count: number; t: number }>();
  for (const e of errors) {
    const text = humanError(e.msg);
    const g = map.get(text);
    if (g) {
      g.count += 1;
      g.t = Math.max(g.t, e.t);
    } else {
      map.set(text, { text, count: 1, t: e.t });
    }
  }
  return [...map.values()]
    .sort((a, b) => b.t - a.t)
    .map((g) => ({
      text: g.text,
      count: g.count,
      ago: agoLabel(uptimeSec, g.t),
      // Старые перебои не должны вечно гореть оранжевым.
      fresh: uptimeSec - g.t < 30 * 60,
    }));
}

/*
  Обзор — hero-зона статуса (НЕ карточка: открытая композиция с орбом),
  живые метрики с тикерами и sparkline, и группа управления одной
  поверхностью с hairline-разделителями (вместо трёх отдельных карточек).
  Описания «для чайника» живут в «?»-подсказках, не в инлайн-тексте.
*/
export function Overview({ onOpenProxies }: Props) {
  const {
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
    setEnv,
  } = useStatus();

  const [busy, setBusy] = useState<"start" | "stop" | "restart" | "test" | null>(null);
  const [testResult, setTestResult] = useState<ShimTestResult | null>(null);
  const [envBusy, setEnvBusy] = useState(false);
  const [routeBusy, setRouteBusy] = useState(false);
  const [taskBusy, setTaskBusy] = useState(false);
  const [updatingBridge, setUpdatingBridge] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [errorsOpen, setErrorsOpen] = useState(false);

  const loading = status === null;
  const running = status?.running ?? false;
  // Прокси, у которых за последнюю минуту часть подключений не прошла.
  const shaky = quality.filter((q) => q.state !== "ok");
  const shakyFails = shaky.reduce((a, q) => a + q.recentFail, 0);
  const shakyTotal = shaky.reduce((a, q) => a + q.recentOk + q.recentFail, 0);
  const bridgeOutdated = running && (versions?.outdated ?? false);

  const envOn = (env?.present ?? false) && (env?.points_to_bridge ?? false);
  const envForeign = (env?.present ?? false) && !(env?.points_to_bridge ?? false);
  const smartOn = health?.mode === "smart";
  const oldBridge =
    !health && (exeLegacy || (healthErr !== null && healthErr.includes("разобрать")));

  // ── Действия моста ──
  // Все команды могут реально упасть (порт занят, exe удалён, PowerShell
  // отказал) — без catch клик выглядел бы как «нажал и ничего»: спиннер
  // мигнул, ошибки нет. Показываем её строкой под hero.
  const [actionError, setActionError] = useState<string | null>(null);

  const bridgeAction = async (
    kind: "start" | "stop" | "restart",
    fn: () => Promise<unknown>,
  ) => {
    setBusy(kind);
    setActionError(null);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const handleTest = async () => {
    setBusy("test");
    setTestResult(null);
    setActionError(null);
    try {
      setTestResult(await shimTest());
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const handleEnvToggle = async () => {
    setEnvBusy(true);
    setActionError(null);
    try {
      setEnv(envOn ? await clearGlobalProxyEnv() : await setGlobalProxyEnv());
    } catch (e) {
      setActionError(String(e));
    } finally {
      setEnvBusy(false);
    }
  };

  const handleRouteToggle = async () => {
    setRouteBusy(true);
    setActionError(null);
    try {
      await bridgeSetRouteMode(smartOn ? "all" : "smart");
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setRouteBusy(false);
    }
  };

  const handleTaskRegister = async () => {
    setTaskBusy(true);
    setActionError(null);
    try {
      await bridgeTaskRegister();
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setTaskBusy(false);
    }
  };

  const handleBridgeUpdate = async () => {
    setUpdatingBridge(true);
    setUpdateError(null);
    try {
      await bridgeUpdate();
      await refresh();
    } catch (e) {
      setUpdateError(String(e));
    } finally {
      setUpdatingBridge(false);
    }
  };

  // Прокси, через который сейчас выходим: основной из пула. IP — ровно
  // одно место на экране; после «Проверить» — реальный выходной IP и время.
  const primaryHost = poolPing[0]
    ? poolPing[0].host
    : health?.upstreams[0]
      ? chipLabel(health.upstreams[0].url).split(":")[0]
      : null;
  const primaryState = quality[0]?.state ?? (health?.upstreams[0]?.healthy === false ? "down" : "ok");
  const exitIp = testResult?.ok && testResult.external_ip ? testResult.external_ip : primaryHost;
  const exitMs = testResult?.ok ? testResult.latency_ms : (poolPing[0]?.ms ?? undefined);
  const spare = Math.max(0, (health?.upstreams.length ?? poolPing.length) - 1);

  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[460px] flex-col px-6 py-6"
    >
      {/* ── Статус ── */}
      <motion.section variants={fadeInUp} className="flex items-center gap-3">
        <StatusOrb state={orb} size={12} />
        <div className="min-w-0 flex-1">
          <h1 className="text-[20px] font-semibold leading-tight tracking-[-0.02em] text-vb-fg">
            {loading ? "Проверяю…" : running ? "Мост активен" : "Мост остановлен"}
          </h1>
          <p className="tnum text-[12px] text-vb-silver-faint">
            {running
              ? `работает ${formatUptime(health?.uptime_sec ?? status?.uptime_sec)}`
              : loading
                ? " "
                : "Трафик не проксируется"}
          </p>
        </div>
        {!loading &&
          (running ? (
            <div className="flex shrink-0 items-center">
              <button
                type="button"
                onClick={() => bridgeAction("restart", shimRestart)}
                disabled={busy !== null}
                title="Перезапустить мост"
                className="rounded-lg p-2 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.94] disabled:opacity-40"
              >
                <RotateCw className={cn("h-4 w-4", busy === "restart" && "animate-spin")} />
              </button>
              <button
                type="button"
                onClick={() => bridgeAction("stop", shimStop)}
                disabled={busy !== null}
                title="Остановить мост"
                className="rounded-lg p-2 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.94] disabled:opacity-40"
              >
                <Pause className="h-4 w-4" />
              </button>
            </div>
          ) : (
            <Button
              variant="primary"
              size="sm"
              onClick={() => bridgeAction("start", shimStart)}
              disabled={busy !== null}
            >
              <Play className="h-3.5 w-3.5" />
              {busy === "start" ? "Запуск…" : "Запустить"}
            </Button>
          ))}
      </motion.section>

      <AnimatePresence>
        {actionError && (
          <motion.p
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            className="mt-2 overflow-hidden text-[12px] text-vb-loss"
          >
            {actionError}
          </motion.p>
        )}
      </AnimatePresence>

      {/* ── Выход в интернет: единственное место с IP ── */}
      {running && (
        <motion.section variants={fadeInUp} className="surface-card mt-5">
          <div className="flex items-center gap-3 p-4">
            <button
              type="button"
              onClick={onOpenProxies}
              className="group flex min-w-0 flex-1 items-center gap-3 text-left"
            >
              <span
                className={cn(
                  "h-2 w-2 shrink-0 rounded-full",
                  testResult && !testResult.ok
                    ? "bg-vb-loss"
                    : primaryState === "ok"
                      ? "bg-vb-emerald"
                      : primaryState === "unstable"
                        ? "bg-vb-warn"
                        : "bg-vb-loss",
                )}
              />
              <span className="min-w-0">
                <span className="block text-[11px] text-vb-silver-faint">
                  Выход в интернет{spare > 0 && ` · +${spare} запасн.`}
                </span>
                <span className="flex items-baseline gap-2">
                  <span className="truncate font-mono text-[14px] text-vb-fg">
                    {exitIp ?? "прокси не настроен"}
                  </span>
                  {exitMs !== undefined && exitMs !== null && (
                    <span className="tnum shrink-0 font-mono text-[11px] text-vb-silver-faint">
                      {exitMs} мс
                    </span>
                  )}
                </span>
              </span>
              <ChevronRight className="ml-auto h-4 w-4 shrink-0 text-vb-silver-faint transition-transform group-hover:translate-x-0.5" />
            </button>
            <Button variant="secondary" size="sm" onClick={handleTest} disabled={busy !== null}>
              {busy === "test" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
              Проверить
            </Button>
          </div>
          {(testResult?.error || shaky.length > 0) && (
            <p
              className={cn(
                "border-t border-vb-border/70 px-4 py-2.5 text-[12px] leading-snug",
                testResult?.error ? "text-vb-loss" : "text-vb-warn",
              )}
            >
              {testResult?.error
                ? testResult.error
                : shaky.some((q) => q.state === "down")
                  ? "Прокси не отвечает — мост ждёт его, IP не меняется"
                  : `Нестабилен: ${shakyFails} из ${shakyTotal} подключений за минуту не прошли`}
            </p>
          )}
        </motion.section>
      )}

      {/* ── Трафик: доля через прокси / напрямую + человеческий статус сбоев ── */}
      {running && health && (
        <motion.section variants={fadeInUp} className="mt-5">
          {(() => {
            const total = health.via_upstream + health.via_direct;
            const share = total > 0 ? (health.via_upstream / total) * 100 : 0;
            const groups = errorGroups(health.last_errors ?? [], health.uptime_sec);
            const lastAgo = groups[0]?.ago;
            return (
              <>
                <div className="flex items-center gap-3">
                  <div className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-vb-surface-2">
                    <motion.div
                      className="absolute inset-y-0 left-0 rounded-full bg-vb-emerald"
                      initial={false}
                      animate={{ width: `${share}%` }}
                      transition={{ type: "spring", bounce: 0, duration: 0.6 }}
                    />
                  </div>
                  <Sparkline data={history} width={72} height={20} className="shrink-0 opacity-80" />
                </div>
                <div className="tnum mt-2 flex items-center gap-x-3 text-[12px] text-vb-silver-faint">
                  <span>
                    <span className="text-vb-silver">
                      <NumberTicker value={health.via_upstream} />
                    </span>{" "}
                    через прокси
                  </span>
                  <span>
                    <span className="text-vb-silver">
                      <NumberTicker value={health.via_direct} />
                    </span>{" "}
                    напрямую
                  </span>
                  <span className="flex items-center gap-1.5">
                    <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-vb-emerald" />
                    <NumberTicker value={health.active} /> сейчас
                  </span>
                  <button
                    type="button"
                    onClick={() => groups.length > 0 && setErrorsOpen((v) => !v)}
                    className={cn(
                      "ml-auto flex items-center gap-0.5 transition-colors",
                      groups.length === 0
                        ? "cursor-default"
                        : groups[0].fresh
                          ? "text-vb-warn/90 hover:text-vb-warn"
                          : "hover:text-vb-silver",
                    )}
                  >
                    {groups.length > 0 ? `перебои · ${lastAgo}` : "без сбоев"}
                    {groups.length > 0 && (
                      <ChevronDown className={cn("h-3 w-3 transition-transform", errorsOpen && "rotate-180")} />
                    )}
                  </button>
                </div>
                <AnimatePresence>
                  {errorsOpen && groups.length > 0 && (
                    <motion.ul
                      initial={{ opacity: 0, height: 0 }}
                      animate={{ opacity: 1, height: "auto" }}
                      exit={{ opacity: 0, height: 0 }}
                      transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
                      className="mt-2 overflow-hidden"
                    >
                      {groups.slice(0, 3).map((g) => (
                        <li key={g.text} className="flex items-center gap-2 py-1 text-[12px]">
                          <span className="h-1 w-1 shrink-0 rounded-full bg-vb-warn/70" />
                          <span className="min-w-0 flex-1 truncate text-vb-silver">{g.text}</span>
                          {g.count > 1 && <span className="tnum text-vb-silver-faint">×{g.count}</span>}
                          <span className="tnum shrink-0 text-vb-silver-faint">{g.ago}</span>
                        </li>
                      ))}
                    </motion.ul>
                  )}
                </AnimatePresence>
              </>
            );
          })()}
        </motion.section>
      )}

      {/* ── Переключатели: смысл — прямо в подписи, без «?» ── */}
      <motion.section variants={fadeInUp} className="surface-card mt-5 divide-y divide-vb-border/70">
        <div className="flex items-center justify-between gap-4 px-4 py-3.5">
          <div className="min-w-0">
            <div className="text-[14px] font-medium text-vb-fg">Все программы через мост</div>
            <div className="text-[12px] text-vb-silver-faint">
              Терминал, VS Code, Claude Code — без настройки
            </div>
          </div>
          {env === null ? (
            <Loader2 className="h-4 w-4 shrink-0 animate-spin text-vb-silver-dim" />
          ) : (
            <Toggle checked={envOn} onChange={handleEnvToggle} disabled={envBusy} label="Все программы через мост" />
          )}
        </div>
        {envForeign && (
          <p className="px-4 py-2.5 text-[12px] text-vb-warn">
            Сейчас в системе задан чужой прокси — включение заменит его на мост.
          </p>
        )}
        <div className="flex items-center justify-between gap-4 px-4 py-3.5">
          <div className="min-w-0">
            <div className="text-[14px] font-medium text-vb-fg">Умная маршрутизация</div>
            <div className="text-[12px] text-vb-silver-faint">
              Через прокси только AI, остальное напрямую
            </div>
          </div>
          {health ? (
            <Toggle checked={smartOn} onChange={handleRouteToggle} disabled={routeBusy} label="Умная маршрутизация" />
          ) : running && oldBridge ? (
            <Button variant="secondary" size="sm" onClick={handleBridgeUpdate} disabled={updatingBridge}>
              {updatingBridge && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              Обновить мост
            </Button>
          ) : running ? (
            <Loader2 className="h-4 w-4 shrink-0 animate-spin text-vb-silver-dim" />
          ) : null}
        </div>
      </motion.section>

      {/* ── Требует внимания (только когда есть что сделать) ── */}
      {(bridgeOutdated || (task !== null && !task.registered)) && (
        <motion.section variants={fadeInUp} className="mt-3 space-y-2">
          {bridgeOutdated && (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-vb-border bg-vb-surface-2/60 px-4 py-2.5">
              <p className="text-[12px] text-vb-silver">
                Новый мост {versions?.bundled} — стабильнее при сбоях прокси
              </p>
              <Button variant="secondary" size="sm" onClick={handleBridgeUpdate} disabled={updatingBridge}>
                {updatingBridge && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Обновить
              </Button>
            </div>
          )}
          {task !== null && !task.registered && (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-vb-warn/30 bg-vb-warn/[0.05] px-4 py-2.5">
              <p className="text-[12px] text-vb-warn">Мост не запустится сам после перезагрузки</p>
              <Button variant="secondary" size="sm" onClick={handleTaskRegister} disabled={taskBusy}>
                {taskBusy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Исправить
              </Button>
            </div>
          )}
          {updateError && <p className="text-[12px] text-vb-loss">{updateError}</p>}
        </motion.section>
      )}

      {health && (
        <p className="mt-4 text-center font-mono text-[10px] text-vb-silver-faint/70">
          мост {health.version}
        </p>
      )}
    </motion.div>
  );
}
