import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { ArrowRight, Check, Download, Loader2, RotateCw, ShieldCheck, Sparkles, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { Button } from "./Button";
import { updateInstall } from "../lib/api";
import { formatMb, onOpenUpdateDialog } from "../lib/updateBus";
import { cn } from "../lib/cn";
import type { UpdateInfo, UpdateProgress } from "../lib/types";

const RELEASES = "https://github.com/iimperium-dev/ClodHelper/releases/latest";

type Phase = "offer" | "installing" | "error";

const STEPS: { id: UpdateProgress["stage"]; title: string }[] = [
  { id: "download", title: "Скачивание" },
  { id: "verify", title: "Проверка целостности" },
  { id: "install", title: "Установка" },
  { id: "restart", title: "Перезапуск" },
];

/*
  Окно обновления — одно на всё приложение (App shell). Открывается кнопкой
  «Обновить» в баннере или кликом по уведомлению в углу экрана. Показывает,
  что нового, а после «Обновить» — живой ход установки: кольцо прогресса
  скачивания и шаги. Пока идёт установка, окно не закрывается.
*/
export function UpdateDialog() {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [phase, setPhase] = useState<Phase>("offer");
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [current, setCurrent] = useState("");
  // Подписки ставятся один раз — текущая фаза читается через ref.
  const phaseRef = useRef<Phase>(phase);
  phaseRef.current = phase;

  useEffect(() => {
    void getVersion().then(setCurrent).catch(() => {});
    const offBus = onOpenUpdateDialog((i) => {
      if (phaseRef.current !== "installing") setInfo(i);
    });
    const unlisten = listen<UpdateInfo>("update:open", (e) => {
      if (phaseRef.current !== "installing") setInfo(e.payload);
    });
    const unProgress = listen<UpdateProgress>("update:progress", (e) => setProgress(e.payload));
    return () => {
      offBus();
      void unlisten.then((f) => f());
      void unProgress.then((f) => f());
    };
  }, []);

  const close = () => {
    if (phase === "installing") return;
    setInfo(null);
    setPhase("offer");
    setError(null);
    setProgress(null);
  };

  useEffect(() => {
    if (!info) return;
    const esc = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  });

  const install = async () => {
    if (!info) return;
    setPhase("installing");
    setError(null);
    setProgress({ stage: "download", received: 0, total: 0 });
    try {
      await updateInstall(info);
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  };

  const highlights = info?.highlights?.length ? info.highlights : null;

  return createPortal(
    <AnimatePresence>
      {info && (
        <motion.div
          key="update-backdrop"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          onMouseDown={close}
          className="fixed inset-x-0 bottom-0 top-9 z-[80] flex items-center justify-center bg-black/50 p-5 backdrop-blur-[6px]"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.94, y: 14 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 8 }}
            transition={{ type: "spring", bounce: 0.2, duration: 0.45 }}
            onMouseDown={(e) => e.stopPropagation()}
            role="dialog"
            aria-label="Обновление Clod Helper"
            className="relative flex max-h-full w-full max-w-[420px] flex-col overflow-hidden rounded-2xl border border-vb-border bg-vb-surface shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
          >
            {/* Мягкое изумрудное свечение сверху */}
            <div
              aria-hidden
              className="pointer-events-none absolute inset-x-0 -top-24 h-48"
              style={{
                background: "radial-gradient(60% 100% at 50% 0%, var(--color-vb-emerald-glow), transparent 70%)",
              }}
            />
            {phase !== "installing" && (
              <button
                type="button"
                onClick={close}
                aria-label="Закрыть"
                className="absolute right-3 top-3 z-10 rounded-full bg-vb-surface-2 p-1.5 text-vb-silver-faint transition-colors hover:text-vb-silver"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}

            <AnimatePresence mode="wait" initial={false}>
              {phase === "offer" ? (
                <motion.div
                  key="offer"
                  initial={{ opacity: 0, y: 8 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -8 }}
                  transition={{ duration: 0.2 }}
                  className="relative flex min-h-0 flex-col"
                >
                  <div className="flex flex-col items-center px-6 pt-7 text-center">
                    <PulseLogo />
                    <h2 className="mt-4 text-[18px] font-semibold tracking-[-0.01em] text-vb-fg">
                      Доступна версия {info.version}
                    </h2>
                    {current && (
                      <div className="mt-1.5 flex items-center gap-1.5 font-mono text-[12px] text-vb-silver-faint">
                        v{current}
                        <ArrowRight className="h-3 w-3" />
                        <span className="text-vb-emerald">v{info.version}</span>
                      </div>
                    )}
                  </div>

                  <div className="min-h-0 overflow-y-auto px-5 pb-1 pt-5">
                    <div className="mb-2 flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-[0.06em] text-vb-silver-faint">
                      <Sparkles className="h-3 w-3" />
                      Что нового
                    </div>
                    <ul className="space-y-1.5">
                      {(highlights ?? ["Исправления и улучшения стабильности"]).map((h, i) => (
                        <motion.li
                          key={h}
                          initial={{ opacity: 0, x: -6 }}
                          animate={{ opacity: 1, x: 0 }}
                          transition={{ delay: 0.12 + i * 0.05, duration: 0.25 }}
                          className="flex gap-2.5 rounded-lg bg-vb-surface-2/50 px-3 py-2 text-[13px] leading-snug text-vb-silver"
                        >
                          <Check className="mt-0.5 h-3.5 w-3.5 shrink-0 text-vb-emerald" />
                          {h}
                        </motion.li>
                      ))}
                    </ul>
                  </div>

                  <div className="flex items-center justify-between gap-3 px-5 pb-5 pt-4">
                    <span className="flex items-center gap-1.5 text-[11px] text-vb-silver-faint">
                      <ShieldCheck className="h-3.5 w-3.5" />
                      Проверка по SHA-256
                    </span>
                    <div className="flex gap-2">
                      <Button variant="ghost" size="sm" onClick={close}>
                        Позже
                      </Button>
                      <Button variant="primary" size="sm" onClick={install}>
                        <Download className="h-3.5 w-3.5" />
                        Обновить
                      </Button>
                    </div>
                  </div>
                </motion.div>
              ) : (
                <motion.div
                  key="progress"
                  initial={{ opacity: 0, y: 8 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -8 }}
                  transition={{ duration: 0.2 }}
                  className="relative flex flex-col items-center px-6 pb-6 pt-8"
                >
                  <ProgressRing progress={progress} failed={phase === "error"} />
                  <h2 className="mt-5 text-[16px] font-semibold text-vb-fg">
                    {phase === "error" ? "Не получилось обновить" : `Обновляю до ${info.version}`}
                  </h2>
                  <p className="mt-1 min-h-[18px] text-center text-[12px] text-vb-silver-faint">
                    {phase === "error"
                      ? "Текущая версия продолжает работать"
                      : stageHint(progress)}
                  </p>

                  {phase === "error" ? (
                    <>
                      <p className="mt-4 w-full rounded-lg border border-vb-loss/30 bg-vb-loss/10 px-3 py-2 text-[12px] leading-snug text-vb-loss">
                        {error}
                      </p>
                      <div className="mt-5 flex w-full justify-end gap-2">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => import("@tauri-apps/plugin-opener").then((m) => m.openUrl(RELEASES))}
                        >
                          Скачать вручную
                        </Button>
                        <Button variant="secondary" size="sm" onClick={install}>
                          <RotateCw className="h-3.5 w-3.5" />
                          Повторить
                        </Button>
                      </div>
                    </>
                  ) : (
                    <Steps stage={progress?.stage ?? "download"} progress={progress} />
                  )}
                </motion.div>
              )}
            </AnimatePresence>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

function stageHint(p: UpdateProgress | null): string {
  switch (p?.stage) {
    case "verify":
      return "Сверяю контрольную сумму — файл не подменён";
    case "install":
      return "Ставлю новую версию на место старой";
    case "restart":
      return "Clod Helper откроется заново через пару секунд. Мост не прерывается";
    default:
      return p && p.total > 0
        ? `${formatMb(p.received)} из ${formatMb(p.total)}`
        : p && p.received > 0
          ? formatMb(p.received)
          : "Подключаюсь к GitHub…";
  }
}

/** Логотип с расходящимися волнами — «есть что-то новое». */
function PulseLogo() {
  return (
    <div className="relative grid h-16 w-16 place-items-center">
      {[0, 1].map((i) => (
        <motion.span
          key={i}
          aria-hidden
          className="absolute inset-0 rounded-[18px] border border-vb-emerald/40"
          initial={{ opacity: 0.6, scale: 1 }}
          animate={{ opacity: 0, scale: 1.55 }}
          transition={{ duration: 2.2, repeat: Infinity, delay: i * 1.1, ease: "easeOut" }}
        />
      ))}
      <LogoTile size={64} />
    </div>
  );
}

/** C-монограмма (как BrandMark, но без статуса — не нужен StatusProvider). */
export function LogoTile({ size }: { size: number }) {
  return (
    <svg viewBox="0 0 32 32" width={size} height={size} aria-hidden className="relative">
      <rect width="32" height="32" rx="7.5" fill="oklch(21% 0.012 160)" />
      <rect x="0.5" y="0.5" width="31" height="31" rx="7" fill="none" stroke="oklch(100% 0 0 / 0.08)" />
      <path
        d="M 23.09 21.54 A 9 9 0 1 1 23.09 10.46"
        fill="none"
        stroke="var(--color-vb-emerald)"
        strokeWidth="3.1"
        strokeLinecap="round"
      />
      <circle cx="25.2" cy="16" r="2.3" fill="var(--color-vb-emerald-bright)" />
    </svg>
  );
}

/*
  Кольцо прогресса: при скачивании — точная доля (если размер известен),
  на остальных шагах — бегущая дуга. Внутри — проценты или значок шага.
*/
function ProgressRing({ progress, failed }: { progress: UpdateProgress | null; failed: boolean }) {
  const R = 52;
  const C = 2 * Math.PI * R;
  const stage = progress?.stage ?? "download";
  const determinate = stage === "download" && !!progress && progress.total > 0;
  const frac = determinate ? Math.min(1, progress!.received / progress!.total) : 0;
  const pct = Math.round(frac * 100);

  return (
    <div className="relative grid h-[132px] w-[132px] place-items-center">
      {!failed && (
        <motion.div
          aria-hidden
          className="absolute inset-2 rounded-full"
          style={{ background: "radial-gradient(circle, var(--color-vb-emerald-glow), transparent 68%)" }}
          animate={{ opacity: [0.5, 1, 0.5], scale: [0.95, 1.05, 0.95] }}
          transition={{ duration: 2.4, repeat: Infinity, ease: "easeInOut" }}
        />
      )}
      <svg viewBox="0 0 120 120" className="absolute inset-0 h-full w-full -rotate-90">
        <defs>
          <linearGradient id="upd-ring" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--color-vb-emerald-bright)" />
            <stop offset="100%" stopColor="var(--color-vb-emerald-dim)" />
          </linearGradient>
        </defs>
        <circle cx="60" cy="60" r={R} fill="none" stroke="var(--color-vb-border)" strokeWidth="5" />
        {failed ? (
          <circle cx="60" cy="60" r={R} fill="none" stroke="var(--color-vb-loss)" strokeWidth="5" opacity="0.7" />
        ) : determinate ? (
          <motion.circle
            cx="60"
            cy="60"
            r={R}
            fill="none"
            stroke="url(#upd-ring)"
            strokeWidth="5"
            strokeLinecap="round"
            strokeDasharray={C}
            initial={false}
            animate={{ strokeDashoffset: C * (1 - frac) }}
            transition={{ duration: 0.25, ease: "easeOut" }}
          />
        ) : (
          <motion.circle
            cx="60"
            cy="60"
            r={R}
            fill="none"
            stroke="url(#upd-ring)"
            strokeWidth="5"
            strokeLinecap="round"
            strokeDasharray={`${C * 0.28} ${C}`}
            style={{ originX: "50%", originY: "50%" }}
            animate={{ rotate: 360 }}
            transition={{ duration: 1.1, repeat: Infinity, ease: "linear" }}
          />
        )}
      </svg>
      <AnimatePresence mode="wait">
        <motion.div
          key={failed ? "fail" : determinate ? "pct" : stage}
          initial={{ opacity: 0, scale: 0.8 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.8 }}
          transition={{ duration: 0.18 }}
          className="relative grid place-items-center"
        >
          {failed ? (
            <X className="h-8 w-8 text-vb-loss" />
          ) : determinate ? (
            <span className="tnum text-[26px] font-semibold tracking-[-0.02em] text-vb-fg">
              {pct}
              <span className="text-[14px] text-vb-silver-faint">%</span>
            </span>
          ) : stage === "verify" ? (
            <ShieldCheck className="h-8 w-8 text-vb-emerald" />
          ) : stage === "restart" ? (
            <RotateCw className="h-8 w-8 animate-spin text-vb-emerald [animation-duration:1.6s]" />
          ) : stage === "install" ? (
            <LogoTile size={40} />
          ) : (
            <Download className="h-8 w-8 text-vb-emerald" />
          )}
        </motion.div>
      </AnimatePresence>
    </div>
  );
}

function Steps({ stage, progress }: { stage: UpdateProgress["stage"]; progress: UpdateProgress | null }) {
  const at = STEPS.findIndex((s) => s.id === stage);
  return (
    <ol className="mt-5 w-full space-y-1 rounded-xl bg-vb-surface-2/50 p-2">
      {STEPS.map((s, i) => {
        const done = i < at;
        const active = i === at;
        return (
          <li
            key={s.id}
            className={cn(
              "flex items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-[13px] transition-colors duration-300",
              active ? "bg-vb-surface text-vb-fg" : done ? "text-vb-silver-dim" : "text-vb-silver-faint",
            )}
          >
            <span className="grid h-4 w-4 shrink-0 place-items-center">
              {done ? (
                <motion.span initial={{ scale: 0 }} animate={{ scale: 1 }} transition={{ type: "spring", bounce: 0.5 }}>
                  <Check className="h-4 w-4 text-vb-emerald" />
                </motion.span>
              ) : active ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin text-vb-emerald" />
              ) : (
                <span className="h-1.5 w-1.5 rounded-full bg-vb-border-strong" />
              )}
            </span>
            <span className="flex-1">{s.title}</span>
            {s.id === "download" && progress && progress.received > 0 && (
              <span className="tnum font-mono text-[11px] text-vb-silver-faint">
                {formatMb(progress.received)}
              </span>
            )}
          </li>
        );
      })}
    </ol>
  );
}
