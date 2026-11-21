import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { ArrowUpRight, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { noticeEvent, noticesBanner, noticesBannerDismiss } from "../lib/api";
import type { BannerView } from "../lib/types";
import { cn } from "../lib/cn";

/*
  Баннер на «Обзоре»: объявление из админки с оформлением. Что показывать,
  кому и когда - решается в админке (заголовок, текст, цвет, метка, до двух
  кнопок, можно ли закрыть). Программа только рисует: ссылки только https,
  картинок с чужих адресов нет. Закрытый крестиком не возвращается.
*/

/** Цвет акцента: точка, метка, свечение в углу и главная кнопка. */
const ACCENT: Record<BannerView["accent"], string> = {
  emerald: "var(--color-vb-emerald)",
  blue: "#6aa8ff",
  violet: "#a98bff",
  amber: "var(--color-vb-warn)",
  rose: "#ff7a95",
  neutral: "#d8dde3",
};

export function PromoBanner({ className }: { className?: string }) {
  const reduce = useReducedMotion();
  const [b, setB] = useState<BannerView | null>(null);
  const shown = useRef<string | null>(null);

  useEffect(() => {
    const load = () => void noticesBanner().then(setB).catch(() => setB(null));
    load();
    // Рассылки обновились (админка включила или сняла баннер) - перечитать.
    const un = listen("notices:changed", load);
    return () => void un.then((f) => f());
  }, []);

  useEffect(() => {
    if (b && shown.current !== b.id) {
      shown.current = b.id;
      void noticeEvent(b.id, "shown").catch(() => {});
    }
  }, [b]);

  const open = (url: string) => {
    if (!b) return;
    void noticeEvent(b.id, "clicked").catch(() => {});
    void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(url));
  };

  const dismiss = () => {
    if (!b) return;
    const id = b.id;
    setB(null);
    void noticeEvent(id, "dismissed").catch(() => {});
    void noticesBannerDismiss(id).catch(() => {});
  };

  const acc = b ? ACCENT[b.accent] ?? ACCENT.emerald : ACCENT.emerald;

  return (
    <AnimatePresence initial={false}>
      {b && (
        <motion.section
          key={b.id}
          aria-label={b.title}
          initial={reduce ? { opacity: 0 } : { opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.22, 1, 0.36, 1] } }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, y: 4, transition: { duration: 0.18, ease: "easeIn" } }}
          style={{ ["--acc" as string]: acc }}
          className={cn(
            "relative overflow-hidden rounded-2xl border border-vb-border bg-vb-surface p-4 pr-10",
            "shadow-[0_1px_0_rgba(255,255,255,0.03)_inset,0_10px_30px_rgba(0,0,0,0.35)]",
            className,
          )}
        >
          {/* Мягкое свечение цвета акцента в углу и тонкая линия сверху. */}
          <span
            aria-hidden
            className="pointer-events-none absolute -right-16 -top-20 h-44 w-44 rounded-full opacity-[0.14] blur-3xl"
            style={{ background: "var(--acc)" }}
          />
          <span
            aria-hidden
            className="pointer-events-none absolute inset-x-6 top-0 h-px opacity-50"
            style={{ background: "linear-gradient(90deg, transparent, var(--acc), transparent)" }}
          />

          {b.eyebrow && (
            <p className="relative mb-1 flex items-center gap-1.5 text-[11px] font-medium" style={{ color: "var(--acc)" }}>
              <span aria-hidden className="h-1 w-1 rounded-full" style={{ background: "var(--acc)" }} />
              {b.eyebrow}
            </p>
          )}
          <h2 className="relative text-[14px] font-semibold leading-snug tracking-[-0.01em] text-vb-fg">{b.title}</h2>
          {b.body && (
            <p className="relative mt-1 line-clamp-3 whitespace-pre-line text-[12px] leading-relaxed text-vb-silver-dim">{b.body}</p>
          )}

          {b.buttons.length > 0 && (
            <div className="relative mt-3 flex flex-wrap gap-2">
              {b.buttons.map((x, i) =>
                i === 0 ? (
                  <button
                    key={x.url + i}
                    type="button"
                    onClick={() => open(x.url)}
                    className="inline-flex h-8 items-center gap-1 rounded-lg px-3 text-[12px] font-semibold text-black transition-[filter,transform] hover:brightness-110 active:scale-[0.97]"
                    style={{ background: "var(--acc)" }}
                  >
                    {x.label}
                    <ArrowUpRight className="h-3.5 w-3.5" aria-hidden />
                  </button>
                ) : (
                  <button
                    key={x.url + i}
                    type="button"
                    onClick={() => open(x.url)}
                    className="inline-flex h-8 items-center rounded-lg border border-vb-border px-3 text-[12px] font-medium text-vb-silver transition-colors hover:bg-vb-surface-2 hover:text-vb-fg active:scale-[0.97]"
                  >
                    {x.label}
                  </button>
                ),
              )}
            </div>
          )}

          {b.dismissible && (
            <button
              type="button"
              onClick={dismiss}
              aria-label="Скрыть"
              title="Скрыть"
              className="absolute right-2.5 top-2.5 rounded-md p-1 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </motion.section>
      )}
    </AnimatePresence>
  );
}
