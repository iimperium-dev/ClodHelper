import { motion, useReducedMotion } from "framer-motion";
import { cn } from "../lib/cn";

export type OrbState = "loading" | "ok" | "warn" | "down";

interface Props {
  state: OrbState;
  /** Период мягкого жёлтого мигания зелёного орба, с (часть прокси с проблемой). */
  pulse?: number | null;
  /** Диаметр ядра, px (кольцо и свечение масштабируются от него). */
  size?: number;
  className?: string;
}

/*
  Статус-орб: ядро + тонкое кольцо + мягкое «дыхание» свечения (только ok).
  Правила скиллов: idle-анимация тонкая и медленная (не мигание), reduced
  motion - статичный вариант, никаких bounce.
*/
export function StatusOrb({ state, pulse, size = 14, className }: Props) {
  const reduce = useReducedMotion();

  const core =
    state === "ok"
      ? "bg-vb-emerald"
      : state === "warn"
        ? "bg-vb-warn"
        : state === "down"
          ? "bg-vb-loss"
          : "bg-vb-silver-faint";

  const ring =
    state === "ok"
      ? "border-vb-emerald/35"
      : state === "warn"
        ? "border-vb-warn/35"
        : state === "down"
          ? "border-vb-loss/35"
          : "border-vb-border-strong";

  const glow =
    state === "ok"
      ? "0 0 18px var(--color-vb-emerald-glow)"
      : state === "warn"
        ? "0 0 14px oklch(76% 0.15 75 / 0.3)"
        : state === "down"
          ? "0 0 14px oklch(68% 0.19 22 / 0.3)"
          : "none";

  return (
    <span
      className={cn("relative inline-flex items-center justify-center", className)}
      style={{ width: size * 2.1, height: size * 2.1 }}
    >
      {/* Кольцо */}
      <span
        className={cn("absolute inset-0 rounded-full border", ring)}
        aria-hidden
      />
      {/* Дыхание - только для активного состояния */}
      {state === "ok" && !reduce && (
        <motion.span
          className="absolute inset-0 rounded-full border border-vb-emerald/25"
          animate={{ scale: [1, 1.28, 1], opacity: [0.7, 0, 0.7] }}
          transition={{ duration: 3.2, repeat: Infinity, ease: "easeInOut" }}
          aria-hidden
        />
      )}
      {/* Пульс загрузки */}
      {state === "loading" && !reduce && (
        <motion.span
          className="absolute inset-0 rounded-full border border-vb-border-strong"
          animate={{ opacity: [0.3, 0.9, 0.3] }}
          transition={{ duration: 1.4, repeat: Infinity, ease: "easeInOut" }}
          aria-hidden
        />
      )}
      {/* Ядро */}
      <span
        className={cn("relative rounded-full", core)}
        style={{ width: size, height: size, boxShadow: glow }}
      >
        <WarnPulse on={state === "ok" && !!pulse} period={pulse ?? 10} reduce={!!reduce} />
      </span>
    </span>
  );
}

/**
 * Мягкое жёлтое мигание поверх зелёного: «есть проблема с одним из прокси,
 * но всё работает». Короткая вспышка раз в period секунд (CSS-анимация,
 * index.css). Без анимаций (reduced motion) - тонкий жёлтый оттенок.
 */
export function WarnPulse({ on, period, reduce }: { on: boolean; period: number; reduce: boolean }) {
  if (!on) return null;
  if (reduce) {
    return <span className="absolute inset-0 rounded-full bg-vb-warn/45" aria-hidden />;
  }
  return (
    <span
      className={cn("absolute inset-0 rounded-full bg-vb-warn opacity-0", period <= 5 ? "warn-pulse-5" : "warn-pulse-10")}
      aria-hidden
    />
  );
}
