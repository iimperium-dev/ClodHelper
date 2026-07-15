import { useEffect, useMemo, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { AnimatePresence, motion } from "framer-motion";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Loader2,
  XCircle,
  Terminal as TerminalIcon,
  ChevronDown,
  ChevronRight,
  Download,
} from "lucide-react";
import { Button } from "../components/Button";
import type { InstallMode } from "../lib/types";
import { runInstall } from "../lib/api";
import { cn } from "../lib/cn";
import { SuccessScreen } from "./SuccessScreen";

const VERBOSE_PREFIX = "__verbose__:";
// How long between revealing successive log lines (ms). Lower = faster
// typewriter. The Rust side already streams live; this just paces lines that
// arrive in tight bursts so the UI feels alive instead of dumping a wall.
const REVEAL_INTERVAL_MS = 110;

interface Props {
  mode: InstallMode | null;
  cursor: boolean;
  proxyUrl: string;
  onBack: () => void;
  onDone: () => void;
  onRestartWithCode?: () => void;
}

type Status =
  | "idle"
  | "running"
  | "awaiting-desktop"
  | "done"
  | "error";

export function StepInstall({
  mode,
  cursor,
  proxyUrl,
  onBack,
  onDone: _onDone,
  onRestartWithCode,
}: Props) {
  const [status, setStatus] = useState<Status>("idle");
  // pending = raw events from backend, revealed = paced output for the UI
  const pendingRef = useRef<string[]>([]);
  const [revealed, setRevealed] = useState<string[]>([]);
  const [verbose, setVerbose] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const desktopMissingRef = useRef(false);
  const logRef = useRef<HTMLDivElement>(null);

  const visibleLog = useMemo(() => {
    const stripped = verbose
      ? revealed.map((l) =>
          l.startsWith(VERBOSE_PREFIX) ? l.slice(VERBOSE_PREFIX.length) : l,
        )
      : revealed.filter((l) => !l.startsWith(VERBOSE_PREFIX));
    return verbose ? stripped : prettifyLog(stripped);
  }, [revealed, verbose]);

  // Subscribe to backend events.
  useEffect(() => {
    let unlistenLog: UnlistenFn | null = null;
    let unlistenMissing: UnlistenFn | null = null;
    let cancelled = false;
    (async () => {
      const uLog = await listen<string>("install:log", (e) => {
        pendingRef.current.push(e.payload);
      });
      const uMiss = await listen("install:claude_desktop_missing", () => {
        desktopMissingRef.current = true;
      });
      if (cancelled) {
        uLog();
        uMiss();
      } else {
        unlistenLog = uLog;
        unlistenMissing = uMiss;
      }
    })();
    return () => {
      cancelled = true;
      unlistenLog?.();
      unlistenMissing?.();
    };
  }, []);

  // Reveal one line at a time from pending → revealed. If the pending queue
  // backs up (script dumped 40 lines at once), we still pace at the same
  // interval so the user gets a feel of progress.
  const lastLineAtRef = useRef<number>(0);
  const [stalled, setStalled] = useState(false);
  useEffect(() => {
    if (status !== "running" && status !== "done" && status !== "error") return;
    lastLineAtRef.current = Date.now();
    setStalled(false);
    const t = window.setInterval(() => {
      if (pendingRef.current.length === 0) {
        // No new lines for a while — show "still working" reassurance.
        if (status === "running" && Date.now() - lastLineAtRef.current > 20000) {
          setStalled(true);
        }
        return;
      }
      const next = pendingRef.current.shift()!;
      setRevealed((prev) => [...prev, next]);
      lastLineAtRef.current = Date.now();
      setStalled(false);
    }, REVEAL_INTERVAL_MS);
    return () => window.clearInterval(t);
  }, [status]);

  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [visibleLog]);

  async function handleStart() {
    setStatus("running");
    setError(null);
    setRevealed([]);
    pendingRef.current = [];
    desktopMissingRef.current = false;
    try {
      await runInstall({ mode, cursor, proxyUrl });
      // Don't flip to "done" until the typewriter has caught up — otherwise
      // the success banner appears while the log is still scrolling.
      await drainPending(pendingRef);
      // Desktop-mode and Claude Desktop wasn't found → pause for the user
      // to install it, then they hit "Продолжить" and we re-run the script.
      const needsDesktopPause =
        (mode === "desktop" || mode === "both") && desktopMissingRef.current;
      setStatus(needsDesktopPause ? "awaiting-desktop" : "done");
    } catch (e: any) {
      await drainPending(pendingRef);
      setError(typeof e === "string" ? e : (e?.message ?? "Неизвестная ошибка"));
      setStatus("error");
    }
  }

  async function handleContinueAfterDesktopInstall() {
    // Append a "5/5 retry" header to the existing log instead of clearing it.
    pendingRef.current.push("");
    pendingRef.current.push("▸ Повторная установка ярлыка…");
    desktopMissingRef.current = false;
    setStatus("running");
    try {
      await runInstall({ mode, cursor, proxyUrl });
      await drainPending(pendingRef);
      const stillMissing =
        (mode === "desktop" || mode === "both") && desktopMissingRef.current;
      setStatus(stillMissing ? "awaiting-desktop" : "done");
    } catch (e: any) {
      await drainPending(pendingRef);
      setError(typeof e === "string" ? e : (e?.message ?? "Неизвестная ошибка"));
      setStatus("error");
    }
  }

  // Success state — completely separate visual layout (no header / log dump).
  if (status === "done") {
    const cleaned = prettifyLog(
      revealed.filter((l) => !l.startsWith(VERBOSE_PREFIX)),
    );
    return (
      <SuccessScreen
        mode={mode}
        cursor={cursor}
        log={cleaned}
        onBack={onBack}
        onInstallCodeToo={onRestartWithCode}
      />
    );
  }

  return (
    <div className="mx-auto max-w-2xl pt-4">
      <h2 className="text-[22px] font-semibold tracking-tight text-vb-silver">
        {status === "idle" && "Готовы поставить?"}
        {status === "running" && "Устанавливаем Claude…"}
        {status === "awaiting-desktop" && "Остался один шаг"}
        {status === "error" && "Что-то пошло не так"}
      </h2>
      <p className="mt-1.5 text-[13px] text-vb-silver-dim">
        {status === "idle" &&
          "Это займёт 1-3 минуты. Не закрывайте окно — мы покажем прогресс."}
        {status === "running" &&
          "Может возникнуть запрос от Windows — разрешите."}
        {status === "awaiting-desktop" &&
          "Подключение настроено. Осталось установить само приложение Claude — и мы добавим ярлык на рабочий стол."}
      </p>

      {status === "idle" && (
        <div className="mt-6 flex justify-center">
          <Button onClick={handleStart} className="px-8">
            Установить
          </Button>
        </div>
      )}

      {(status === "running" ||
        status === "error" ||
        status === "awaiting-desktop") && (
        <div className="mt-6 glass-card overflow-hidden">
          <div className="flex items-center justify-between gap-2 border-b border-vb-border/60 px-4 py-2.5">
            <div className="flex items-center gap-2 text-[11px] uppercase tracking-wider text-vb-silver-dim">
              <TerminalIcon className="h-3.5 w-3.5" />
              Журнал установки
              {status === "running" && (
                <Loader2 className="ml-1 h-3 w-3 animate-spin text-vb-silver-dim" />
              )}
            </div>
            <button
              type="button"
              onClick={() => setVerbose((v) => !v)}
              className={cn(
                "flex items-center gap-1 rounded-md px-2 py-1 text-[11px] transition-colors",
                verbose
                  ? "bg-vb-surface/60 text-vb-silver"
                  : "text-vb-silver-dim hover:text-vb-silver",
              )}
            >
              {verbose ? (
                <ChevronDown className="h-3 w-3" />
              ) : (
                <ChevronRight className="h-3 w-3" />
              )}
              Подробно
            </button>
          </div>
          <div
            ref={logRef}
            className="max-h-[420px] min-h-[180px] overflow-y-auto px-5 py-4 text-vb-silver"
          >
            {visibleLog.length === 0 && status === "running" && (
              <div className="flex items-center gap-2 text-vb-silver-dim">
                <Loader2 className="h-3.5 w-3.5 animate-spin" /> Запускаем…
              </div>
            )}
            <AnimatePresence initial={false}>
              {visibleLog.map((line, i) => (
                <LogLine key={`${i}:${line}`} line={line} />
              ))}
            </AnimatePresence>
          </div>
        </div>
      )}

      <AnimatePresence>
        {status === "running" && stalled && (
          <motion.div
            key="stalled"
            initial={{ opacity: 0, y: -4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            className="mt-3 flex items-center gap-2 rounded-lg border border-vb-border/60 bg-vb-surface/40 px-4 py-2.5 text-[12px] text-vb-silver-dim"
          >
            <Loader2 className="h-3.5 w-3.5 animate-spin text-vb-emerald" />
            Скачиваем пакет с npm-registry. Это может занять до 2 минут на
            медленном соединении — не закрывайте окно.
          </motion.div>
        )}
      </AnimatePresence>

      {error && (
        <div className="mt-4 flex items-start gap-2 rounded-lg border border-vb-loss/30 bg-vb-loss/5 px-4 py-3 text-[13px] text-vb-loss">
          <XCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <div>
            <div className="font-medium">Установка не завершилась</div>
            <div className="mt-1 font-mono text-[12px] opacity-90">{error}</div>
          </div>
        </div>
      )}

      {status === "awaiting-desktop" && (
        <motion.div
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.25 }}
          className="mt-5 overflow-hidden rounded-2xl border border-white/[0.06] bg-[rgba(10,10,10,0.55)] backdrop-blur-2xl"
        >
          <div className="border-l-2 border-vb-silver/40 px-6 py-5">
            <div className="text-[15px] font-medium tracking-tight text-vb-silver">
              Скачайте Claude
            </div>
            <p className="mt-2 max-w-md text-[13px] leading-relaxed text-vb-silver-dim">
              Claude Desktop — официальное приложение от Anthropic. Скачайте
              его и установите как обычную программу. Когда закончите —
              вернитесь сюда, мы добавим ярлык.
            </p>
            <div className="mt-4 flex flex-wrap items-center gap-3">
              <Button
                onClick={() => openUrl("https://claude.ai/download")}
              >
                <Download className="h-4 w-4" />
                Скачать с сайта Anthropic
              </Button>
              <button
                type="button"
                onClick={handleContinueAfterDesktopInstall}
                className="text-[13px] font-medium text-vb-silver-dim transition-colors hover:text-vb-silver"
              >
                Я установил →
              </button>
            </div>
          </div>
        </motion.div>
      )}

      <div className="mt-8 flex items-center justify-between">
        <Button
          variant="ghost"
          onClick={onBack}
          disabled={status === "running" || status === "awaiting-desktop"}
        >
          Назад
        </Button>
        {status === "error" && (
          <Button variant="secondary" onClick={handleStart}>
            Повторить
          </Button>
        )}
      </div>
    </div>
  );
}

