import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Check, Loader2 } from "lucide-react";
import { Button } from "./Button";
import { telemetryReport } from "../lib/api";
import { cn } from "../lib/cn";

interface Props {
  open: boolean;
  onClose: () => void;
  /** Куда вернуть фокус после закрытия (кнопка, открывшая окно). */
  returnFocusTo?: React.RefObject<HTMLElement | null>;
}

type Phase = "edit" | "sending" | "done" | "error";

const FIELD =
  "w-full rounded-lg border border-vb-border bg-vb-bg px-3 py-2 text-[13px] text-vb-fg placeholder:text-vb-silver-faint " +
  "focus-visible:border-vb-emerald/60 focus-visible:outline-none disabled:opacity-60";

/*
  «Сообщить о проблеме»: человек пишет, что случилось, к тексту уходит
  состояние программы (адреса и пароли прокси вырезает бэкенд). Ошибка не
  стирает введённое - можно нажать «Повторить».
*/
export function ReportDialog({ open, onClose, returnFocusTo }: Props) {
  const reduce = useReducedMotion();
  const uid = useId();
  const [text, setText] = useState("");
  const [contact, setContact] = useState("");
  const [phase, setPhase] = useState<Phase>("edit");
  const [error, setError] = useState("");
  const sending = useRef(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  const textRef = useRef<HTMLTextAreaElement>(null);
  const doneRef = useRef<HTMLButtonElement>(null);
  const wasOpen = useRef(false);

  useEffect(() => {
    if (open) {
      wasOpen.current = true;
      setText("");
      setContact("");
      setPhase("edit");
      setError("");
      sending.current = false;
      const t = window.setTimeout(() => textRef.current?.focus(), 60);
      return () => window.clearTimeout(t);
    }
    if (wasOpen.current) {
      wasOpen.current = false;
      returnFocusTo?.current?.focus();
    }
  }, [open, returnFocusTo]);

  useEffect(() => {
    if (phase === "done") doneRef.current?.focus();
    if (phase === "error") dialogRef.current?.querySelector<HTMLElement>("[data-retry]")?.focus();
  }, [phase]);

  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !sending.current) {
        e.preventDefault();
        onClose();
      }
      if (e.key === "Tab" && dialogRef.current) {
        const f = Array.from(
          dialogRef.current.querySelectorAll<HTMLElement>("textarea:not([disabled]), input:not([disabled]), button:not([disabled])"),
        );
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
    return () => window.removeEventListener("keydown", key);
  }, [open, onClose]);

  const send = async () => {
    if (sending.current || !text.trim()) return;
    sending.current = true;
    setPhase("sending");
    try {
      await telemetryReport(text.trim(), contact.trim() || null);
      setPhase("done");
    } catch (e) {
      setError(typeof e === "string" && e ? e : "Не получилось отправить. Проверьте интернет и попробуйте ещё раз");
      setPhase("error");
    } finally {
      sending.current = false;
    }
  };

  const locked = phase === "sending";
  const ease = [0.22, 1, 0.36, 1] as const;

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          key="report-dialog"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1, transition: { duration: 0.2, ease: "easeOut" } }}
          exit={{ opacity: 0, transition: { duration: 0.15, ease: "easeIn" } }}
          className="fixed inset-x-0 bottom-0 top-9 z-[86] flex items-center justify-center bg-black/55 p-5 backdrop-blur-[6px]"
        >
          <motion.div
            ref={dialogRef}
            initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.96 }}
            animate={{ opacity: 1, scale: 1, transition: { duration: 0.2, ease } }}
            exit={reduce ? { opacity: 0, transition: { duration: 0.15 } } : { opacity: 0, scale: 0.97, transition: { duration: 0.15, ease: "easeIn" } }}
            role="dialog"
            aria-modal="true"
            aria-labelledby={`${uid}-title`}
            className="flex max-h-full w-full max-w-[420px] flex-col overflow-y-auto rounded-2xl border border-vb-border bg-vb-surface p-5 shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
          >
            <h2 id={`${uid}-title`} className="text-[17px] font-semibold tracking-[-0.01em] text-vb-fg">
              Сообщить о проблеме
            </h2>

            {phase === "done" ? (
              <>
                <p className="mt-4 flex items-center gap-2 text-[14px] text-vb-fg">
                  <Check className="h-4 w-4 text-vb-emerald" strokeWidth={3} aria-hidden />
                  Отправлено. Спасибо
                </p>
                <div className="mt-5 flex justify-end">
                  <Button ref={doneRef} onClick={onClose}>
                    Закрыть
                  </Button>
                </div>
              </>
            ) : (
              <>
                <label htmlFor={`${uid}-text`} className="mt-4 block text-[12px] text-vb-silver-dim">
                  Что случилось
                </label>
                <textarea
                  id={`${uid}-text`}
                  ref={textRef}
                  rows={5}
                  value={text}
                  disabled={locked}
                  onChange={(e) => setText(e.target.value)}
                  className={cn(FIELD, "mt-1 resize-none")}
                />
                <label htmlFor={`${uid}-contact`} className="mt-3 block text-[12px] text-vb-silver-dim">
                  Как с вами связаться <span className="text-vb-silver-faint">(необязательно)</span>
                </label>
                <input
                  id={`${uid}-contact`}
                  value={contact}
                  disabled={locked}
                  onChange={(e) => setContact(e.target.value)}
                  placeholder="Telegram или почта, если нужен ответ"
                  className={cn(FIELD, "mt-1")}
                />
                <p className="mt-3 text-[12px] leading-snug text-vb-silver-faint">
                  Вместе с текстом уйдёт состояние программы. Адреса и пароли прокси вырезаются
                </p>
                {phase === "error" && (
                  <p role="alert" className="mt-3 text-[12px] leading-snug text-vb-loss">
                    {error}
                  </p>
                )}
                <div className="mt-5 flex justify-end gap-2">
                  <Button variant="ghost" onClick={onClose} disabled={locked}>
                    Отмена
                  </Button>
                  <Button data-retry onClick={() => void send()} disabled={locked || !text.trim()}>
                    {locked && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
                    {locked ? "Отправляю…" : phase === "error" ? "Повторить" : "Отправить"}
                  </Button>
                </div>
              </>
            )}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
