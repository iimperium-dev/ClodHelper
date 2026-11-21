import { motion, useReducedMotion } from "framer-motion";
import { cn } from "../lib/cn";

interface Props {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  /** Цвет включённого состояния: emerald (по умолчанию) или loss (для killswitch-опасности). */
  tone?: "emerald" | "loss";
  label?: string;
  id?: string;
  /** id элемента с пояснением (aria-describedby). */
  describedBy?: string;
  /** Размер: md (по умолчанию) или компактный sm. */
  size?: "md" | "sm";
}

/*
  Переключатель. Дизайн-правила:
  - spring на движение ручки (bounce:0), 1:1 с состоянием
  - focus-visible ring, role=switch + aria-checked
  - press-feedback на всём контроле
  - touch-таргет ≥ высоты 24px, ширина 44px
*/
export function Toggle({
  checked,
  onChange,
  disabled = false,
  tone = "emerald",
  label,
  id,
  describedBy,
  size = "md",
}: Props) {
  const reduce = useReducedMotion();
  const onColor = tone === "loss" ? "bg-vb-loss" : "bg-vb-emerald";

  return (
    <button
      type="button"
      role="switch"
      id={id}
      aria-checked={checked}
      aria-label={label}
      aria-describedby={describedBy}
      disabled={disabled}
      onClick={() => !disabled && onChange(!checked)}
      className={cn(
        "relative inline-flex shrink-0 items-center rounded-full p-0.5",
        size === "sm" ? "h-5 w-9" : "h-6 w-11",
        "transition-colors duration-200 ease-[cubic-bezier(0.22,1,0.36,1)]",
        "active:scale-[0.97] disabled:opacity-40 disabled:active:scale-100",
        checked ? onColor : "bg-vb-border-strong",
      )}
    >
      <motion.span
        className={cn(
          "block rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.4)]",
          size === "sm" ? "h-4 w-4" : "h-5 w-5",
        )}
        animate={{ x: checked ? (size === "sm" ? 16 : 20) : 0 }}
        transition={
          reduce
            ? { duration: 0 }
            : { type: "spring", bounce: 0, duration: 0.28 }
        }
      />
    </button>
  );
}
