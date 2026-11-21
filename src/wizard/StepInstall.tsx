import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { AlertTriangle, ArrowLeft, Check, Loader2, RotateCw } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "../components/Button";
import { WizardFooter, WizardTitle } from "../components/WizardShell";
import { wizardInstall } from "../lib/api";
import type { WizardInstallResult } from "../lib/types";
import { cn } from "../lib/cn";
import { isMac } from "../lib/platform";

interface Props {
  proxyUrl: string;
  onBack: () => void;
  onDone: (result: WizardInstallResult) => void;
}

const PHASES: { id: string; label: string }[] = [
  { id: "bridge", label: "Устанавливаю мост" },
  { id: "task", label: "Включаю автозапуск" },
  { id: "env", label: "Направляю терминал и VS Code через мост" },
  { id: "presets", label: "Готовлю профили приложений" },
  // macOS: Claude просит пароль Mac один раз. Telegram мастер не трогает -
  // он подключается по выбору в «Приложениях».
  ...(isMac ? [{ id: "claude", label: "Подключаю Claude Desktop - macOS спросит пароль" }] : []),
  { id: "test", label: "Проверяю соединение настоящим запросом" },
];

type PhaseState = "pending" | "run" | "ok" | "err";

/*
  Шаг 2: установка стартует сама, без кнопки. Всё делает одна Rust-команда
  wizard_install (мост из ресурсов, автозапуск, env, пресеты, честный
  shim-тест). Прогресс - события "wizard:step" вида "фаза:start|ok|skip|err".
  Фазы - маршрут сверху вниз: линия между точками заливается по мере хода.
*/
export function StepInstall({ proxyUrl, onBack, onDone }: Props) {
  const [states, setStates] = useState<Record<string, PhaseState>>({});
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const started = useRef(false);
  const reduce = useReducedMotion();

  const run = useCallback(async () => {
    setError(null);
    setStates({});
    setRunning(true);
    try {
      onDone(await wizardInstall(proxyUrl));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }, [proxyUrl, onDone]);

  useEffect(() => {
    const un = listen<string>("wizard:step", (e) => {
      const idx = e.payload.indexOf(":");
      if (idx < 0) return;
      const phase = e.payload.slice(0, idx);
      const state = e.payload.slice(idx + 1);
      setStates((s) => ({
        ...s,
        // skip - приложения нет, шаг не нужен: показываем как выполненный.
        [phase]: state === "start" ? "run" : state === "ok" || state === "skip" ? "ok" : "err",
      }));
    });
    if (!started.current) {
      started.current = true;
      void run();
    }
    return () => {
      void un.then((f) => f());
    };
  }, [run]);

  const done = PHASES.filter((p) => states[p.id] === "ok" || (p.id === "env" && states[p.id] === "err")).length;
  const pct = Math.round((done / PHASES.length) * 100);

  return (
    <>
      <WizardTitle
        sub={
          error
            ? "Причина - ниже. Проверьте прокси или просто повторите."
            : "Обычно меньше минуты. Ничего нажимать не нужно."
        }
      >
        {error ? "Настройка не закончилась" : "Настраиваю всё сам"}
      </WizardTitle>

      <div
        className="mt-5 flex items-center gap-3"
        role="progressbar"
        aria-label="Ход настройки"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
      >
        <div className="h-1 flex-1 overflow-hidden rounded-full bg-vb-border">
          <div
            className={cn(
              "h-full origin-left rounded-full transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]",
              error ? "bg-vb-warn" : "bg-vb-emerald",
            )}
            style={{ transform: `scaleX(${pct / 100})` }}
          />
        </div>
        <span className="tnum w-9 text-right text-[12px] text-vb-silver-faint">{pct}%</span>
      </div>

      <ol className="mt-4" aria-label="Что настраивается">
        {PHASES.map(({ id, label }, i) => {
          const st: PhaseState = states[id] ?? "pending";
          // env - единственная нефатальная фаза: err здесь = «включите позже».
          const softErr = id === "env" && st === "err";
          const last = i === PHASES.length - 1;
          return (
            <li key={id} className="relative flex gap-3 pb-3 last:pb-0">
              {!last && (
                <span aria-hidden className="absolute left-[9px] top-6 bottom-0 w-px bg-vb-border">
                  <span
                    className="block h-full w-full origin-top bg-vb-emerald-dim transition-transform duration-300"
                    style={{ transform: `scaleY(${st === "ok" || softErr ? 1 : 0})` }}
                  />
                </span>
              )}
              <span className="relative z-[1] mt-0.5 flex h-[19px] w-[19px] shrink-0 items-center justify-center rounded-full bg-vb-bg">
                {st === "run" ? (
                  <Loader2 className="h-4 w-4 animate-spin text-vb-silver" aria-hidden />
                ) : st === "ok" ? (
                  <motion.span
                    initial={reduce ? false : { opacity: 0, scale: 0.6 }}
                    animate={{ opacity: 1, scale: 1 }}
                    transition={{ type: "spring", bounce: 0, duration: 0.3 }}
                    className="flex h-[19px] w-[19px] items-center justify-center rounded-full bg-vb-emerald/15"
                  >
                    <Check className="h-3 w-3 text-vb-emerald" strokeWidth={3} aria-hidden />
                  </motion.span>
                ) : st === "err" ? (
                  <AlertTriangle className={cn("h-4 w-4", softErr ? "text-vb-warn" : "text-vb-loss")} aria-hidden />
                ) : (
                  <span className="h-2 w-2 rounded-full border border-vb-border-strong" aria-hidden />
                )}
              </span>
              <span className="min-w-0">
                <span
                  className={cn(
                    "block text-[13px] leading-[19px] transition-colors duration-200",
                    st === "pending" ? "text-vb-silver-faint" : st === "run" ? "text-vb-fg" : "text-vb-silver",
                  )}
                >
                  {label}
                  <span className="sr-only">
                    {st === "run" ? " - идёт" : st === "ok" ? " - готово" : st === "err" ? " - ошибка" : ""}
                  </span>
                </span>
                {softErr && (
                  <span className="mt-0.5 block text-[12px] text-vb-warn">
                    Не включилось, можно позже: «Приложения», Claude Desktop, шестерёнка
                  </span>
                )}
              </span>
            </li>
          );
        })}
      </ol>

      <AnimatePresence initial={false}>
        {error && (
          <motion.div
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
            role="alert"
            className="mt-4 rounded-xl border border-vb-loss/35 bg-vb-loss/[0.06] px-4 py-3 text-[12px] leading-relaxed text-vb-loss"
          >
            {error}
          </motion.div>
        )}
      </AnimatePresence>

      <WizardFooter
        left={
          <Button variant="ghost" size="sm" onClick={onBack} disabled={running} className="-ml-3">
            <ArrowLeft className="h-3.5 w-3.5" aria-hidden />
            Другой прокси
          </Button>
        }
      >
        {error && (
          <Button onClick={() => void run()} disabled={running}>
            <RotateCw className="h-4 w-4" aria-hidden />
            Повторить
          </Button>
        )}
      </WizardFooter>
    </>
  );
}
