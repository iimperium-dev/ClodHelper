import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Check } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogoTile } from "../components/UpdateDialog";

type State = "installing" | "starting" | "started" | "failed";

const TEXT: Record<State, string> = {
  installing: "Устанавливаю новую версию…",
  starting: "Запускаю новую версию…",
  started: "Готово — открываю",
  failed: "Новая версия не запустилась. Откройте Clod Helper из меню «Пуск»",
};

/*
  Заставка на время установки обновления (Windows, отдельный процесс —
  update_splash.rs): вместо белого окна установщика. Логотип в бегущем
  кольце, шиммер-полоска, статус. Уходит сама, когда новая версия стартовала.
*/
export function UpdateSplash() {
  const version = new URLSearchParams(window.location.search).get("v") ?? "";
  const [state, setState] = useState<State>("installing");

  useEffect(() => {
    const un = listen<State>("splash:state", (e) => setState(e.payload));
    return () => void un.then((f) => f());
  }, []);

  const done = state === "started";

  return (
    <div className="flex h-screen w-screen items-center justify-center p-4">
      {/* На «started» карточка сама тает (animate) — процесс закроется следом. */}
      <motion.div
            data-tauri-drag-region
            initial={{ opacity: 0, scale: 0.94, y: 10 }}
            animate={done ? { opacity: 0, scale: 0.97, y: -6 } : { opacity: 1, scale: 1, y: 0 }}
            transition={done ? { delay: 0.7, duration: 0.6 } : { type: "spring", bounce: 0.2, duration: 0.5 }}
            className="relative flex w-full flex-col items-center overflow-hidden rounded-2xl border border-vb-border bg-vb-surface px-6 pb-6 pt-7 shadow-[0_18px_48px_rgba(0,0,0,0.6)]"
          >
            <div
              aria-hidden
              className="pointer-events-none absolute inset-x-0 -top-20 h-40"
              style={{ background: "radial-gradient(60% 100% at 50% 0%, var(--color-vb-emerald-glow), transparent 70%)" }}
            />
            <div className="relative grid h-[84px] w-[84px] place-items-center">
              <svg viewBox="0 0 84 84" className="absolute inset-0">
                <circle cx="42" cy="42" r="39" fill="none" stroke="var(--color-vb-border)" strokeWidth="3" />
              </svg>
              {state !== "failed" && (
                <motion.svg
                  viewBox="0 0 84 84"
                  className="absolute inset-0"
                  animate={done ? { rotate: 0 } : { rotate: 360 }}
                  transition={done ? { duration: 0.3 } : { duration: 1.2, repeat: Infinity, ease: "linear" }}
                >
                  <circle
                    cx="42"
                    cy="42"
                    r="39"
                    fill="none"
                    stroke="var(--color-vb-emerald)"
                    strokeWidth="3"
                    strokeLinecap="round"
                    strokeDasharray={done ? "245 245" : "60 245"}
                    style={{ transition: "stroke-dasharray 0.5s cubic-bezier(0.22,1,0.36,1)" }}
                  />
                </motion.svg>
              )}
              <AnimatePresence mode="wait">
                {done ? (
                  <motion.span
                    key="ok"
                    initial={{ scale: 0 }}
                    animate={{ scale: 1 }}
                    transition={{ type: "spring", bounce: 0.5 }}
                    className="grid h-12 w-12 place-items-center rounded-full bg-vb-emerald/15"
                  >
                    <Check className="h-7 w-7 text-vb-emerald" />
                  </motion.span>
                ) : (
                  <motion.span key="logo" exit={{ scale: 0.6, opacity: 0 }}>
                    <LogoTile size={52} />
                  </motion.span>
                )}
              </AnimatePresence>
            </div>

            <div className="relative mt-4 text-[16px] font-semibold text-vb-fg">
              Обновляю Clod Helper{version ? ` до ${version}` : ""}
            </div>
            <AnimatePresence mode="wait">
              <motion.div
                key={state}
                initial={{ opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.2 }}
                className={`relative mt-1 text-center text-[12px] leading-snug ${state === "failed" ? "text-vb-warn" : "text-vb-silver-faint"}`}
              >
                {TEXT[state]}
              </motion.div>
            </AnimatePresence>

            {state === "failed" ? (
              <button
                type="button"
                onClick={() => void invoke("splash_close")}
                className="relative mt-4 rounded-lg border border-vb-border bg-vb-surface-2 px-4 py-1.5 text-[13px] text-vb-silver transition-colors hover:border-vb-border-strong"
              >
                Закрыть
              </button>
            ) : (
              <div className="relative mt-5 h-1 w-full overflow-hidden rounded-full bg-vb-surface-2">
                <motion.div
                  className="absolute inset-y-0 w-1/3 rounded-full"
                  style={{ background: "linear-gradient(90deg, transparent, var(--color-vb-emerald), transparent)" }}
                  animate={done ? { left: "0%", width: "100%" } : { left: ["-35%", "100%"] }}
                  transition={done ? { duration: 0.4 } : { duration: 1.4, repeat: Infinity, ease: "easeInOut" }}
                />
              </div>
            )}
            <div className="relative mt-3 text-[11px] text-vb-silver-faint">Мост и прокси работают без перерыва</div>
      </motion.div>
    </div>
  );
}
