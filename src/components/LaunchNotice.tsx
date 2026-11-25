import { useState } from "react";
import { AlertTriangle, Loader2, RotateCw, X } from "lucide-react";
import { Button } from "./Button";
import { cn } from "../lib/cn";

interface Props {
  /** Текст: предложение перезапуска или причина, почему не открылось. */
  text: string;
  /** Показать кнопку «Перезапустить» (Claude Desktop открыт без прокси). */
  restart: boolean;
  /** Перезапуск: закрыть и открыть через мост. Ошибка - текстом вместо кнопки. */
  onRestart: () => Promise<unknown>;
  onClose?: () => void;
  className?: string;
}

/*
  Итог «Открыть»: Claude Desktop уже открыт без прокси - не ошибка, а вопрос
  с кнопкой «Перезапустить». Если перезапуск не вышел, здесь же текст из
  бэкенда в том же духе, что у других приложений: «… не закрылся - закройте
  его и нажмите «Открыть»».
*/
export function LaunchNotice({ text, restart, onRestart, onClose, className }: Props) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const shown = failed ?? text;
  const offer = restart && !failed;

  const run = async () => {
    setBusy(true);
    try {
      await onRestart();
      onClose?.();
    } catch (e) {
      setFailed(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      role={offer ? "status" : "alert"}
      className={cn(
        "flex items-start gap-2.5 rounded-xl border px-3.5 py-2.5 text-[13px] leading-snug",
        offer ? "border-vb-warn/30 bg-vb-warn/[0.06] text-vb-silver" : "border-vb-loss/30 bg-vb-loss/[0.06] text-vb-loss",
        className,
      )}
    >
      <AlertTriangle className={cn("mt-0.5 h-4 w-4 shrink-0", offer ? "text-vb-warn" : "text-vb-loss")} aria-hidden />
      <span className="min-w-0 flex-1">{shown}</span>
      {offer && (
        <Button size="sm" variant="secondary" onClick={() => void run()} disabled={busy} className="-my-1 shrink-0">
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : <RotateCw className="h-3.5 w-3.5" aria-hidden />}
          Перезапустить
        </Button>
      )}
      {onClose && !busy && (
        <button
          type="button"
          onClick={onClose}
          aria-label="Скрыть"
          className="-my-0.5 shrink-0 rounded p-1 text-vb-silver-faint hover:text-vb-silver focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-vb-emerald/50"
        >
          <X className="h-3.5 w-3.5" aria-hidden />
        </button>
      )}
    </div>
  );
}
