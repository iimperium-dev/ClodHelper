import { useEffect, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { cn } from "../lib/cn";

interface Props {
  /** Содержимое поповера (текст-пояснение «для чайника»). */
  children: ReactNode;
  className?: string;
}

/*
  «?»-подсказка вместо инлайн-описаний (описания под каждым тумблером
  засоряли интерфейс — фидбек). Открывается кликом и по hover, закрывается
  кликом мимо/Escape. Поповер масштабируется от триггера (origin-aware).
*/
export function InfoTip({ children, className }: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLSpanElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    // Закрытие по уводу мыши — на всём корне (кнопка + поповер), чтобы
    // подсказки не «зависали» и не копились по экрану.
    <span
      ref={rootRef}
      className={cn("relative inline-flex", className)}
      onMouseLeave={() => setOpen(false)}
    >
      <button
        type="button"
        aria-label="Подробнее"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        onMouseEnter={() => setOpen(true)}
        className={cn(
          "flex h-[17px] w-[17px] items-center justify-center rounded-full border text-[10px] font-semibold leading-none",
          "transition-colors duration-150",
          open
            ? "border-vb-emerald/50 bg-vb-emerald/10 text-vb-emerald"
            : "border-vb-border-strong text-vb-silver-faint hover:border-vb-emerald/40 hover:text-vb-silver",
        )}
      >
        ?
      </button>
      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, scale: 0.95, y: 4 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 2 }}
            transition={{ duration: 0.15, ease: [0.22, 1, 0.36, 1] }}
            style={{ transformOrigin: "top left" }}
            className="z-tooltip absolute left-0 top-full mt-1.5 w-[290px] rounded-lg border border-vb-border bg-vb-surface p-3 text-[12px] font-normal leading-relaxed text-vb-silver shadow-[0_12px_32px_rgba(0,0,0,0.55)]"
          >
            {children}
          </motion.div>
        )}
      </AnimatePresence>
    </span>
  );
}
