import { useEffect, useRef, useState } from "react";
import { Loader2, MessageSquareWarning } from "lucide-react";
import { Button } from "./Button";
import { PrivacySwitches } from "./PrivacySwitches";
import { ReportDialog } from "./ReportDialog";
import { privacyGet, privacySet } from "../lib/api";
import type { PrivacyKey, PrivacyState } from "../lib/types";

/*
  Настройки: «Обновления и отчёты». Три переключателя пишутся сразу; при
  ошибке записи возвращаются обратно. Под ними - «Сообщить о проблеме».
*/
export function PrivacyCard() {
  const [state, setState] = useState<PrivacyState | null>(null);
  const [reportOpen, setReportOpen] = useState(false);
  const reportBtn = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    void privacyGet()
      .then(setState)
      .catch(() => setState({ auto_update: false, crash_reports: true, usage_stats: true, offer: false }));
  }, []);

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
      {state === null ? (
        <Loader2 className="h-4 w-4 animate-spin text-vb-silver-dim" aria-hidden />
      ) : (
        <PrivacySwitches idPrefix="settings" values={state} onChange={change} />
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
