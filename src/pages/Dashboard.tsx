import { useCallback, useEffect, useRef, useState } from "react";
import { motion, useReducedMotion } from "framer-motion";
import {
  Activity,
  Copy,
  ExternalLink,
  Globe,
  Loader2,
  MousePointer2,
  Pause,
  Play,
  RotateCw,
  Settings as SettingsIcon,
  Wifi,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "../components/Button";
import {
  detectCursor,
  launchCursor,
  setupCursor,
  shimRestart,
  shimStart,
  shimStatus,
  shimStop,
  shimTest,
} from "../lib/api";
import type { CursorInfo, ShimStatus, ShimTestResult } from "../lib/types";
import { cn } from "../lib/cn";

const POLL_INTERVAL_MS = 5_000;

function formatUptime(sec?: number): string {
  if (!sec || sec < 0) return "—";
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  if (h > 0) return `${h}ч ${m}м`;
  if (m > 0) return `${m}м ${s}с`;
  return `${s}с`;
}

interface Props {
  onOpenSettings: () => void;
}

export function Dashboard({ onOpenSettings }: Props) {
  const reduce = useReducedMotion();
  const [status, setStatus] = useState<ShimStatus | null>(null);
  const [busy, setBusy] = useState<"start" | "stop" | "restart" | "test" | null>(null);
  const [testResult, setTestResult] = useState<ShimTestResult | null>(null);
  const [copied, setCopied] = useState(false);
  const [cursor, setCursor] = useState<CursorInfo | null>(null);
  const [cursorBusy, setCursorBusy] = useState(false);
  const [cursorMsg, setCursorMsg] = useState<string | null>(null);
  const intervalRef = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await shimStatus();
      setStatus(s);
    } catch {
      // ignore — leave stale
    }
  }, []);

  const refreshCursor = useCallback(async () => {
    try {
      setCursor(await detectCursor());
    } catch {
      // ignore
    }
  }, []);

  const handleCursorSetup = async () => {
    setCursorBusy(true);
    setCursorMsg(null);
    try {
      const r = await setupCursor();
      setCursorMsg(r.message);
      await refreshCursor();
      if (r.ok) {
        await launchCursor().catch(() => undefined);
      }
    } catch (e) {
      setCursorMsg(String(e));
    } finally {
      setCursorBusy(false);
    }
  };

  const handleCursorLaunch = async () => {
    setCursorBusy(true);
    setCursorMsg(null);
    try {
      await launchCursor();
    } catch (e) {
      setCursorMsg(String(e));
    } finally {
      setCursorBusy(false);
    }
  };

  useEffect(() => {
    void refresh();
    void refreshCursor();
    intervalRef.current = window.setInterval(() => {
      void refresh();
    }, POLL_INTERVAL_MS);
    return () => {
      if (intervalRef.current) {
        window.clearInterval(intervalRef.current);
      }
    };
  }, [refresh, refreshCursor]);

  const handleStart = async () => {
    setBusy("start");
    try {
      await shimStart();
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  const handleStop = async () => {
    setBusy("stop");
    try {
      await shimStop();
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  const handleRestart = async () => {
    setBusy("restart");
    try {
      await shimRestart();
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  const handleTest = async () => {
    setBusy("test");
    setTestResult(null);
    try {
      const r = await shimTest();
      setTestResult(r);
    } finally {
      setBusy(null);
    }
  };

  const copyUpstream = async () => {
    if (!status?.upstream_masked) return;
    try {
      await navigator.clipboard.writeText(status.upstream_masked);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // ignore
    }
  };

  const running = status?.running ?? false;

  return (
    <div className="flex h-full flex-col gap-6 p-8 text-vb-silver">
      {/* Header */}
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight text-white">
            Clod Helper
          </h1>
          <p className="mt-0.5 text-sm text-vb-silver-dim">
            Прокси-мост для Claude Code, Claude Desktop и Cursor
          </p>
        </div>
        <Button variant="ghost" onClick={onOpenSettings} aria-label="Настройки">
          <SettingsIcon className="h-4 w-4" />
          Настройки
        </Button>
      </header>

      {/* Status card */}
      <section className="rounded-2xl border border-white/[0.04] bg-[rgba(10,10,10,0.55)] p-6 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] backdrop-blur-xl">
        <div className="flex items-center gap-3">
          <span className="relative inline-block h-2.5 w-2.5">
            <span
              className={cn(
                "absolute inset-0 rounded-full",
                running
                  ? "bg-vb-emerald shadow-[0_0_12px_rgba(16,185,129,0.7)]"
                  : "bg-red-500 shadow-[0_0_8px_rgba(239,68,68,0.6)]",
              )}
            />
            {running && !reduce && (
              <motion.span
                className="absolute -inset-1 rounded-full bg-vb-emerald/40"
                animate={{ scale: [1, 1.7, 1], opacity: [0.5, 0, 0.5] }}
                transition={{
                  duration: 2.4,
                  repeat: Infinity,
                  ease: "easeInOut",
                }}
              />
            )}
          </span>
          <span className="text-lg font-semibold text-white">
            {running ? "Шим активен" : "Шим остановлен"}
          </span>
          {running && status?.pid !== undefined && (
            <span className="ml-auto rounded-md border border-vb-border bg-vb-surface px-2 py-0.5 font-mono text-xs text-vb-silver-dim">
              PID {status.pid}
            </span>
          )}
        </div>

        {/* Metrics grid */}
        <dl className="mt-6 grid grid-cols-2 gap-x-6 gap-y-4 text-sm">
          <div>
            <dt className="flex items-center gap-1.5 text-vb-silver-dim">
              <Activity className="h-3.5 w-3.5" />
              Время работы
            </dt>
            <dd className="mt-1 font-mono text-white">
              {running ? formatUptime(status?.uptime_sec) : "—"}
            </dd>
          </div>
          <div>
            <dt className="flex items-center gap-1.5 text-vb-silver-dim">
              <Wifi className="h-3.5 w-3.5" />
              Слушает
            </dt>
            <dd className="mt-1 font-mono text-white">
              {status?.listen ?? "—"}
            </dd>
          </div>
          <div className="col-span-2">
            <dt className="flex items-center gap-1.5 text-vb-silver-dim">
              <Globe className="h-3.5 w-3.5" />
              Upstream-прокси
            </dt>
            <dd className="mt-1 flex items-center gap-2">
              <span className="truncate font-mono text-white">
                {status?.upstream_masked ?? "—"}
              </span>
              {status?.upstream_masked && (
                <button
                  type="button"
                  onClick={copyUpstream}
                  className="text-vb-silver-dim hover:text-vb-silver"
                  title="Копировать"
                >
                  <Copy className="h-3.5 w-3.5" />
                </button>
              )}
              {copied && (
                <span className="text-xs text-vb-emerald">скопировано</span>
              )}
            </dd>
          </div>
        </dl>

        {/* Action buttons */}
        <div className="mt-6 flex flex-wrap gap-2">
          {running ? (
            <>
              <Button
                variant="primary"
                onClick={handleTest}
                disabled={busy !== null}
              >
                <Activity className="h-4 w-4" />
                {busy === "test" ? "Проверка…" : "Тест"}
              </Button>
              <Button
                variant="secondary"
                onClick={handleRestart}
                disabled={busy !== null}
              >
                <RotateCw className="h-4 w-4" />
                Перезапустить
              </Button>
              <Button
                variant="ghost"
                onClick={handleStop}
                disabled={busy !== null}
              >
                <Pause className="h-4 w-4" />
                Остановить
              </Button>
            </>
          ) : (
            <Button
              variant="primary"
              onClick={handleStart}
              disabled={busy !== null}
            >
              <Play className="h-4 w-4" />
              {busy === "start" ? "Запуск…" : "Запустить шим"}
            </Button>
          )}
        </div>
      </section>

      {/* Test result */}
      {testResult && (
        <section
          className={cn(
            "rounded-2xl border p-5 backdrop-blur-xl",
            testResult.ok
              ? "border-vb-emerald/30 bg-vb-emerald/[0.04]"
              : "border-red-500/30 bg-red-500/[0.04]",
          )}
        >
          <div className="flex items-center gap-2 text-sm">
            <span
              className={cn(
                "h-2 w-2 rounded-full",
                testResult.ok ? "bg-vb-emerald" : "bg-red-500",
              )}
            />
            <span className="font-medium text-white">
              {testResult.ok ? "Соединение работает" : "Не удалось проверить"}
            </span>
            {testResult.latency_ms !== undefined && (
              <span className="ml-auto font-mono text-xs text-vb-silver-dim">
                {testResult.latency_ms} ms
              </span>
            )}
          </div>
          <div className="mt-2 space-y-1 font-mono text-xs text-vb-silver-dim">
            {testResult.external_ip && (
              <div>
                <span className="text-vb-silver-dim">IP-выход:</span>{" "}
                <span className="text-white">{testResult.external_ip}</span>
              </div>
            )}
            {testResult.error && (
              <div className="text-red-300/80">{testResult.error}</div>
            )}
          </div>
        </section>
      )}

      {/* Apps — Cursor */}
      <section className="rounded-2xl border border-white/[0.04] bg-[rgba(10,10,10,0.55)] p-6 backdrop-blur-xl">
        <h2 className="text-base font-semibold text-white">Приложения</h2>
        <p className="mt-1 text-xs text-vb-silver-dim">
          Запуск через тот же мост. Для Cursor создаётся ярлык «Cursor (proxy)».
        </p>

        <div className="mt-4 flex items-center gap-4 rounded-xl border border-vb-border bg-vb-surface/40 p-4">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-vb-bg text-vb-silver-dim">
            <MousePointer2 className="h-5 w-5" strokeWidth={1.5} />
          </div>
          <div className="flex-1">
            <div className="text-[14px] font-medium text-vb-silver">Cursor IDE</div>
            <div className="text-[12px] text-vb-silver-dim">
              {cursor === null
                ? "Проверяю…"
                : !cursor.installed
                  ? "Не найден на этом ПК"
                  : cursor.shortcut_exists
                    ? "Ярлык «Cursor (proxy)» готов"
                    : "Установлен — нужен ярлык с прокси"}
            </div>
          </div>
          {cursor && !cursor.installed && (
            <Button
              variant="secondary"
              onClick={() => openUrl("https://cursor.com")}
            >
              Скачать
              <ExternalLink className="h-3 w-3" />
            </Button>
          )}
          {cursor && cursor.installed && cursor.shortcut_exists && (
            <Button
              variant="primary"
              onClick={handleCursorLaunch}
              disabled={cursorBusy}
            >
              {cursorBusy ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Play className="h-4 w-4" />
              )}
              Открыть
            </Button>
          )}
          {cursor && cursor.installed && !cursor.shortcut_exists && (
            <Button
              variant="primary"
              onClick={handleCursorSetup}
              disabled={cursorBusy}
            >
              {cursorBusy ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <MousePointer2 className="h-4 w-4" />
              )}
              Настроить
            </Button>
          )}
        </div>
        {cursorMsg && (
          <p className="mt-3 text-xs text-vb-silver-dim">{cursorMsg}</p>
        )}
      </section>

      <div className="mt-auto pt-2 text-center text-xs text-vb-silver-dim">
        <a
          href="https://github.com/mogylnyy/Clod-Helper-v1"
          className="inline-flex items-center gap-1 hover:text-vb-silver"
          target="_blank"
          rel="noreferrer"
        >
          GitHub
          <ExternalLink className="h-3 w-3" />
        </a>
      </div>
    </div>
  );
}
