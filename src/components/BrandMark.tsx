import { orbStateOf, useStatus } from "../lib/status";
import { cn } from "../lib/cn";

interface Props {
  size?: number;
  /** Живая статус-точка в углу (как у иконки в трее). */
  withStatus?: boolean;
  className?: string;
}

/*
  Фирменный знак — C-монограмма из иконки приложения (изумрудная дуга с
  узлом-точкой на тёмной плитке), в SVG чтобы был чёткий на любом размере.
  Со статус-бейджем в углу: зелёный/жёлтый/красный — то же состояние, что
  у иконки в трее. Используется в titlebar и сайдбаре.
*/
export function BrandMark({ size = 32, withStatus = true, className }: Props) {
  const { status, health } = useStatus();
  const st = orbStateOf(status, health);
  const dotColor =
    st === "ok"
      ? "bg-vb-emerald"
      : st === "warn"
        ? "bg-vb-warn"
        : st === "down"
          ? "bg-vb-loss"
          : "bg-vb-silver-faint";

  return (
    <span
      className={cn("relative inline-flex shrink-0", className)}
      style={{ width: size, height: size }}
      aria-hidden
    >
      <svg viewBox="0 0 32 32" width={size} height={size}>
        <rect width="32" height="32" rx="7.5" fill="oklch(21% 0.012 160)" />
        <rect
          x="0.5"
          y="0.5"
          width="31"
          height="31"
          rx="7"
          fill="none"
          stroke="oklch(100% 0 0 / 0.06)"
        />
        {/* C-дуга: разрыв справа, в разрыве — узел-точка */}
        <path
          d="M 23.09 21.54 A 9 9 0 1 1 23.09 10.46"
          fill="none"
          stroke="var(--color-vb-emerald)"
          strokeWidth="3.1"
          strokeLinecap="round"
        />
        <circle cx="25.2" cy="16" r="2.3" fill="var(--color-vb-emerald-bright)" />
      </svg>
      {withStatus && (
        <span
          className={cn("absolute rounded-full border-2 border-vb-bg", dotColor)}
          style={{
            width: Math.max(8, size * 0.34),
            height: Math.max(8, size * 0.34),
            right: -size * 0.07,
            bottom: -size * 0.07,
          }}
        />
      )}
    </span>
  );
}
