import { type ButtonHTMLAttributes, forwardRef } from "react";
import { cn } from "../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md";

interface Props extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
}

/*
  Кнопка по дизайн-правилам:
  - press-feedback scale(0.97) на :active, 120ms ease-out-quint
  - только transform+opacity в переходах
  - focus-visible ring наследуется из base-стилей (index.css)
  - акцент (emerald) редок — variant=primary только для одного CTA на экран
*/
export const Button = forwardRef<HTMLButtonElement, Props>(function Button(
  { variant = "primary", size = "md", className, children, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      {...rest}
      className={cn(
        "inline-flex items-center justify-center gap-2 rounded-lg font-medium",
        "transition-[transform,background-color,border-color,filter,opacity] duration-[160ms] ease-[cubic-bezier(0.22,1,0.36,1)]",
        "active:scale-[0.97] disabled:cursor-not-allowed disabled:opacity-40 disabled:active:scale-100",
        size === "sm" && "px-3 py-1.5 text-[13px]",
        size === "md" && "px-4 py-2.5 text-[14px]",
        variant === "primary" &&
          "bg-vb-emerald text-black hover:bg-vb-emerald-bright shadow-[0_2px_12px_var(--color-vb-emerald-glow)]",
        variant === "secondary" &&
          "bg-vb-surface-2 text-vb-silver border border-vb-border hover:border-vb-border-strong hover:bg-vb-surface-2/80",
        variant === "ghost" &&
          "text-vb-silver-dim hover:text-vb-silver hover:bg-vb-surface-2/60",
        variant === "danger" &&
          "bg-vb-loss/10 text-vb-loss border border-vb-loss/30 hover:bg-vb-loss/15 hover:border-vb-loss/50",
        className,
      )}
    >
      {children}
    </button>
  );
});
