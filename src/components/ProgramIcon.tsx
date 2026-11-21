import { useEffect, useState } from "react";
import { appsIcons } from "../lib/api";
import { cn } from "../lib/cn";

/*
  Маленькая иконка программы перед её названием (браузеры на «Сайтах»).
  Берётся настоящая иконка установленной программы - та же, что в «Пуске»
  или в Dock. Размер - с заглавную букву строки, квадрат со скруглением и
  лёгкой тенью снизу, чтобы иконка не выглядела наклейкой. Пока иконки нет
  (или её не извлечь) - плитка с первой буквой того же размера.
*/

const cache = new Map<string, string | null>();
const waiting = new Map<string, Promise<void>>();

function load(paths: string[]): Promise<void> {
  const need = paths.filter((p) => !cache.has(p) && !waiting.has(p));
  if (need.length === 0) return Promise.all(paths.map((p) => waiting.get(p))).then(() => undefined);
  const job = appsIcons(need)
    .then((found) => {
      for (const p of need) cache.set(p, found[p] ?? null);
    })
    .catch(() => {
      for (const p of need) cache.set(p, null);
    })
    .finally(() => {
      for (const p of need) waiting.delete(p);
    });
  for (const p of need) waiting.set(p, job);
  return job;
}

/** Иконки для списка путей: путь → data-URL (null - нет иконки). */
export function useProgramIcons(paths: (string | undefined)[]): Record<string, string | null> {
  const list = [...new Set(paths.filter((p): p is string => !!p))];
  const key = list.join("\n");
  const [, setTick] = useState(0);
  useEffect(() => {
    if (list.length === 0) return;
    let live = true;
    void load(list).then(() => live && setTick((t) => t + 1));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return Object.fromEntries(list.map((p) => [p, cache.get(p) ?? null]));
}

export function ProgramIcon({ name, icon, size = 16, className }: { name: string; icon?: string | null; size?: number; className?: string }) {
  return (
    <span
      aria-hidden
      style={{ width: size, height: size }}
      className={cn(
        "inline-flex shrink-0 items-center justify-center overflow-hidden rounded-[28%]",
        "shadow-[0_1px_1.5px_rgba(0,0,0,0.45),inset_0_0_0_0.5px_rgba(255,255,255,0.08)]",
        icon ? "bg-transparent" : "bg-gradient-to-b from-white/[0.16] to-white/[0.06] text-vb-silver",
        className,
      )}
    >
      {icon ? (
        <img src={icon} alt="" draggable={false} className="h-full w-full select-none object-contain" />
      ) : (
        <span className="font-semibold leading-none" style={{ fontSize: size * 0.58 }}>
          {name.slice(0, 1).toUpperCase()}
        </span>
      )}
    </span>
  );
}
