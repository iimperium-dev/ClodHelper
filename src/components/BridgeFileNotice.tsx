import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Check, CheckCircle2, Copy, ExternalLink, Loader2, ShieldAlert } from "lucide-react";
import { Button } from "./Button";
import { bridgeFileDismiss, bridgeFileRetry, openWindowsSecurity } from "../lib/api";
import { isMac } from "../lib/platform";
import { useStatus } from "../lib/status";
import { cn } from "../lib/cn";

/** Путь папки: виден целиком, по кнопке — в буфер обмена. */
function PathCopy({ path }: { path: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="mt-1.5 flex min-w-0 items-start gap-1.5">
      <span className="min-w-0 break-all rounded-md border border-vb-border bg-vb-surface-2 px-2 py-1 font-mono text-[11px] leading-snug text-vb-fg">
        {path}
      </span>
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard.writeText(path).then(() => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1400);
          });
        }}
        className="flex shrink-0 items-center gap-1 rounded-md border border-vb-border px-2 py-1 text-[11px] font-medium text-vb-silver transition-colors hover:border-vb-border-strong hover:bg-vb-surface-2"
      >
        {copied ? <Check className="h-3 w-3 text-vb-emerald" /> : <Copy className="h-3 w-3" />}
        {copied ? "Скопировано" : "Скопировать"}
      </button>
    </div>
  );
}

const OpenSecurity = ({ label }: { label: string }) =>
  isMac ? null : (
    <button
      type="button"
      onClick={() => void openWindowsSecurity().catch(() => {})}
      className="mt-1.5 inline-flex items-center gap-1 rounded-md border border-vb-border px-2 py-1 text-[11px] font-medium text-vb-silver transition-colors hover:border-vb-border-strong hover:bg-vb-surface-2"
    >
      <ExternalLink className="h-3 w-3" />
      {label}
    </button>
  );

/** Шаги «добавить исключение» — общие для совета и для инструкции. */
function ExclusionSteps({ dirs, start = 1, open = true }: { dirs: string[]; start?: number; open?: boolean }) {
  return (
    <li className="flex gap-2.5">
      <Num n={start} />
      <div className="min-w-0 flex-1">
        {isMac
          ? "Добавьте папку в исключения вашего антивируса:"
          : "Добавьте папку в исключения: «Защита от вирусов и угроз» → «Управление настройками» → «Исключения» → «Добавить исключение» → «Папка»:"}
        {dirs.map((d) => (
          <PathCopy key={d} path={d} />
        ))}
        {open && <OpenSecurity label="Открыть «Безопасность Windows»" />}
      </div>
    </li>
  );
}

const Num = ({ n }: { n: number }) => (
  <span className="mt-px flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full bg-white/[0.06] text-[10.5px] text-vb-silver-dim">
    {n}
  </span>
);

/*
  Файл моста удалил антивирус. «Восстановлен» — программа вернула мост
  сама; совет добавить исключение, чтобы не повторилось. «Удаляет снова» /
  «нет копии» — инструкция по шагам с кнопками и «Проверить снова».
  Показывается на «Обзоре» и в «Прокси» (там чаще всего и всплывает).
*/
export function BridgeFileNotice({ className }: { className?: string }) {
  const { bridgeFile, setBridgeFile, refresh } = useStatus();
  const [howto, setHowto] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const retry = async () => {
    setBusy(true);
    setError(null);
    try {
      setBridgeFile(await bridgeFileRetry());
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const dismiss = () => {
    setBridgeFile(null);
    void bridgeFileDismiss().catch(() => {});
  };

  return (
    <AnimatePresence initial={false}>
      {bridgeFile && (
        <motion.div
          key={bridgeFile.kind}
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 4 }}
          transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
          className={className}
        >
          {bridgeFile.kind === "restored" ? (
            <div className="rounded-xl border border-vb-emerald/30 bg-vb-emerald/[0.06] p-3.5">
              <div className="flex items-center gap-2 text-[13px] font-semibold text-vb-emerald">
                <CheckCircle2 className="h-4 w-4 shrink-0" />
                Мост восстановлен
              </div>
              <p className="mt-1 text-[12px] leading-snug text-vb-silver">
                Его файл удалил антивирус — Clod Helper вернул его из своей копии, всё работает. Чтобы не повторилось,
                добавьте папку моста в исключения антивируса.
              </p>
              <AnimatePresence initial={false}>
                {howto && (
                  <motion.ol
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: "auto" }}
                    exit={{ opacity: 0, height: 0 }}
                    className="mt-2.5 overflow-hidden text-[12px] leading-snug text-vb-silver"
                  >
                    <ExclusionSteps dirs={bridgeFile.dirs} />
                  </motion.ol>
                )}
              </AnimatePresence>
              <div className="mt-2.5 flex gap-2">
                {!howto && (
                  <Button variant="secondary" size="sm" onClick={() => setHowto(true)}>
                    Как добавить исключение
                  </Button>
                )}
                <Button variant="ghost" size="sm" onClick={dismiss}>
                  Понятно
                </Button>
              </div>
            </div>
          ) : (
            <div className="rounded-xl border border-vb-loss/40 bg-vb-loss/[0.06] p-3.5">
              <div className="flex items-center gap-2 text-[13px] font-semibold text-vb-loss">
                <ShieldAlert className="h-4 w-4 shrink-0" />
                {bridgeFile.kind === "no_copy" ? "Антивирус удалил мост" : "Антивирус удаляет мост"}
              </div>
              <p className="mt-1 text-[12px] leading-snug text-vb-silver">
                {bridgeFile.kind === "no_copy"
                  ? "Удалён и мост, и его копия внутри Clod Helper — восстановить не из чего."
                  : "Clod Helper вернул файл моста, но антивирус сразу удалил его снова."}{" "}
                Пока мост не работает, Claude без интернета — напрямую с вашего IP ничего не уходит.
              </p>
              <ol className="mt-2.5 space-y-2.5 text-[12px] leading-snug text-vb-silver">
                <li className="flex gap-2.5">
                  <Num n={1} />
                  <div className="min-w-0 flex-1">
                    {isMac ? (
                      <>Разрешите файл <span className="font-mono text-vb-fg">clod-bridge</span> в карантине вашего антивируса.</>
                    ) : (
                      <>
                        Разрешите файл: «Защита от вирусов и угроз» → «Журнал защиты» → запись с{" "}
                        <span className="font-mono text-vb-fg">clod-bridge.exe</span> → «Действия» → «Восстановить».
                        Другой антивирус — то же в его карантине.
                      </>
                    )}
                    <div>
                      <OpenSecurity label="Открыть «Безопасность Windows»" />
                    </div>
                  </div>
                </li>
                <ExclusionSteps dirs={bridgeFile.dirs} start={2} open={false} />
                <li className="flex gap-2.5">
                  <Num n={3} />
                  <div className="min-w-0 flex-1">
                    {bridgeFile.kind === "no_copy"
                      ? "Вернитесь и нажмите «Проверить снова». Не вышло — переустановите Clod Helper с сайта (исключение добавьте до установки)."
                      : "Вернитесь и нажмите «Проверить снова» — Clod Helper восстановит и запустит мост."}
                    <div className="mt-1.5">
                      <Button variant="primary" size="sm" onClick={() => void retry()} disabled={busy}>
                        {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                        {busy ? "Проверяю…" : "Проверить снова"}
                      </Button>
                    </div>
                  </div>
                </li>
              </ol>
              {error && <p className={cn("mt-2 text-[12px] leading-snug text-vb-loss")}>{error}</p>}
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