// Render a single log line with style based on its kind.
export function LogLine({ line }: { line: string }) {
  // Section header — was inside ASCII ┌─┐ frame.
  if (line.startsWith("## ")) {
    const title = line.slice(3);
    return (
      <motion.div
        initial={{ opacity: 0, y: 4 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.2, ease: "easeOut" }}
        className="mt-3 mb-1 flex items-center gap-2 first:mt-0"
      >
        <div className="h-px flex-1 bg-white/[0.06]" />
        <span className="font-sans text-[10px] font-medium uppercase tracking-[0.14em] text-vb-silver-dim/80">
          {title}
        </span>
        <div className="h-px flex-1 bg-white/[0.06]" />
      </motion.div>
    );
  }

  const trimmed = line.trim();
  const isStep = /^\[\d\/\d\]/.test(trimmed);
  const isOk = trimmed.startsWith("✓") || trimmed.startsWith("✔");
  const isWarn = trimmed.startsWith("⚠");
  const isErr = trimmed.startsWith("✗") || line.startsWith("[stderr]");
  const isArrow = trimmed.startsWith("▸") || trimmed.startsWith("↓");
  const isInternal =
    line.startsWith("[exec]") ||
    line.startsWith("[exit]") ||
    line.startsWith("[diag") ||
    /^[⓵⓶⓷⓸]/.test(line) ||
    line.startsWith("──") ||
    /^D[1-6]:/.test(line);

  return (
    <motion.div
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: "easeOut" }}
      className={cn(
        "whitespace-pre-wrap font-mono text-[11px] leading-[1.7]",
        isOk && "text-vb-emerald/90",
        isWarn && "text-vb-warn",
        isErr && "text-vb-loss",
        isStep && "mt-1 font-medium text-vb-silver",
        isArrow && "text-vb-silver",
        isInternal && "text-vb-silver-dim/60",
        !isOk &&
          !isWarn &&
          !isErr &&
          !isStep &&
          !isArrow &&
          !isInternal &&
          "text-vb-silver-dim",
      )}
    >
      {line}
    </motion.div>
  );
}

