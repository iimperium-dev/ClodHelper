import { useEffect, useRef, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Loader2 } from "lucide-react";
import { cn } from "../lib/cn";

interface Props {
  open: boolean;
  /** Вопрос: «Удалить прокси?», «Убрать Telegram из списка?». */
  text: ReactNode;
  /** Что ещё случится - второй строкой. */
  note?: ReactNode;
  confirmLabel: string;
  busy?: boolean;
  /** Кнопка подтверждения недоступна (идёт другое действие). */
  disabled?: boolean;
  /** Действие не вышло - строка под вопросом, полоса остаётся. */
  error?: string | null;
  onConfirm: () => void;
  onCancel: () => void;
  className?: string;
}

/*
  Подтверждение опасного действия - полоса, которая выезжает под строкой.
  Висит, пока не ответили: клик мимо полосы или Esc - отмена (Esc не
  закрывает окно, в котором полоса). Фокус - на «Отмена»: Enter по
  привычке ничего не удалит.
*/
export function ConfirmStrip({ open, ...rest }: Props) {
  // Без движения по просьбе системы - только проявление.
  const still = !!useReducedMotion();
  return (
    <AnimatePresence initial={false}>
      {open && (
        <motion.div
          initial={{ height: still ? "auto" : 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: still ? "auto" : 0, opacity: 0, transition: { duration: 0.16, ease: [0.4, 0, 0.2, 1] } }}
          transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
          className="overflow-hidden"
        >
          <Strip {...rest} />
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function Strip({ text, note, confirmLabel, busy, disabled, error, onConfirm, onCancel, className }: Omit<Props, "open">) {
  const ref = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const latest = useRef({ busy, onCancel });
  latest.current = { busy, onCancel };

  useEffect(() => {
    // Кадр спустя: меню, из которого пришли, успевает закрыться и вернуть фокус.
    const raf = requestAnimationFrame(() => {
      cancelRef.current?.focus({ preventScroll: true });
      ref.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    });
    const outside = (e: PointerEvent) => {
      if (latest.current.busy) return;
      if (ref.current && !ref.current.contains(e.target as Node)) latest.current.onCancel();
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // Перехват до остальных: Esc снимает только вопрос, не окно под ним.
      e.stopPropagation();
      e.preventDefault();
      if (!latest.current.busy) latest.current.onCancel();
    };
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("keydown", esc, true);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("keydown", esc, true);
    };
  }, []);

  return (
    <div
      ref={ref}
      role="group"
      aria-label="Подтверждение"
      data-keep-editing
      className={cn("scroll-my-4 rounded-xl border border-vb-loss/35 bg-vb-loss/[0.06] px-3 py-2", className)}
    >
      {/* Узко - кнопки уходят строкой ниже, вправо; текст не налезает на них. */}
      <div className="flex flex-wrap items-center justify-end gap-x-2 gap-y-1.5">
        <div className="min-w-[10rem] flex-1 break-words text-[12px] leading-snug">
          <div className="text-vb-loss">{text}</div>
          {note && <div className="mt-0.5 text-vb-silver-dim">{note}</div>}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            ref={cancelRef}
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="rounded-full px-2.5 py-1 text-[12px] font-medium text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-silver disabled:opacity-40"
          >
            Отмена
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={busy || disabled}
            className={cn(
              "relative inline-flex items-center justify-center rounded-full bg-vb-loss px-3 py-1 text-[12px] font-semibold text-white transition-colors hover:bg-vb-loss/85 active:scale-[0.96] disabled:opacity-40",
              busy && "disabled:opacity-100",
            )}
          >
            {/* Ширина кнопки не прыгает: подпись держит место под спиннер. */}
            <span className={cn(busy && "invisible")}>{confirmLabel}</span>
            {busy && <Loader2 className="absolute h-3.5 w-3.5 animate-spin" aria-hidden />}
          </button>
        </div>
      </div>
      {error && (
        <p role="alert" className="mt-1.5 text-[12px] leading-snug text-vb-loss">
          {error}
        </p>
      )}
    </div>
  );
}
