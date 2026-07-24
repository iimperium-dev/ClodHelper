import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, Check, Loader2, RotateCw } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "../components/Button";
import { wizardInstall } from "../lib/api";
import type { WizardInstallResult } from "../lib/types";
import { cn } from "../lib/cn";

interface Props {
  proxyUrl: string;
  onBack: () => void;
  onDone: (result: WizardInstallResult) => void;
}

const PHASES: { id: string; label: string }[] = [
  { id: "bridge", label: "Установка моста" },
  { id: "task", label: "Автозапуск и старт моста" },
  { id: "env", label: "Системное проксирование" },
  { id: "presets", label: "Профили приложений" },
  { id: "test", label: "Проверка соединения" },
];

type PhaseState = "pending" | "run" | "ok" | "err";

/*
  Шаг 2 нового мастера: установка стартует сама, без кнопки. Всё делает одна
  Rust-команда wizard_install (существующие пути: мост из ресурсов, задача
  Планировщика, env, пресеты, честный shim-тест) — легаси PS-скрипты и
  проверки Node/Python отсюда исчезли. Прогресс — события "wizard:step".
*/
export function StepInstall({ proxyUrl, onBack, onDone }: Props) {
  const [states, setStates] = useState<Record<string, PhaseState>>({});
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const started = useRef(false);

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
        [phase]: state === "start" ? "run" : state === "ok" ? "ok" : "err",
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

  return (
    <div className="mx-auto max-w-2xl pt-4">
      <h2 className="text-[22px] font-semibold tracking-tight text-vb-silver">
        Настраиваю всё сам
      </h2>
      <p className="mt-1.5 text-[13px] text-vb-silver-dim">
        Обычно это занимает меньше минуты.
      </p>

      <div className="surface-card mt-5 divide-y divide-vb-border/60">
        {PHASES.map(({ id, label }) => {
          const st: PhaseState = states[id] ?? "pending";
          // env — единственная нефатальная фаза: err здесь = «включите позже».
          const softErr = id === "env" && st === "err";
          return (
            <div key={id} className="flex items-center gap-3 px-4 py-3">
              <span className="flex h-5 w-5 shrink-0 items-center justify-center">
                {st === "run" ? (
                  <Loader2 className="h-4 w-4 animate-spin text-vb-silver-dim" />
                ) : st === "ok" ? (
                  <motion.span
                    initial={{ scale: 0 }}
                    animate={{ scale: 1 }}
                    transition={{ type: "spring", stiffness: 500, damping: 24 }}
                  >
                    <Check className="h-4 w-4 text-vb-emerald" />
                  </motion.span>
                ) : st === "err" ? (
                  <AlertTriangle
                    className={cn("h-4 w-4", softErr ? "text-vb-warn" : "text-vb-loss")}
                  />
                ) : (
                  <span className="h-1.5 w-1.5 rounded-full bg-vb-border" />
                )}
              </span>
              <span
                className={cn(
                  "text-[13px]",
                  st === "pending" ? "text-vb-silver-faint" : "text-vb-silver",
                )}
              >
                {label}
                {softErr && (
                  <span className="ml-2 text-[11px] text-vb-warn">
                    не включилось — тумблер есть в Обзоре
                  </span>
                )}
              </span>
            </div>
          );
        })}
      </div>

      <AnimatePresence>
        {error && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            className="overflow-hidden"
          >
            <div className="mt-3 rounded-lg border border-vb-loss/35 bg-vb-loss/[0.06] p-3 text-[12px] leading-relaxed text-vb-loss">
              {error}
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <div className="mt-8 flex items-center justify-between">
        <Button variant="ghost" onClick={onBack} disabled={running}>
          Назад
        </Button>
        {error && (
          <Button onClick={() => void run()} disabled={running}>
            <RotateCw className="h-3.5 w-3.5" />
            Повторить
          </Button>
        )}
      </div>
    </div>
  );
}
