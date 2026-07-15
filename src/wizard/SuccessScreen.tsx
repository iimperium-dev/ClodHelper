import { useState } from "react";
import { motion, useReducedMotion } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, MessageSquare, MousePointer2, Terminal } from "lucide-react";
import { SuccessCheck } from "../components/SuccessCheck";
import { StatusPill } from "../components/StatusPill";
import { ActionCard } from "../components/ActionCard";
import { Button } from "../components/Button";
import { cn } from "../lib/cn";
import { LogLine } from "./StepInstall";
import type { InstallMode } from "../lib/types";

interface Props {
  mode: InstallMode | null;
  cursor: boolean;
  log: string[];
  onBack: () => void;
  onInstallCodeToo?: () => void;
}

const EASE_OUT: [number, number, number, number] = [0.16, 1, 0.3, 1];

export function SuccessScreen({
  mode,
  cursor,
  log,
  onBack,
  onInstallCodeToo,
}: Props) {
  const reduce = useReducedMotion();
  const [logOpen, setLogOpen] = useState(false);

  // Sequenced reveal — see motion spec.
  const fadeUp = (delay: number) => ({
    initial: { opacity: 0, y: reduce ? 0 : 8 },
    animate: { opacity: 1, y: 0 },
    transition: { duration: 0.4, delay: reduce ? 0 : delay, ease: EASE_OUT },
  });

  async function handleOpenDesktop() {
    try {
      await invoke("launch_claude_desktop");
    } catch (e) {
      console.error("Failed to launch Claude Desktop", e);
    }
  }

  async function handleOpenTerminal() {
    try {
      await invoke("launch_claude_code");
    } catch (e) {
      console.error("Failed to launch Claude Code", e);
    }
  }

  async function handleOpenCursor() {
    try {
      await invoke("launch_cursor");
    } catch (e) {
      console.error("Failed to launch Cursor", e);
    }
  }

  const showDesktop = mode === "desktop" || mode === "both";
  const showCode = mode === "code" || mode === "both";
  const showCursor = cursor;
  const primaryAction = showDesktop
    ? handleOpenDesktop
    : showCode
      ? handleOpenTerminal
      : handleOpenCursor;
  const primaryLabel = showDesktop
    ? "Открыть Claude"
    : showCode
      ? "Запустить Code"
      : "Открыть Cursor";

  const pills: { label: string; pulse?: boolean }[] = [];
  pills.push({ label: "Подключение работает", pulse: true });
  if (showCode) pills.push({ label: "Claude Code на месте" });
  if (showDesktop) pills.push({ label: "Ярлык на рабочем столе" });
  if (showCursor) pills.push({ label: "Cursor (proxy) готов" });
  pills.push({ label: "Готово к запуску" });

  return (
    <div className="relative mx-auto flex max-w-[640px] flex-col px-2 pt-2 pb-10">
      {/* Ambient glow */}
      <div className="pointer-events-none fixed inset-x-0 top-0 z-0 mx-auto h-[420px] max-w-[800px]">
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: reduce ? 0.06 : [0.04, 0.09, 0.04] }}
          transition={
            reduce
              ? { duration: 0.6 }
              : { duration: 8, repeat: Infinity, ease: "easeInOut" }
          }
          className="h-full w-full bg-[radial-gradient(ellipse_at_top,rgba(16,185,129,0.12)_0%,transparent_60%)] blur-3xl"
        />
      </div>

      <div className="relative z-10">
        {/* Hero */}
        <div className="mt-2 flex flex-col items-center text-center">
          <SuccessCheck />
          <motion.h1
            {...fadeUp(0.35)}
            className="mt-7 text-[40px] font-medium leading-[1.05] tracking-[-0.03em] text-vb-silver"
          >
            Можно работать
          </motion.h1>
          <motion.p
            {...fadeUp(0.48)}
            className="mt-3 max-w-md text-[15px] leading-[1.5] tracking-[-0.01em] text-vb-silver-dim"
          >
            {mode
              ? "Claude установлен и подключён. Открывайте — он уже ждёт."
              : "Cursor подключён через прокси. Запускайте — всё готово."}
          </motion.p>
        </div>

        {/* Status pills */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.3, delay: reduce ? 0 : 0.9 }}
          className="mt-9 flex flex-wrap items-center justify-center gap-2"
        >
          {pills.map((p, i) => (
            <motion.div
              key={p.label}
              initial={{ opacity: 0, y: reduce ? 0 : 6 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{
                duration: 0.28,
                delay: reduce ? 0 : 0.9 + i * 0.08,
                ease: EASE_OUT,
              }}
            >
              <StatusPill label={p.label} pulse={p.pulse} />
            </motion.div>
          ))}
        </motion.div>

        {/* Next actions */}
        <div className="mt-10 space-y-2">
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ duration: 0.3, delay: reduce ? 0 : 1.28 }}
            className="mb-3 text-center text-[11px] font-medium uppercase tracking-[0.14em] text-vb-silver-dim/70"
          >
            Что дальше
          </motion.div>

          {showDesktop && (
            <motion.div
              initial={{ opacity: 0, y: reduce ? 0 : 12, scale: reduce ? 1 : 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              transition={{ duration: 0.36, delay: reduce ? 0 : 1.28, ease: EASE_OUT }}
            >
              <ActionCard
                icon={<MessageSquare className="h-5 w-5" strokeWidth={1.5} />}
                title="Открыть Claude"
                description="Чат-приложение — как ChatGPT, только Claude."
                onClick={handleOpenDesktop}
              />
            </motion.div>
          )}

          {showCode && (
            <motion.div
              initial={{ opacity: 0, y: reduce ? 0 : 12, scale: reduce ? 1 : 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              transition={{
                duration: 0.36,
                delay: reduce ? 0 : 1.38,
                ease: EASE_OUT,
              }}
            >
              <ActionCard
                icon={<Terminal className="h-5 w-5" strokeWidth={1.5} />}
                title="Запустить Code"
                description="Claude, который умеет работать с вашими файлами и проектами."
                onClick={handleOpenTerminal}
              />
            </motion.div>
          )}

          {showCursor && (
            <motion.div
              initial={{ opacity: 0, y: reduce ? 0 : 12, scale: reduce ? 1 : 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              transition={{
                duration: 0.36,
                delay: reduce ? 0 : 1.46,
                ease: EASE_OUT,
              }}
            >
              <ActionCard
                icon={<MousePointer2 className="h-5 w-5" strokeWidth={1.5} />}
                title="Открыть Cursor"
                description="Запустится через прокси-мост (ярлык «Cursor (proxy)»)."
                onClick={handleOpenCursor}
              />
            </motion.div>
          )}

          {showDesktop && !showCode && onInstallCodeToo && (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: 0.3, delay: reduce ? 0 : 1.5 }}
              className="pt-2 text-center"
            >
              <button
                type="button"
                onClick={onInstallCodeToo}
                className="inline-flex items-center gap-1 text-[12px] text-vb-silver-dim transition-colors hover:text-vb-silver"
              >
                Добавить Claude в редактор кода
                <ChevronRight className="h-3 w-3" />
              </button>
            </motion.div>
          )}
        </div>

        {/* Details accordion */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.3, delay: reduce ? 0 : 1.6 }}
          className="mt-10"
        >
          <button
            type="button"
            onClick={() => setLogOpen((v) => !v)}
            className="mx-auto flex items-center gap-1.5 text-[12px] text-vb-silver-dim/70 transition-colors hover:text-vb-silver-dim"
          >
            <ChevronRight
              className={cn(
                "h-3 w-3 transition-transform duration-200",
                logOpen && "rotate-90",
              )}
            />
            Подробности установки
          </button>
          <motion.div
            initial={false}
            animate={{
              height: logOpen ? "auto" : 0,
              opacity: logOpen ? 1 : 0,
            }}
            transition={{ duration: 0.24, ease: EASE_OUT }}
            className="overflow-hidden"
          >
            <div className="mt-4 rounded-xl border border-white/[0.04] bg-black/40 p-4">
              <div className="max-h-[200px] overflow-y-auto pr-1">
                {log.length === 0 ? (
                  <div className="font-mono text-[11px] text-vb-silver-dim/60">
                    Лог пуст.
                  </div>
                ) : (
                  log.map((line, i) => <LogLine key={i} line={line} />)
                )}
              </div>
            </div>
          </motion.div>
        </motion.div>
      </div>

      {/* Footer */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 0.3, delay: reduce ? 0 : 1.78 }}
        className="relative z-10 mt-10 flex items-center justify-between border-t border-white/[0.04] pt-6"
      >
        <Button variant="ghost" onClick={onBack}>
          Свернуть
        </Button>
        <Button
          onClick={primaryAction}
          className="shadow-[0_0_24px_rgba(16,185,129,0.25)] hover:shadow-[0_0_32px_rgba(16,185,129,0.4)]"
        >
          {primaryLabel}
        </Button>
      </motion.div>
    </div>
  );
}