// Clean PowerShell raw output: drop ASCII box-drawing frames, hint-blocks,
// long Windows paths, and other noise that scares non-technical users.
// Keeps the meaningful step lines and success/warning markers.
function prettifyLog(lines: string[]): string[] {
  const out: string[] = [];
  let skipHintBlock = false;
  for (const raw of lines) {
    const line = raw.replace(/\r$/, "");
    const trimmed = line.trim();

    // Drop ASCII frames. Note: PowerShell sometimes downgrades Unicode
    // box-drawing chars (│ U+2502) to plain ASCII pipe (| U+007C) when
    // streams pass through utf-8/cp866 conversion. So we accept both.
    const isHorizontalBorder = /^[┌┐└┘├┤┬┴┼─\s]+$/.test(line) && /[┌┐└┘├┤┬┴┼─]/.test(line);
    if (isHorizontalBorder) continue;

    const vertMatch = trimmed.match(/^[│|]\s*(.+?)\s*[│|]$/);
    if (vertMatch && vertMatch[1].trim().length > 0) {
      out.push(`## ${vertMatch[1].trim()}`);
      continue;
    }
    // Single dangling vertical bar with only whitespace — drop it.
    if (/^[│|\s]+$/.test(line) && /[│|]/.test(line)) continue;

    // PowerShell command hints / "что дальше" blocks — noise for end users.
    if (/^Сменить прокси:/.test(trimmed)) {
      skipHintBlock = true;
      continue;
    }
    if (/^Проверить что прокси работает:/.test(trimmed)) {
      skipHintBlock = true;
      continue;
    }
    if (skipHintBlock) {
      if (trimmed === "" || /^\s/.test(line)) continue;
      skipHintBlock = false;
    }

    // .\claude-…-setup.ps1 lines — internal command hints.
    if (/^\.\\claude-.+\.ps1/.test(trimmed)) continue;
    // netstat hint.
    if (/^netstat -an/.test(trimmed)) continue;
    // Verbose "Что дальше:" + numbered instructions inside script — we have
    // our own better next-actions on the success screen.
    if (/^Что дальше:/.test(trimmed)) {
      skipHintBlock = true;
      continue;
    }
    // Empty lines collapse to one.
    if (trimmed === "" && out[out.length - 1] === "") continue;

    out.push(line);
  }
  // Trim leading/trailing empty lines.
  while (out.length && out[0].trim() === "") out.shift();
  while (out.length && out[out.length - 1].trim() === "") out.pop();
  return out;
}

async function drainPending(ref: React.MutableRefObject<string[]>) {
  // Wait until the typewriter interval has emptied the queue, with a safety
  // cap so we never block forever.
  const start = Date.now();
  while (ref.current.length > 0 && Date.now() - start < 8000) {
    await new Promise((r) => setTimeout(r, REVEAL_INTERVAL_MS));
  }
}
