import { useEffect, useId, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { ChevronDown, Loader2 } from "lucide-react";
import { Button } from "./Button";
import { PrivacySwitches } from "./PrivacySwitches";
import { privacyGet, privacySet } from "../lib/api";
import { cn } from "../lib/cn";
import type { PrivacyKey, PrivacyState } from "../lib/types";

/** Раскрыть блок при следующем показе настроек: переход по ссылке «в
    настройках» или с плашки про отчёты - там человек пришёл именно сюда. */
let openNext = false;
export function openPrivacyNext() {
  openNext = true;
}

/*
  Настройки: «Обновления и отчёты» - тихий свёрнутый блок внизу страницы.
  Три переключателя пишутся сразу; при ошибке записи возвращаются обратно.
  Что именно отправляется - на сайте и в README, не в программе.
*/
export function PrivacyCard() {
  const [open, setOpen] = useState(() => {
    const v = openNext;
    openNext = false;
    return v;
  });
  const reduce = useReducedMotion();
  const bodyId = useId();
  const [state, setState] = useState<PrivacyState | null>(null);

  // Не прочиталось - не рисуем переключатели с выдуманными значениями.
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = () => {
    setLoadError(null);
    setState(null);
    void privacyGet()
      .then(setState)
      .catch((e) => setLoadError(typeof e === "string" && e ? e : "Не получилось прочитать настройки"));
  };
  useEffect(load, []);

  const change = (key: PrivacyKey, on: boolean) => {
    setState((s) => (s ? { ...s, [key]: on } : s));
    void privacySet(key, on)
      .then(setState)
      .catch(() => setState((s) => (s ? { ...s, [key]: !on } : s)));
  };

  return (
    <section className="mt-4" aria-labelledby="privacy-card-title">
      <h2 id="privacy-card-title" className="text-[13px] font-normal">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={bodyId}
          onClick={() => setOpen((o) => !o)}
          className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-vb-silver-dim transition-colors hover:text-vb-silver"
        >
          Обновления и отчёты
          <ChevronDown
            className={cn("h-3.5 w-3.5 transition-transform duration-200", reduce && "transition-none", open && "rotate-180")}
            aria-hidden
          />
        </button>
      </h2>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            id={bodyId}
            key="privacy"
            initial={reduce ? { opacity: 1 } : { height: 0, opacity: 0 }}
            animate={reduce ? { opacity: 1 } : { height: "auto", opacity: 1 }}
            exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { height: 0, opacity: 0 }}
            transition={reduce ? { duration: 0 } : { duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <div className="surface-card mt-2 px-4 py-3">
              {loadError !== null ? (
                <div role="alert" className="flex items-center gap-3">
                  <p className="min-w-0 flex-1 text-[12px] leading-snug text-vb-loss">{loadError}</p>
                  <Button variant="secondary" size="sm" onClick={load}>
                    Повторить
                  </Button>
                </div>
              ) : state === null ? (
                <Loader2 className="h-4 w-4 animate-spin text-vb-silver-dim" role="status" aria-label="Загрузка настроек" />
              ) : (
                <PrivacySwitches idPrefix="settings" toggleSize="sm" values={state} onChange={change} />
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </section>
  );
}
