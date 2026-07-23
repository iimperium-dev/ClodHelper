import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  Check,
  ChevronDown,
  ExternalLink,
  Loader2,
  MonitorDown,
  Play,
  Trash2,
} from "lucide-react";
import { Toggle } from "./Toggle";
import { appBrand, BrandTile } from "./BrandLogos";
import { cn } from "../lib/cn";
import type { AppProfileStatus, ShortcutTarget } from "../lib/types";

interface Props {
  app: AppProfileStatus;
  onToggle: (enabled: boolean) => void;
  onLaunch: () => Promise<void>;
  onShortcut: (target: ShortcutTarget) => Promise<string>;
  onRemove?: () => void;
}

/*
  Строка приложения — плоская (строки в общем контейнере с hairline,
  не карточка на карточке): иконка, имя+тип, тихие действия, тумблер.
*/
export function AppRow({ app, onToggle, onLaunch, onShortcut, onRemove }: Props) {
  const [busy, setBusy] = useState<null | "launch" | "shortcut">(null);
  const [shortcutOpen, setShortcutOpen] = useState(false);
  const [flash, setFlash] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  // Light dismiss: клик вне меню ярлыка закрывает его.
  useEffect(() => {
    if (!shortcutOpen) return;
    const close = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setShortcutOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [shortcutOpen]);

  const launch = async () => {
    setBusy("launch");
    try {
      await onLaunch();
    } catch (e) {
      setFlash(String(e));
      window.setTimeout(() => setFlash(null), 3000);
    } finally {
      setBusy(null);
    }
  };

  const makeShortcut = async (target: ShortcutTarget) => {
    setBusy("shortcut");
    setShortcutOpen(false);
    try {
      await onShortcut(target);
      setFlash(
        target === "desktop"
          ? "Ярлык на рабочем столе"
          : "Ярлык в Пуске — правым кликом закрепите на панель",
      );
      window.setTimeout(() => setFlash(null), 4000);
    } catch (e) {
      setFlash(String(e));
      window.setTimeout(() => setFlash(null), 3000);
    } finally {
      setBusy(null);
    }
  };

  const kindLabel =
    app.kind === "msix"
      ? "Microsoft Store"
      : app.kind === "custom"
        ? "Своё приложение"
        : "Приложение";

  const brand = appBrand(app.name);

  return (
    <motion.div layout className="px-4 py-3">
      <div className="flex items-center gap-3">
        {/* Бренд-логотип приложения; generic-иконка — только для неизвестных */}
        {brand ? (
          <BrandTile glow={brand.glow} disabled={!app.enabled}>
            {brand.glyph}
          </BrandTile>
        ) : (
          <div
            className={cn(
              "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors duration-200",
              app.enabled
                ? "bg-vb-emerald/12 text-vb-emerald"
                : "bg-vb-surface-2 text-vb-silver-dim",
            )}
          >
            <MonitorDown className="h-[18px] w-[18px]" strokeWidth={1.75} />
          </div>
        )}

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-[14px] font-semibold text-vb-fg">
              {app.name}
            </span>
            {!app.installed && (
              <span className="rounded-md bg-vb-surface-2 px-1.5 py-0.5 text-[10px] font-medium text-vb-silver-faint">
                не найдено
              </span>
            )}
          </div>
          <div className="mt-px truncate text-[11px] text-vb-silver-faint">
            {kindLabel}
            {app.installed && app.desktop_shortcut && " · ярлык готов"}
          </div>
        </div>

        {/* Действия: тихие icon-кнопки */}
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={launch}
            disabled={!app.installed || busy !== null}
            className={cn(
              "flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px] font-medium text-vb-silver-dim",
              "transition-colors duration-150 hover:bg-vb-surface-2 hover:text-vb-silver",
              "active:scale-[0.97] disabled:cursor-not-allowed disabled:opacity-35",
            )}
            title="Запустить через мост"
          >
            {busy === "launch" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Play className="h-3.5 w-3.5" />
            )}
            Открыть
          </button>

          <div className="relative" ref={menuRef}>
            <button
              type="button"
              onClick={() => setShortcutOpen((v) => !v)}
              disabled={!app.installed || busy !== null}
              className={cn(
                "flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-[12px] font-medium text-vb-silver-dim",
                "transition-colors duration-150 hover:bg-vb-surface-2 hover:text-vb-silver",
                "active:scale-[0.97] disabled:cursor-not-allowed disabled:opacity-35",
              )}
              title="Создать ярлык"
            >
              {busy === "shortcut" ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <ExternalLink className="h-3.5 w-3.5" />
              )}
              <ChevronDown className="h-3 w-3 opacity-60" />
            </button>

            <AnimatePresence>
              {shortcutOpen && (
                <motion.div
                  initial={{ opacity: 0, scale: 0.96, y: -4 }}
                  animate={{ opacity: 1, scale: 1, y: 0 }}
                  exit={{ opacity: 0, scale: 0.98, y: -2 }}
                  transition={{ duration: 0.14, ease: [0.22, 1, 0.36, 1] }}
                  style={{ transformOrigin: "top right" }}
                  className="z-dropdown absolute right-0 top-full mt-1 w-44 overflow-hidden rounded-lg border border-vb-border bg-vb-surface shadow-[0_8px_24px_rgba(0,0,0,0.5)]"
                >
                  <button
                    type="button"
                    onClick={() => makeShortcut("desktop")}
                    className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-vb-silver transition-colors hover:bg-vb-surface-2"
                  >
                    На рабочий стол
                  </button>
                  <button
                    type="button"
                    onClick={() => makeShortcut("start_menu")}
                    className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-vb-silver transition-colors hover:bg-vb-surface-2"
                  >
                    В меню Пуск
                  </button>
                </motion.div>
              )}
            </AnimatePresence>
          </div>

          {onRemove && !app.builtin && (
            <button
              type="button"
              onClick={onRemove}
              className="rounded-lg p-1.5 text-vb-silver-faint transition-colors duration-150 hover:bg-vb-loss/10 hover:text-vb-loss active:scale-[0.97]"
              title="Удалить профиль"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          )}
        </div>

        <Toggle
          checked={app.enabled}
          onChange={onToggle}
          disabled={!app.installed}
          label={`Проксировать ${app.name}`}
        />
      </div>

      <AnimatePresence>
        {flash && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <div
              className={cn(
                "flex items-center gap-1.5 pl-12 pt-1.5 text-[12px]",
                flash.includes("Ярлык") ? "text-vb-emerald" : "text-vb-loss",
              )}
            >
              {flash.includes("Ярлык") && <Check className="h-3 w-3" />}
              {flash}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}
