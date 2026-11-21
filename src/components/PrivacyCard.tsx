import { useEffect, useId, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { ChevronDown, Loader2, MessageSquareWarning } from "lucide-react";
import { Button } from "./Button";
import { PrivacySwitches } from "./PrivacySwitches";
import { ReportDialog } from "./ReportDialog";
import { privacyGet, privacySet } from "../lib/api";
import { cn } from "../lib/cn";
import type { PrivacyKey, PrivacyState } from "../lib/types";

/** Полный список того, что уходит на сервер, без прикрас. */
const SENT_ITEMS: { title: string; text: string }[] = [
  {
    title: "Раз в день (если включена статистика)",
    text:
      "версия программы, система, процессор, включено ли автообновление, первый ли это запуск за день, неделю и месяц, " +
      "неделя установки (только в первый раз и раз в неделю), способ обновления, работал ли прокси",
  },
  {
    title: "При сбое (если включены отчёты)",
    text: "какой это сбой (программа, окно, мост, обновление), текст ошибки и стек",
  },
  { title: "События объявлений (если включена статистика)", text: "показано, нажато, закрыто" },
  { title: "Объявления", text: "раз в несколько часов программа спрашивает сервер объявлений" },
];

/*
  Тихая раскрывашка «Что именно отправляется»: высота плавно, ~200 мс; при
  reduced-motion - без движения.
*/
function WhatIsSent() {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion();
  const id = useId();
  return (
    <div className="mt-3">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((o) => !o)}
        className="inline-flex items-center gap-1 rounded text-[12px] text-vb-silver-dim transition-colors hover:text-vb-silver"
      >
        Что именно отправляется
        <ChevronDown
          className={cn("h-3.5 w-3.5 transition-transform duration-200", reduce && "transition-none", open && "rotate-180")}
          aria-hidden
        />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            id={id}
            key="sent"
            initial={reduce ? { opacity: 1 } : { height: 0, opacity: 0 }}
            animate={reduce ? { opacity: 1 } : { height: "auto", opacity: 1 }}
            exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { height: 0, opacity: 0 }}
            transition={reduce ? { duration: 0 } : { duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <ul className="mt-2 space-y-2 border-l border-vb-border/60 pl-3 text-[12px] leading-snug text-vb-silver-dim">
              {SENT_ITEMS.map((i) => (
                <li key={i.title}>
                  <span className="text-vb-silver">{i.title}:</span> {i.text}
                </li>
              ))}
              <li>Адреса, логины и пароли прокси, имя компьютера и пользователя не отправляются.</li>
            </ul>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/*
  Настройки: «Обновления и отчёты». Три переключателя пишутся сразу; при
  ошибке записи возвращаются обратно. Под ними - «Сообщить о проблеме».
*/
export function PrivacyCard() {
  const [state, setState] = useState<PrivacyState | null>(null);
  const [reportOpen, setReportOpen] = useState(false);
  const reportBtn = useRef<HTMLButtonElement>(null);

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
    <section className="surface-card mt-4 p-4" aria-labelledby="privacy-card-title">
      <h2 id="privacy-card-title" className="mb-3 text-[14px] font-medium text-vb-fg">
        Обновления и отчёты
      </h2>
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
        <>
          <PrivacySwitches idPrefix="settings" toggleSize="sm" values={state} onChange={change} />
          <WhatIsSent />
        </>
      )}
      <div className="mt-4 border-t border-vb-border/60 pt-3">
        <Button ref={reportBtn} variant="secondary" size="sm" onClick={() => setReportOpen(true)}>
          <MessageSquareWarning className="h-3.5 w-3.5" aria-hidden />
          Сообщить о проблеме
        </Button>
      </div>
      <ReportDialog open={reportOpen} onClose={() => setReportOpen(false)} returnFocusTo={reportBtn} />
    </section>
  );
}
