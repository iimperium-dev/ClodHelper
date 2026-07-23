import { useId } from "react";
import type { TrafficSample } from "../lib/status";

interface Props {
  data: TrafficSample[];
  width?: number;
  height?: number;
  className?: string;
}

/*
  Мини-график трафика (соединений за тик). SVG-area, изумруд с прозрачной
  заливкой. Сглаживания нет — честные данные, линии по точкам.
*/
export function Sparkline({ data, width = 160, height = 32, className }: Props) {
  const gradId = useId();
  if (data.length < 2) {
    return (
      <div
        className={className}
        style={{ width, height }}
        aria-hidden
      >
        <div className="flex h-full items-end gap-px opacity-30">
          {Array.from({ length: 20 }, (_, i) => (
            <div
              key={i}
              className="flex-1 rounded-t-sm bg-vb-border-strong"
              style={{ height: 3 }}
            />
          ))}
        </div>
      </div>
    );
  }

  const max = Math.max(1, ...data.map((d) => d.delta));
  const stepX = width / (data.length - 1);
  const pad = 2;
  const usable = height - pad * 2;

  const points = data.map((d, i) => {
    const x = i * stepX;
    const y = pad + usable * (1 - d.delta / max);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });

  const line = points.join(" ");
  const area = `0,${height} ${line} ${width},${height}`;

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      className={className}
      role="img"
      aria-label="График трафика"
    >
      <defs>
        <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--color-vb-emerald)" stopOpacity="0.25" />
          <stop offset="100%" stopColor="var(--color-vb-emerald)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <polygon points={area} fill={`url(#${gradId})`} />
      <polyline
        points={line}
        fill="none"
        stroke="var(--color-vb-emerald)"
        strokeWidth="1.5"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}
