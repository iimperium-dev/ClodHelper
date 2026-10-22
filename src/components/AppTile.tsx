import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Minus, Settings } from "lucide-react";
import { appBrand, BrandTile } from "./BrandLogos";
import { cn } from "../lib/cn";
import type { RowStatus } from "./AppRow";

/** Иконка приложения: настоящая (macOS) → фирменная плитка → первая буква. */
export function AppIcon({
  name,
  icon,
  size = 32,
  dim,
}: {
  name: string;
  icon?: string;
  size?: number;
  dim?: boolean;
}) {
  if (icon) {
    return (
      <img
        src={icon}
        alt=""
        draggable={false}
        style={{ width: size, height: size }}
        className={cn("shrink-0 select-none", dim && "opacity-50 saturate-50")}
      />
    );
  }
  const brand = appBrand(name);
  if (brand) {
    return (
      <span style={{ width: size, height: size }} className="flex shrink-0 items-center justify-center">
        <span style={{ transform: `scale(${size / 32})` }}>
          <BrandTile glow={brand.glow} disabled={dim}>
            {brand.glyph}
          </BrandTile>
        </span>
      </span>
    );
  }
  return (
    <span
      style={{ width: size, height: size, fontSize: size * 0.42 }}
      className={cn(
        "flex shrink-0 items-center justify-center rounded-[22%] bg-vb-surface-2 font-semibold text-vb-silver",
        dim && "opacity-50",
      )}
    >
      {name.slice(0, 1).toUpperCase()}
    </span>
  );
}

/**
 * Иконка с «выезжающей шестерёнкой»: при наведении на приложение снизу на
 * логотип выезжает (с поворотом) половинка кружка-шестерёнки — клик по ней
 * открывает настройки приложения.
 */
export function IconWithGear({
  show,
  size,
  onSettings,
  name,
  children,
}: {
  show: boolean;
  size: number;
  /** Для подписи кнопки: «Настройки — Telegram». */
  name: string;
  onSettings?: () => void;
  children: React.ReactNode;
}) {
  // Крупнее, чем раньше (была 42% иконки — терялась); появляется коротко и
  // ровно, без пружины и вращения — те дёргали кнопку.
  const g = Math.round(Math.max(20, size * 0.56));
  return (
    <div className="relative shrink-0" style={{ width: size, height: size }}>
      {children}
      <AnimatePresence>
        {show && onSettings && (
          <motion.button
            type="button"
            initial={{ opacity: 0, y: 6, scale: 0.85 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 4, scale: 0.9 }}
            transition={{ duration: 0.16, ease: [0.22, 1, 0.36, 1] }}
            onPointerDown={(e) => e.stopPropagation()}
            onPointerUp={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              onSettings();
            }}
            aria-label={`Настройки — ${name}`}
            title="Настройки"
            style={{ width: g, height: g, left: (size - g) / 2, bottom: -g / 2 }}
            className="group/gear absolute z-10 flex items-center justify-center rounded-full border border-vb-border-strong bg-vb-surface text-vb-fg shadow-[0_4px_10px_rgba(0,0,0,0.45)] transition-colors hover:border-vb-emerald/70 hover:bg-vb-emerald/15 hover:text-vb-emerald"
          >
            <Settings
              style={{ width: g * 0.62, height: g * 0.62 }}
              strokeWidth={2}
              className="transition-transform duration-300 group-hover/gear:rotate-45"
            />
          </motion.button>
        )}
      </AnimatePresence>
    </div>
  );
}

interface Props {
  name: string;
  icon?: string;
  installed: boolean;
  status: RowStatus | null;
  editing: boolean;
  onRemove: () => void;
  onSettings?: () => void;
}

/*
  Плитка для вида «Сетка» — как иконка на домашнем экране iOS: крупная
  иконка, подпись, точка статуса в углу. В режиме упорядочивания — «−»
  для удаления из списка (покачивание задаёт обёртка).
*/
export function AppTile({ name, icon, installed, status, editing, onRemove, onSettings }: Props) {
  const [hover, setHover] = useState(false);
  const dot =
    !installed || !status || status.tone === "muted"
      ? null
      : status.tone === "ok"
        ? "bg-vb-emerald"
        : "bg-vb-warn";
  return (
    <div
      className="flex w-[84px] flex-col items-center gap-1.5 pt-2"
      title={status?.text}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      <div className="relative">
        <IconWithGear show={hover && !editing} size={52} onSettings={onSettings} name={name}>
          <AppIcon name={name} icon={icon} size={52} dim={!installed} />
        </IconWithGear>
        {dot && !editing && (
          <span
            className={cn(
              "absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-vb-bg",
              dot,
            )}
          />
        )}
        {editing && (
          <motion.button
            type="button"
            initial={{ scale: 0 }}
            animate={{ scale: 1 }}
            exit={{ scale: 0 }}
            transition={{ type: "spring", bounce: 0.4, duration: 0.3 }}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              onRemove();
            }}
            aria-label={`Убрать ${name}`}
            className="absolute -left-1.5 -top-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-vb-silver-dim text-vb-bg shadow"
          >
            <Minus className="h-3 w-3" strokeWidth={3} />
          </motion.button>
        )}
      </div>
      <span
        className={cn(
          "w-full truncate text-center text-[11px] leading-tight",
          installed ? "text-vb-silver" : "text-vb-silver-faint",
        )}
      >
        {name}
      </span>
    </div>
  );
}
