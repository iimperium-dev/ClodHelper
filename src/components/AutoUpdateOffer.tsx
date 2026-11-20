import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Check, Loader2 } from "lucide-react";
import { CycleMark } from "./CycleMark";
import { cn } from "../lib/cn";

interface Props {
  open: boolean;
  /** Ответ человека: true — включить, false — «Не сейчас» (Esc тоже). */
  onAnswer: (enabled: boolean) => Promise<void> | void;
  /** «в настройках»: закрыть окно БЕЗ ответа и открыть вкладку «Настройки». */
  onOpenSettings: () => void;
}

/*
  Предложение автообновления — для тех, у кого программа стояла до этой
  функции. Показывается один раз, сразу после «Что нового»: любой ответ
  (и «Не сейчас», и Esc) записывается и больше окно не появляется. Ссылка
  «в настройках» ответа не пишет: человек решит там, а если не тронет -
  предложение покажется при следующем запуске.

  Иерархия намеренно неравная: «Включить» — крупная зелёная кнопка во всю
  ширину, в фокусе с открытия (Enter = да); отказ — маленькая серая ссылка
  под ней. Щелчок мимо окна ничего не решает — окно ждёт ответа.
*/
export function AutoUpdateOffer({ open, onAnswer, onOpenSettings }: Props) {
  const reduce = useReducedMotion();
  const [busy, setBusy] = useState<null | boolean>(null);
  const [accepted, setAccepted] = useState(false);
  const yesRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  const answer = async (enabled: boolean) => {
    if (busy !== null) return;
    setBusy(enabled);
    try {
      if (enabled) {
        setAccepted(true);
        // Галочка успевает показаться, прежде чем окно уйдёт.
        await new Promise((r) => setTimeout(r, reduce ? 0 : 650));
      }
      await onAnswer(enabled);
    } finally {
      setBusy(null);
    }
  };

  useEffect(() => {
    if (!open) {
      setAccepted(false);
      return;
    }
    const t = window.setTimeout(() => yesRef.current?.focus(), 60);
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") void answer(false);
      // Фокус не выходит из окна: Tab ходит между двумя кнопками.
      if (e.key === "Tab" && dialogRef.current) {
        const f = dialogRef.current.querySelectorAll<HTMLElement>("button:not([disabled])");
        if (!f.length) return;
        const first = f[0];
        const last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener("keydown", key);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener("keydown", key);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          key="auto-update-offer"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          className="fixed inset-x-0 bottom-0 top-9 z-[86] flex items-center justify-center bg-black/55 p-5 backdrop-blur-[6px]"
        >
          <motion.div
            ref={dialogRef}
            initial={reduce ? { opacity: 0 } : { opacity: 0, y: 14, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: 6, scale: 0.98 }}
            transition={{ type: "spring", bounce: 0, duration: 0.42 }}
            role="dialog"
            aria-modal="true"
            aria-labelledby="auto-update-title"
            aria-describedby="auto-update-desc"
            className="relative flex max-h-full w-full max-w-[380px] flex-col overflow-y-auto rounded-2xl border border-vb-border bg-vb-surface px-6 pb-5 pt-5 text-center shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
          >
            <CycleMark accepted={accepted} reduce={!!reduce} compact />

            <h2 id="auto-update-title" className="mt-3 text-[20px] font-semibold tracking-[-0.01em] text-vb-fg">
              Автообновление. Включить?
            </h2>
            <p id="auto-update-desc" className="mx-auto mt-2 max-w-[30ch] text-[13px] leading-relaxed text-vb-silver-dim">
              Clod Helper будет сам ставить новые версии, с исправлениями и новыми возможностями.
            </p>

            <ul className="mx-auto mt-4 w-full max-w-[290px] space-y-1.5 text-left">
              {[
                "Без окон и кнопок, когда программа свёрнута",
                "Только проверенные файлы с подписью разработчика",
                "Выключается в Настройках в один щелчок",
              ].map((t) => (
                <li key={t} className="flex gap-2 text-[12px] leading-snug text-vb-silver">
                  <Check className="mt-px h-3.5 w-3.5 shrink-0 text-vb-emerald" aria-hidden />
                  {t}
                </li>
              ))}
            </ul>

            <button
              ref={yesRef}
              type="button"
              onClick={() => void answer(true)}
              disabled={busy !== null}
              className={cn(
                "mt-4 inline-flex h-12 w-full shrink-0 items-center justify-center gap-2 rounded-xl text-[15px] font-semibold",
                "bg-vb-emerald text-black shadow-[0_4px_20px_var(--color-vb-emerald-glow)]",
                "transition-[transform,background-color] duration-150 ease-[cubic-bezier(0.22,1,0.36,1)]",
                "hover:bg-vb-emerald-bright active:scale-[0.98] disabled:cursor-default",
                "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-vb-emerald",
              )}
            >
              {accepted ? (
                <>
                  <Check className="h-4 w-4" strokeWidth={3} aria-hidden />
                  Включено
                </>
              ) : busy === true ? (
                <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
              ) : (
                "Включить автообновление"
              )}
            </button>
            <button
              type="button"
              onClick={() => void answer(false)}
              disabled={busy !== null}
              className="mx-auto mt-2 rounded-md px-2 py-1.5 text-[12px] text-vb-silver-faint transition-colors duration-150 hover:text-vb-silver-dim disabled:opacity-50"
            >
              Не сейчас
            </button>
            <p className="mx-auto mt-3 max-w-[300px] text-[11px] leading-snug text-vb-silver-faint">
              С этой версии программа также отправляет отчёты об ошибках и анонимную статистику. Выключить можно{" "}
              <button
                type="button"
                onClick={onOpenSettings}
                disabled={busy !== null}
                className="rounded-sm text-vb-silver-dim underline underline-offset-2 transition-colors hover:text-vb-silver disabled:opacity-50"
              >
                в настройках
              </button>
            </p>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
