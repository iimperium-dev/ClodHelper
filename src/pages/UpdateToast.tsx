import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { X } from "lucide-react";
import { LogoTile } from "../components/UpdateDialog";
import { updateLatest, updateToastClose, updateToastOpen } from "../lib/api";
import { isMac } from "../lib/platform";

/** Сколько уведомление висит само; наведение мыши ставит отсчёт на паузу. */
const LIFETIME_MS = 12_000;

/*
  Уведомление «Доступно обновление» в углу экрана — как у Telegram: карточка
  с логотипом, въезжает сбоку, тает через 12 с (полоска внизу показывает
  остаток). Клик — главное окно с окном обновления. Окно прозрачное и не
  забирает фокус (update_notify.rs), звук играет Rust.
*/
export function UpdateToast() {
  const [version, setVersion] = useState<string | null>(null);
  const [shown, setShown] = useState(true);
  const [hover, setHover] = useState(false);
  const left = useRef(LIFETIME_MS);
  const action = useRef<"open" | "close">("close");
  const [, tick] = useState(0);

  useEffect(() => {
    void updateLatest().then((i) => setVersion(i?.version ?? null)).catch(() => {});
  }, []);

  // Отсчёт до автоскрытия (стоит, пока курсор над карточкой).
  useEffect(() => {
    if (hover || !shown) return;
    let last = performance.now();
    const id = window.setInterval(() => {
      const now = performance.now();
      left.current -= now - last;
      last = now;
      tick((n) => n + 1);
      if (left.current <= 0) dismiss("close");
    }, 100);
    return () => window.clearInterval(id);
  }, [hover, shown]);

  const dismiss = (how: "open" | "close") => {
    action.current = how;
    setShown(false);
  };

  const remaining = Math.max(0, left.current / LIFETIME_MS);

  return (
    // Windows — угол над панелью задач (снизу), macOS — под строкой меню.
    <div className={`flex h-screen w-screen justify-end p-4 ${isMac ? "items-start" : "items-end"}`}>
      <AnimatePresence
        onExitComplete={() => void (action.current === "open" ? updateToastOpen() : updateToastClose())}
      >
        {shown && (
          <motion.div
            key="toast"
            role="button"
            tabIndex={0}
            onClick={() => dismiss("open")}
            onKeyDown={(e) => e.key === "Enter" && dismiss("open")}
            onMouseEnter={() => setHover(true)}
            onMouseLeave={() => setHover(false)}
            initial={{ opacity: 0, x: 60, scale: 0.96 }}
            animate={{ opacity: 1, x: 0, scale: 1 }}
            exit={{ opacity: 0, x: action.current === "open" ? 0 : 40, scale: action.current === "open" ? 0.97 : 1 }}
            transition={{ type: "spring", bounce: 0.22, duration: 0.5 }}
            className="group relative flex h-[88px] w-[340px] cursor-pointer select-none items-center gap-3 overflow-hidden rounded-2xl border border-vb-border bg-vb-surface/95 px-4 shadow-[0_12px_32px_rgba(0,0,0,0.55)] backdrop-blur-md transition-colors hover:border-vb-border-strong"
          >
            <div className="relative shrink-0">
              <motion.span
                aria-hidden
                className="absolute inset-0 rounded-[12px] border border-vb-emerald/50"
                initial={{ opacity: 0.7, scale: 1 }}
                animate={{ opacity: 0, scale: 1.5 }}
                transition={{ duration: 1.8, repeat: 2, ease: "easeOut", delay: 0.3 }}
              />
              <LogoTile size={44} />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5 text-[11px] text-vb-silver-faint">
                <span className="font-medium text-vb-silver-dim">Clod Helper</span>
                <span>·</span>
                <span>сейчас</span>
              </div>
              <div className="truncate text-[14px] font-semibold text-vb-fg">
                {version ? `Доступно обновление ${version}` : "Доступно обновление"}
              </div>
              <div className="truncate text-[12px] text-vb-silver-dim">Нажмите, чтобы посмотреть, что нового</div>
            </div>
            <button
              type="button"
              aria-label="Закрыть"
              onClick={(e) => {
                e.stopPropagation();
                dismiss("close");
              }}
              className="absolute right-2 top-2 rounded-full p-1 text-vb-silver-faint opacity-0 transition-opacity hover:bg-vb-surface-2 hover:text-vb-silver group-hover:opacity-100"
            >
              <X className="h-3.5 w-3.5" />
            </button>
            {/* Остаток времени до автоскрытия */}
            <span
              aria-hidden
              className="absolute bottom-0 left-0 h-[2px] bg-vb-emerald/70"
              style={{ width: `${remaining * 100}%`, transition: "width 100ms linear" }}
            />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
