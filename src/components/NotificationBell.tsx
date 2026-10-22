import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Bell, CheckCheck, Download, ExternalLink } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { noticesList, noticesMarkRead, noticesRefresh, updateLatest } from "../lib/api";
import { openUpdateDialog } from "../lib/updateBus";
import { cn } from "../lib/cn";
import type { NoticeItem } from "../lib/types";

/** «2026-10-22» → «22.10.2026». */
function formatDate(d: string): string {
  const [y, m, day] = d.split("-");
  return y && m && day ? `${day}.${m}.${y}` : d;
}

/*
  Колокольчик в заголовке окна: обновления и рассылки разработчика
  (notices.json в репозитории). Непрочитанное — светящийся кружок с числом.
  Панель: «Прочитать все», клик по уведомлению — прочитано; у обновления —
  «Обновить», у рассылки со ссылкой — «Подробнее».
*/
export function NotificationBell() {
  const [items, setItems] = useState<NoticeItem[]>([]);
  const [open, setOpen] = useState(false);
  const [focusId, setFocusId] = useState<string | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const bellRef = useRef<HTMLButtonElement | null>(null);

  const reload = useCallback(() => void noticesList().then(setItems).catch(() => {}), []);

  // Окно открыли — проверить рассылки сразу, не дожидаясь фоновой проверки.
  useEffect(() => {
    const onFocus = () => void noticesRefresh().catch(() => {});
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  useEffect(() => {
    reload();
    const subs = [
      listen("notices:changed", reload),
      listen("update:available", reload),
      // Клик по уведомлению в углу экрана — открыть колокольчик на нём.
      listen<string>("notices:open", (e) => {
        setFocusId(e.payload);
        setOpen(true);
        reload();
      }),
    ];
    return () => subs.forEach((s) => void s.then((f) => f()));
  }, [reload]);

  // Закрыть по клику мимо и по Esc.
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!panelRef.current?.contains(t) && !bellRef.current?.contains(t)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", down);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);

  const unread = items.filter((i) => !i.read).length;

  const markRead = async (ids: string[]) => {
    setItems((prev) => prev.map((i) => (ids.length === 0 || ids.includes(i.id) ? { ...i, read: true } : i)));
    setItems(await noticesMarkRead(ids).catch(() => items));
  };

  const openUpdate = async (item: NoticeItem) => {
    void markRead([item.id]);
    const info = await updateLatest().catch(() => null);
    if (info) {
      setOpen(false);
      openUpdateDialog(info);
    }
  };

  return (
    <>
      <button
        ref={bellRef}
        type="button"
        aria-label={unread ? `Уведомления: ${unread} новых` : "Уведомления"}
        aria-expanded={open}
        onClick={() => {
          setFocusId(null);
          setOpen((v) => !v);
          reload();
        }}
        className={cn(
          "relative flex w-11 items-center justify-center transition-colors duration-150 hover:bg-white/[0.06]",
          open ? "text-vb-fg" : "text-vb-silver-dim hover:text-vb-fg",
        )}
      >
        <Bell className="h-3.5 w-3.5" strokeWidth={2.25} />
        <AnimatePresence>
          {unread > 0 && (
            <motion.span
              initial={{ scale: 0 }}
              animate={{ scale: 1 }}
              exit={{ scale: 0 }}
              transition={{ type: "spring", bounce: 0.45, duration: 0.35 }}
              className="absolute right-1.5 top-1.5"
            >
              {/* Светящийся кружок: мягкая пульсация, чтобы новое заметили */}
              <span className="absolute inset-0 animate-ping rounded-full bg-vb-emerald/60 [animation-duration:2s]" />
              <span className="tnum relative grid h-[15px] min-w-[15px] place-items-center rounded-full bg-vb-emerald px-1 text-[9px] font-bold leading-none text-black shadow-[0_0_8px_var(--color-vb-emerald-glow)]">
                {unread > 9 ? "9+" : unread}
              </span>
            </motion.span>
          )}
        </AnimatePresence>
      </button>

      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={panelRef}
              initial={{ opacity: 0, y: -6, scale: 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -4, scale: 0.98 }}
              transition={{ duration: 0.16, ease: [0.22, 1, 0.36, 1] }}
              role="dialog"
              aria-label="Уведомления"
              className="fixed right-3 top-10 z-[75] flex max-h-[min(460px,calc(100vh-56px))] w-[340px] origin-top-right flex-col overflow-hidden rounded-xl border border-vb-border bg-vb-surface shadow-[0_16px_48px_rgba(0,0,0,0.6)]"
            >
              <div className="flex shrink-0 items-center justify-between border-b border-vb-border/70 px-4 py-2.5">
                <span className="text-[13px] font-semibold text-vb-fg">Уведомления</span>
                {unread > 0 && (
                  <button
                    type="button"
                    onClick={() => void markRead([])}
                    className="flex items-center gap-1 text-[11px] text-vb-silver-dim transition-colors hover:text-vb-emerald"
                  >
                    <CheckCheck className="h-3.5 w-3.5" />
                    Прочитать все
                  </button>
                )}
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
                {items.length === 0 ? (
                  <div className="px-4 py-8 text-center text-[12px] text-vb-silver-faint">Пока ничего нового</div>
                ) : (
                  items.map((i) => (
                    <div
                      key={i.id}
                      role="button"
                      tabIndex={0}
                      onClick={() => !i.read && void markRead([i.id])}
                      onKeyDown={(e) => e.key === "Enter" && !i.read && void markRead([i.id])}
                      className={cn(
                        "flex gap-3 border-b border-vb-border/50 px-4 py-3 transition-colors last:border-b-0",
                        !i.read ? "cursor-pointer bg-vb-emerald/[0.04] hover:bg-vb-emerald/[0.07]" : "hover:bg-vb-surface-2/40",
                        focusId === i.id && "ring-1 ring-inset ring-vb-emerald/40",
                      )}
                    >
                      <span className="mt-1.5 flex h-2 w-2 shrink-0">
                        {!i.read && <span className="h-2 w-2 rounded-full bg-vb-emerald shadow-[0_0_6px_var(--color-vb-emerald-glow)]" />}
                      </span>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-baseline justify-between gap-2">
                          <span className={cn("text-[13px] leading-snug", i.read ? "text-vb-silver" : "font-semibold text-vb-fg")}>
                            {i.title}
                          </span>
                          {i.date && <span className="tnum shrink-0 text-[10px] text-vb-silver-faint">{formatDate(i.date)}</span>}
                        </div>
                        {i.body && (
                          <p className="mt-0.5 whitespace-pre-line text-[12px] leading-snug text-vb-silver-dim">{i.body}</p>
                        )}
                        {(i.kind === "update" || i.link) && (
                          <div className="mt-2">
                            {i.kind === "update" ? (
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void openUpdate(i);
                                }}
                                className="inline-flex items-center gap-1.5 rounded-lg bg-vb-emerald px-2.5 py-1 text-[12px] font-semibold text-black transition-colors hover:bg-vb-emerald-bright active:scale-[0.97]"
                              >
                                <Download className="h-3.5 w-3.5" />
                                Обновить
                              </button>
                            ) : (
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void markRead([i.id]);
                                  void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(i.link!));
                                }}
                                className="inline-flex items-center gap-1.5 rounded-lg border border-vb-border px-2.5 py-1 text-[12px] text-vb-silver transition-colors hover:border-vb-border-strong"
                              >
                                <ExternalLink className="h-3.5 w-3.5" />
                                Подробнее
                              </button>
                            )}
                          </div>
                        )}
                      </div>
                    </div>
                  ))
                )}
              </div>
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}
