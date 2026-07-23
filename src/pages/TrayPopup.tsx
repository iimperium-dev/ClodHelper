import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  Activity,
  ExternalLink,
  Loader2,
  LogOut,
  Network,
  Pause,
  Play,
  RotateCw,
  Route,
} from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Toggle } from "../components/Toggle";
import { StatusOrb } from "../components/StatusOrb";
import { NumberTicker } from "../components/NumberTicker";
import {
  bridgeSetRouteMode,
  clearGlobalProxyEnv,
  quitApp,
  setGlobalProxyEnv,
  shimRestart,
  shimStart,
  shimStop,
  shimTest,
  showMainWindow,
} from "../lib/api";
import { formatUptime, useStatus } from "../lib/status";
import type { ShimTestResult } from "../lib/types";
import { cn } from "../lib/cn";

/*
  Трей-попап: компактная живая панель у иконки трея (левый клик) — паттерн
  Tailscale/WARP вместо нативного меню. Окно прозрачное, скрывается по blur
  (Rust), тень рисуем CSS. Всё управление — не открывая большого окна.
*/
export function TrayPopup() {
  const { status, health, refresh, env, setEnv } = useStatus();

  const [busy, setBusy] = useState<"start" | "stop" | "restart" | "test" | null>(null);
  const [testResult, setTestResult] = useState<ShimTestResult | null>(null);
  const [envBusy, setEnvBusy] = useState(false);
  const [routeBusy, setRouteBusy] = useState(false);
  /** Ошибка последнего действия: без неё упавшая команда выглядит как
      «нажал и ничего не произошло». Гаснет сама. */
  const [actionError, setActionError] = useState<string | null>(null);
  const failFlash = (e: unknown) => {
    setActionError(String(e));
    window.setTimeout(() => setActionError(null), 6000);
  };

  const loading = status === null;
  const running = status?.running ?? false;
  const upstreamDown = health !== null && health.upstreams.some((u) => !u.healthy);
  const orb = loading
    ? ("loading" as const)
    : !running
      ? ("down" as const)
      : upstreamDown
        ? ("warn" as const)
        : ("ok" as const);

  const envOn = (env?.present ?? false) && (env?.points_to_bridge ?? false);
  const smartOn = health?.mode === "smart";

  const act = async (
    kind: "start" | "stop" | "restart",
    fn: () => Promise<unknown>,
  ) => {
    setBusy(kind);
    setActionError(null);
    try {
      await fn();
      await refresh();
    } catch (e) {
      failFlash(e);
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
      window.setTimeout(() => setTestResult(null), 6000);
    } catch (e) {
      failFlash(e);
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
      failFlash(e);
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
      failFlash(e);
    } finally {
      setRouteBusy(false);
    }
  };

  const openApp = async () => {
    await showMainWindow();
    await getCurrentWindow().hide();
  };

  return (
    // Панель хугает контент и прижата к низу окна (к трею): лишняя площадь
    // окна прозрачна и невидима, при раскрытии результата теста панель растёт
    // ВВЕРХ. Клик по прозрачной зоне — закрыть (как клик мимо).
    <div
      className="flex h-screen w-screen flex-col justify-end p-2.5"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) void getCurrentWindow().hide();
      }}
    >
      <div className="flex flex-col overflow-hidden rounded-lg border border-vb-border bg-vb-bg shadow-[0_16px_48px_rgba(0,0,0,0.55)]">
        {/* Статус */}
        <div className="flex items-center gap-3 px-4 pb-3 pt-4">
          <StatusOrb state={orb} size={11} />
          <div className="min-w-0 flex-1">
            <div className="text-[14px] font-semibold leading-tight text-vb-fg">
              {loading ? "Проверяю…" : running ? "Мост активен" : "Мост остановлен"}
            </div>
            <div className="tnum text-[11px] text-vb-silver-faint">
              {running
                ? `${formatUptime(health?.uptime_sec ?? status?.uptime_sec)}${
                    health ? ` · ${health.mode === "smart" ? "smart" : "весь трафик"}` : ""
                  }`
                : "трафик не проксируется"}
            </div>
          </div>
          {running ? (
            <div className="flex shrink-0 gap-1">
              <button
                type="button"
                onClick={() => act("restart", shimRestart)}
                disabled={busy !== null}
                className="rounded-lg p-2 text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.95] disabled:opacity-35"
                title="Перезапустить мост"
              >
                <RotateCw className={cn("h-4 w-4", busy === "restart" && "animate-spin")} />
              </button>
              <button
                type="button"
                onClick={() => act("stop", shimStop)}
                disabled={busy !== null}
                className="rounded-lg p-2 text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-loss active:scale-[0.95] disabled:opacity-35"
                title="Остановить мост"
              >
                <Pause className="h-4 w-4" />
              </button>
            </div>
          ) : loading ? null : (
            <button
              type="button"
              onClick={() => act("start", shimStart)}
              disabled={busy !== null}
              className="flex shrink-0 items-center gap-1.5 rounded-lg bg-vb-emerald px-3 py-1.5 text-[12px] font-semibold text-black transition-colors hover:bg-vb-emerald-bright active:scale-[0.97] disabled:opacity-50"
            >
              <Play className="h-3.5 w-3.5" />
              Запустить
            </button>
          )}
        </div>

        {/* Ошибка последнего действия */}
        <AnimatePresence>
          {actionError && (
            <motion.p
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
              className="overflow-hidden px-4 pb-1 text-[11px] leading-snug text-vb-loss"
            >
              {actionError}
            </motion.p>
          )}
        </AnimatePresence>

        {/* Метрики */}
        {health && (
          <div className="mx-4 flex items-center justify-between rounded-lg bg-vb-surface px-3 py-2">
            <div className="text-center">
              <div className="font-mono text-[15px] font-semibold leading-none text-vb-fg">
                <NumberTicker value={health.via_upstream} />
              </div>
              <div className="mt-1 text-[9px] uppercase tracking-[0.05em] text-vb-silver-faint">
                прокси
              </div>
            </div>
            <div className="text-center">
              <div className="font-mono text-[15px] font-semibold leading-none text-vb-fg">
                <NumberTicker value={health.via_direct} />
              </div>
              <div className="mt-1 text-[9px] uppercase tracking-[0.05em] text-vb-silver-faint">
                напрямую
              </div>
            </div>
            <div className="text-center">
              <div className="font-mono text-[15px] font-semibold leading-none text-vb-emerald">
                <NumberTicker value={health.active} />
              </div>
              <div className="mt-1 text-[9px] uppercase tracking-[0.05em] text-vb-silver-faint">
                активных
              </div>
            </div>
            <div className="text-center">
              <div
                className={cn(
                  "font-mono text-[15px] font-semibold leading-none",
                  health.errors > 0 ? "text-vb-warn" : "text-vb-silver-faint",
                )}
              >
                <NumberTicker value={health.errors} />
              </div>
              <div className="mt-1 text-[9px] uppercase tracking-[0.05em] text-vb-silver-faint">
                ошибок
              </div>
            </div>
          </div>
        )}

        {/* Тумблеры */}
        <div className="mt-3 flex flex-col">
          <button
            type="button"
            onClick={handleEnvToggle}
            disabled={envBusy || env === null}
            className="flex items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-vb-surface-2/50 disabled:opacity-60"
          >
            <Network className="h-4 w-4 shrink-0 text-vb-silver-dim" strokeWidth={1.9} />
            <span className="min-w-0 flex-1 text-[13px] font-medium text-vb-silver">
              Системное проксирование
            </span>
            {/* Toggle визуальный: клик обрабатывает вся строка (pointer-events-none
                убирает двойное срабатывание клика по самому тумблеру). */}
            <span className="pointer-events-none">
              <Toggle
                checked={envOn}
                onChange={handleEnvToggle}
                disabled={envBusy || env === null}
                label="Системный прокси"
              />
            </span>
          </button>
          <button
            type="button"
            onClick={health ? handleRouteToggle : undefined}
            disabled={routeBusy || !health}
            className="flex items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-vb-surface-2/50 disabled:opacity-60"
          >
            <Route className="h-4 w-4 shrink-0 text-vb-silver-dim" strokeWidth={1.9} />
            <span className="min-w-0 flex-1 text-[13px] font-medium text-vb-silver">
              Умная маршрутизация
            </span>
            {health ? (
              <span className="pointer-events-none">
                <Toggle
                  checked={smartOn}
                  onChange={handleRouteToggle}
                  disabled={routeBusy}
                  label="Умная маршрутизация"
                />
              </span>
            ) : (
              <span className="text-[11px] text-vb-silver-faint">н/д</span>
            )}
          </button>
        </div>

        {/* Тест */}
        <div className="px-4 pt-1.5">
          <button
            type="button"
            onClick={handleTest}
            disabled={busy !== null || !running}
            className="flex w-full items-center justify-center gap-2 rounded-lg border border-vb-border bg-vb-surface px-3 py-2 text-[13px] font-medium text-vb-silver transition-colors hover:border-vb-border-strong hover:bg-vb-surface-2 active:scale-[0.98] disabled:opacity-40"
          >
            {busy === "test" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Activity className="h-3.5 w-3.5" />
            )}
            Тест соединения
          </button>
          <AnimatePresence>
            {testResult && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
                className="overflow-hidden"
              >
                <div
                  className={cn(
                    "tnum mt-1.5 flex items-center gap-2 rounded-lg px-3 py-1.5 text-[12px]",
                    testResult.ok
                      ? "bg-vb-emerald/[0.08] text-vb-emerald"
                      : "bg-vb-loss/[0.08] text-vb-loss",
                  )}
                >
                  <span
                    className={cn(
                      "h-1.5 w-1.5 shrink-0 rounded-full",
                      testResult.ok ? "bg-vb-emerald" : "bg-vb-loss",
                    )}
                  />
                  <span className="truncate font-mono">
                    {testResult.ok
                      ? `${testResult.external_ip ?? "OK"} · ${testResult.latency_ms ?? "—"} мс`
                      : testResult.error ?? "ошибка"}
                  </span>
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        {/* Футер */}
        <div className="mt-3 flex items-center justify-between border-t border-vb-border/60 px-2 py-1.5">
          <button
            type="button"
            onClick={openApp}
            className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px] font-medium text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.97]"
          >
            <ExternalLink className="h-3.5 w-3.5" />
            Открыть приложение
          </button>
          <button
            type="button"
            onClick={() => void quitApp()}
            className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px] font-medium text-vb-silver-faint transition-colors hover:bg-vb-loss/10 hover:text-vb-loss active:scale-[0.97]"
            title="Полностью выйти (мост продолжит работать)"
          >
            <LogOut className="h-3.5 w-3.5" />
            Выход
          </button>
        </div>
      </div>
    </div>
  );
}
