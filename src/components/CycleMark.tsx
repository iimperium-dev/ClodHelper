import { motion } from "framer-motion";
import { LogoTile } from "./LogoTile";
import { cn } from "../lib/cn";

/*
  Знак: логотип внутри кольца-цикла. Стрелка кольца делает один оборот при
  появлении (не в цикле); на «Включить» кольцо замыкается в зелёный круг.
*/
export function CycleMark({ accepted, reduce, compact }: { accepted: boolean; reduce: boolean; compact: boolean }) {
  const R = 34;
  const C = 2 * Math.PI * R;
  return (
    <div className={cn("relative mx-auto grid place-items-center", compact ? "h-14 w-14" : "h-[84px] w-[84px]")}>
      <motion.svg
        viewBox="0 0 84 84"
        className="absolute inset-0"
        aria-hidden
        initial={reduce ? false : { rotate: -120 }}
        animate={{ rotate: 0 }}
        transition={{ duration: 0.9, ease: [0.22, 1, 0.36, 1] }}
      >
        <circle cx="42" cy="42" r={R} fill="none" stroke="var(--color-vb-border)" strokeWidth="2" />
        <motion.circle
          cx="42"
          cy="42"
          r={R}
          fill="none"
          stroke="var(--color-vb-emerald)"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeDasharray={C}
          initial={false}
          animate={{ strokeDashoffset: accepted ? 0 : C * 0.3 }}
          transition={{ duration: reduce ? 0 : 0.45, ease: [0.22, 1, 0.36, 1] }}
          transform="rotate(-90 42 42)"
        />
        {/* Наконечник стрелки на конце дуги (в покое). */}
        {!accepted && (
          <path
            d="M 9.6 29.5 L 8.6 35.6 L 14.4 33.6"
            fill="none"
            stroke="var(--color-vb-emerald)"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        )}
      </motion.svg>
      <LogoTile size={compact ? 32 : 48} />
    </div>
  );
}
