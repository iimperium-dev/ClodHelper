import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { Check } from "lucide-react";
import { cn } from "../lib/cn";

export interface MenuItem {
  label: string;
  run: () => void;
  danger?: boolean;
  checked?: boolean;
  /** Подзаголовок группы (не кликается). */
  header?: boolean;
  divider?: boolean;
}

interface Props {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}

/*
  Контекстное меню по правому клику — в духе системного меню macOS:
  появляется из точки клика (scale + fade), не выходит за край окна,
  закрывается кликом мимо, Esc, прокруткой.
*/
export function ContextMenu({ x, y, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ x, y });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.min(x, window.innerWidth - r.width - 8),
      y: Math.min(y, window.innerHeight - r.height - 8),
    });
  }, [x, y]);

  useEffect(() => {
    const close = (e: Event) => {
      if (e instanceof MouseEvent && ref.current?.contains(e.target as Node)) return;
      onClose();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("wheel", close, { passive: true });
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("wheel", close);
      window.removeEventListener("keydown", esc);
    };
  }, [onClose]);

  return (
    <motion.div
      ref={ref}
      initial={{ opacity: 0, scale: 0.92 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.14, ease: [0.22, 1, 0.36, 1] }}
      style={{ left: pos.x, top: pos.y, transformOrigin: "top left" }}
      className="fixed z-[60] min-w-[200px] overflow-hidden rounded-xl border border-vb-border bg-vb-surface/95 p-1 shadow-[0_12px_32px_rgba(0,0,0,0.55)] backdrop-blur-xl"
      role="menu"
    >
      {items.map((it, i) =>
        it.divider ? (
          <div key={`d${i}`} className="my-1 h-px bg-vb-border/70" />
        ) : it.header ? (
          <div key={`h${i}`} className="px-2.5 pb-0.5 pt-1.5 text-[11px] text-vb-silver-faint">
            {it.label}
          </div>
        ) : (
          <button
            key={`${it.label}${i}`}
            type="button"
            role="menuitem"
            onClick={() => {
              onClose();
              it.run();
            }}
            className={cn(
              "flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors",
              it.danger ? "text-vb-loss hover:bg-vb-loss/10" : "text-vb-silver hover:bg-vb-emerald/15 hover:text-vb-fg",
            )}
          >
            <span className="w-3.5 shrink-0">{it.checked && <Check className="h-3.5 w-3.5" />}</span>
            {it.label}
          </button>
        ),
      )}
    </motion.div>
  );
}
