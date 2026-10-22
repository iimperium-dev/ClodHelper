import { useEffect, useMemo } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Check } from "lucide-react";
import { Button } from "./Button";
import { LogoTile } from "./UpdateDialog";
import { playChime } from "../lib/api";
import { whatsNewItems } from "../lib/updateBus";

/*
  «Что нового» — первый запуск после обновления. Логотип с вспышкой искр,
  версия и главные изменения из src/whats-new.json (тот же файл CI кладёт
  в описание релиза). Звук — тот же «динь», что у уведомления.
*/
export function WhatsNewModal({ version, onClose }: { version: string | null; onClose: () => void }) {
  const items = version ? whatsNewItems(version) : [];

  useEffect(() => {
    if (!version) return;
    void playChime().catch(() => {});
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [version, onClose]);

  return createPortal(
    <AnimatePresence>
      {version && (
        <motion.div
          key="whats-new"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          onMouseDown={onClose}
          className="fixed inset-x-0 bottom-0 top-9 z-[85] flex items-center justify-center bg-black/50 p-5 backdrop-blur-[6px]"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.9, y: 18 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 8 }}
            transition={{ type: "spring", bounce: 0.28, duration: 0.55 }}
            onMouseDown={(e) => e.stopPropagation()}
            role="dialog"
            aria-label="Что нового"
            className="relative flex max-h-full w-full max-w-[400px] flex-col overflow-hidden rounded-2xl border border-vb-border bg-vb-surface shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
          >
            <div
              aria-hidden
              className="pointer-events-none absolute inset-x-0 -top-28 h-56"
              style={{
                background: "radial-gradient(60% 100% at 50% 0%, var(--color-vb-emerald-glow), transparent 70%)",
              }}
            />
            <div className="relative flex flex-col items-center px-6 pt-8 text-center">
              <Burst />
              <motion.div
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.25, duration: 0.3 }}
              >
                <div className="mt-4 text-[18px] font-semibold tracking-[-0.01em] text-vb-fg">
                  Clod Helper обновлён
                </div>
                <span className="mt-2 inline-block rounded-full border border-vb-emerald/40 bg-vb-emerald/10 px-2.5 py-0.5 font-mono text-[12px] text-vb-emerald">
                  v{version}
                </span>
              </motion.div>
            </div>

            <div className="relative min-h-0 overflow-y-auto px-5 pt-5">
              <div className="mb-2 text-[11px] font-medium uppercase tracking-[0.06em] text-vb-silver-faint">
                Что нового
              </div>
              <ul className="space-y-1.5">
                {(items.length ? items : ["Исправления и улучшения стабильности"]).map((t, i) => (
                  <motion.li
                    key={t}
                    initial={{ opacity: 0, x: -8 }}
                    animate={{ opacity: 1, x: 0 }}
                    transition={{ delay: 0.35 + i * 0.07, duration: 0.28 }}
                    className="flex gap-2.5 rounded-lg bg-vb-surface-2/50 px-3 py-2 text-[13px] leading-snug text-vb-silver"
                  >
                    <Check className="mt-0.5 h-3.5 w-3.5 shrink-0 text-vb-emerald" />
                    {t}
                  </motion.li>
                ))}
              </ul>
            </div>

            <div className="relative flex justify-end px-5 pb-5 pt-5">
              <Button variant="primary" size="sm" onClick={onClose}>
                Отлично
              </Button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

/** Логотип и разлетающиеся от него искры — один раз при открытии. */
function Burst() {
  const sparks = useMemo(
    () =>
      Array.from({ length: 14 }, (_, i) => {
        const a = (i / 14) * Math.PI * 2 + (i % 2 ? 0.2 : 0);
        const d = 46 + (i % 3) * 14;
        return { x: Math.cos(a) * d, y: Math.sin(a) * d, s: 3 + (i % 3), bright: i % 2 === 0 };
      }),
    [],
  );
  return (
    <div className="relative grid h-16 w-16 place-items-center">
      {sparks.map((p, i) => (
        <motion.span
          key={i}
          aria-hidden
          className="absolute rounded-full"
          style={{
            width: p.s,
            height: p.s,
            background: p.bright ? "var(--color-vb-emerald-bright)" : "var(--color-vb-emerald)",
          }}
          initial={{ x: 0, y: 0, opacity: 0, scale: 0.4 }}
          animate={{ x: p.x, y: p.y, opacity: [0, 1, 0], scale: [0.4, 1, 0.6] }}
          transition={{ delay: 0.15, duration: 0.9, ease: [0.22, 1, 0.36, 1] }}
        />
      ))}
      <motion.div
        initial={{ scale: 0.6, rotate: -12 }}
        animate={{ scale: 1, rotate: 0 }}
        transition={{ type: "spring", bounce: 0.5, duration: 0.7 }}
      >
        <LogoTile size={64} />
      </motion.div>
    </div>
  );
}
