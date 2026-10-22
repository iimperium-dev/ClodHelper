import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  Activity,
  ExternalLink,
  Loader2,
  LogOut,
  Pause,
  Play,
  RotateCw,
} from "lucide-react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { isMac } from "../lib/platform";
import { StatusOrb } from "../components/StatusOrb";
import { NumberTicker } from "../components/NumberTicker";
import { listen } from "@tauri-apps/api/event";
import {
  noticesList,
  noticesRefresh,
  quitApp,
  shimRestart,
  shimStart,
  shimStop,
  shimTest,
  showMainWindow,
  updateToastOpen,
} from "../lib/api";
import { formatUptime, useStatus } from "../lib/status";
import type { NoticeItem, ShimTestResult } from "../lib/types";
import { cn } from "../lib/cn";

/*
  Трей-попап: компактная живая панель у иконки трея (левый клик) — паттерн
  Tailscale/WARP вместо нативного меню. Окно прозрачное, скрывается по blur
  (Rust), тень рисуем CSS. Всё управление — не открывая большого окна.
*/
export function TrayPopup() {
  // orb — единое состояние из StatusProvider (учитывает и «нестабилен»).
  const { status, health, refresh, orb } = useStatus();

  const [busy, setBusy] = useState<"start" | "stop" | "restart" | "test" | null>(null);
  const [testResult, setTestResult] = useState<ShimTestResult | null>(null);
  /** Ошибка последнего действия: без неё упавшая команда выглядит как
      «нажал и ничего не произошло». Гаснет сама. */
  const [actionError, setActionError] = useState<string | null>(null);
  const failFlash = (e: unknown) => {
    setActionError(String(e));
    window.setTimeout(() => setActionError(null), 6000);
  };

  const loading = status === null;
  const running = status?.running ?? false;


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

  const openApp = async () => {
    await showMainWindow();
    await getCurrentWindow().hide();
  };

  // Обновление и непрочитанные уведомления — индикатор прямо в попапе.
  const [notices, setNotices] = useState<NoticeItem[]>([]);
  useEffect(() => {
    const reload = () => void noticesList().then(setNotices).catch(() => {});
    reload();
    const subs = [
      listen("notices:changed", reload),
      listen("update:available", reload),
      getCurrentWindow().onFocusChanged(({ payload }) => {
        if (!payload) return;
        reload();
        void noticesRefresh().catch(() => {});
      }),
    ];
    return () => subs.forEach((s) => void s.then((f) => f()));
  }, []);
  const update = notices.find((n) => n.kind === "update");
  const unreadNews = notices.filter((n) => n.kind === "news" && !n.read).length;
  const openUpdate = async () => {
    if (!update) return;
    await updateToastOpen(update.id);
    await getCurrentWindow().hide();
  };

  // macOS: значок сверху — панель висит от верха окна, а окно ровно по её
  // высоте (растёт вниз при результате теста). Без этого под панелью
  // оставалась невидимая область окна, перехватывающая клики.
  const panelRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = panelRef.current;
    if (!isMac || !el) return;
    // + отступы окна: 3 сверху (вплотную к строке меню) и 10 снизу под тень.
    const fit = () => void getCurrentWindow().setSize(new LogicalSize(332, Math.ceil(el.offsetHeight) + 13)).catch(() => {});
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    // Каждое открытие — панель «выезжает» из значка вниз (CSS .tray-drop).
    const replay = () => {
      el.classList.remove("tray-drop");
      void el.offsetWidth;
      el.classList.add("tray-drop");
    };
    const un = getCurrentWindow().onFocusChanged(({ payload }) => payload && replay());
    return () => {
      ro.disconnect();
      void un.then((f) => f());
    };
  }, []);

  return (
    // Windows: трей внизу — панель прижата к низу окна и растёт ВВЕРХ.
    // Клик по прозрачной зоне — закрыть (как клик мимо).
    <div
      className={cn(
        "flex h-screen w-screen flex-col",
        isMac ? "justify-start px-2.5 pb-2.5 pt-[3px]" : "justify-end p-2.5",
      )}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) void getCurrentWindow().hide();
      }}
    >
      <div
        ref={panelRef}
        // shrink-0: высота панели — по содержимому, а не по окну. Иначе окно
        // подгонялось под панель, панель под окно — и футер обрезался.
        className="flex shrink-0 flex-col overflow-hidden rounded-xl border border-vb-border bg-vb-bg shadow-[0_8px_28px_rgba(0,0,0,0.5)]"
      >
        {/* Статус */}
        <div className="flex items-center gap-3 px-4 pb-3 pt-4">
          <StatusOrb state={orb} size={11} />
          <div className="min-w-0 flex-1">
            <div className="text-[14px] font-semibold leading-tight text-vb-fg">
              {loading ? "Проверяю…" : running ? "Мост активен" : "Мост остановлен"}
            </div>
            <div className="tnum text-[11px] text-vb-silver-faint">
              {running
                ? `работает ${formatUptime(health?.uptime_sec ?? status?.uptime_sec)}`
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

        {/* Вышло обновление — светящаяся строка */}
        {update && (
          <button
            type="button"
            onClick={() => void openUpdate()}
            className="mx-4 mb-3 flex items-center gap-2.5 rounded-lg border border-vb-emerald/30 bg-vb-emerald/[0.07] px-3 py-2 text-left transition-colors hover:bg-vb-emerald/[0.12] active:scale-[0.98]"
          >
            <span className="relative flex h-2 w-2 shrink-0">
              <span className="absolute inset-0 animate-ping rounded-full bg-vb-emerald/70 [animation-duration:2s]" />
              <span className="relative h-2 w-2 rounded-full bg-vb-emerald shadow-[0_0_8px_var(--color-vb-emerald-glow)]" />
            </span>
            <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-vb-fg">{update.title}</span>
            <span className="shrink-0 text-[11px] font-semibold text-vb-emerald">Обновить</span>
          </button>
        )}

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

        {/* Тест */}
        <div className="px-4 pt-3">
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
            {unreadNews > 0 && (
              <span
                className="tnum grid h-4 min-w-4 place-items-center rounded-full bg-vb-emerald px-1 text-[9px] font-bold text-black"
                title={`Новых уведомлений: ${unreadNews}`}
              >
                {unreadNews}
              </span>
            )}
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
