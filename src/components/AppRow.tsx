import { useState } from "react";
import { motion } from "framer-motion";
import {
  Check,
  ChevronDown,
  ExternalLink,
  Loader2,
  MonitorDown,
  Play,
  Trash2,
} from "lucide-react";
import { Button } from "./Button";
import { Toggle } from "./Toggle";
import { cn } from "../lib/cn";
import { fadeInUp } from "../lib/motion";
import type { AppProfileStatus, ShortcutTarget } from "../lib/types";

interface Props {
  app: AppProfileStatus;
  onToggle: (enabled: boolean) => void;
  onLaunch: () => Promise<void>;
  onShortcut: (target: ShortcutTarget) => Promise<string>;
  onRemove?: () => void;
}

/*
  Строка приложения в карте проксируемых приложений.
  - Тумблер «проксировать» слева от действий.
  - Кнопка «Открыть» (запуск через мост) — активна если установлено.
  - Меню ярлыка: Рабочий стол / Панель задач.
  - Кастомные — с кнопкой удаления.
*/
export function AppRow({ app, onToggle, onLaunch, onShortcut, onRemove }: Props) {
  const [busy, setBusy] = useState<null | "launch" | "shortcut">(null);
  const [shortcutOpen, setShortcutOpen] = useState(false);
  const [flash, setFlash] = useState<string | null>(null);

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

  return (
    <motion.div
      variants={fadeInUp}
      layout
      className={cn(
        "surface-card flex flex-col gap-3 p-4 transition-colors",
        app.enabled && "border-vb-emerald/25",
      )}
    >
      <div className="flex items-center gap-3">
        {/* Иконка-плейсхолдер по типу */}
        <div
          className={cn(
            "flex h-10 w-10 shrink-0 items-center justify-center rounded-[10px]",
            app.enabled
              ? "bg-vb-emerald/12 text-vb-emerald"
              : "bg-vb-surface-2 text-vb-silver-dim",
          )}
        >
          <MonitorDown className="h-5 w-5" strokeWidth={1.75} />
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-[15px] font-semibold text-vb-fg">
              {app.name}
            </span>
            {!app.installed && (
              <span className="rounded-md bg-vb-surface-2 px-1.5 py-0.5 text-[10px] font-medium text-vb-silver-faint">
                не найдено
              </span>
            )}
          </div>
          <div className="mt-0.5 truncate text-[12px] text-vb-silver-dim">
            {kindLabel}
            {app.installed && app.desktop_shortcut && " · ярлык готов"}
          </div>
        </div>

        {/* Тумблер проксирования */}
        <Toggle
          checked={app.enabled}
          onChange={onToggle}
          disabled={!app.installed}
          label={`Проксировать ${app.name}`}
        />
      </div>

      {/* Действия */}
      <div className="flex items-center gap-2">
        <Button
          variant="secondary"
          size="sm"
          onClick={launch}
          disabled={!app.installed || busy !== null}
        >
          {busy === "launch" ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Play className="h-3.5 w-3.5" />
          )}
          Открыть
        </Button>

        {/* Ярлык с выбором места */}
        <div className="relative">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setShortcutOpen((v) => !v)}
            disabled={!app.installed || busy !== null}
          >
            {busy === "shortcut" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <ExternalLink className="h-3.5 w-3.5" />
            )}
            Ярлык
            <ChevronDown className="h-3 w-3 opacity-60" />
          </Button>

          {shortcutOpen && (
            <motion.div
              initial={{ opacity: 0, y: -4 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.16, ease: [0.22, 1, 0.36, 1] }}
              className="z-dropdown absolute right-0 top-full mt-1 w-44 overflow-hidden rounded-[10px] border border-vb-border bg-vb-surface shadow-[0_8px_24px_rgba(0,0,0,0.5)]"
              style={{ transformOrigin: "top right" }}
            >
              <button
                type="button"
                onClick={() => makeShortcut("desktop")}
                className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-vb-silver hover:bg-vb-surface-2"
              >
                На рабочий стол
              </button>
              <button
                type="button"
                onClick={() => makeShortcut("start_menu")}
                className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-vb-silver hover:bg-vb-surface-2"
              >
                В меню Пуск
              </button>
            </motion.div>
          )}
        </div>

        {onRemove && !app.builtin && (
          <button
            type="button"
            onClick={onRemove}
            className="ml-auto rounded-md p-1.5 text-vb-silver-faint transition-colors hover:bg-vb-loss/10 hover:text-vb-loss"
            title="Удалить профиль"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        )}

        {flash && (
          <span
            className={cn(
              "ml-auto flex items-center gap-1 text-[12px]",
              flash.includes("Ярлык") ? "text-vb-emerald" : "text-vb-loss",
            )}
          >
            {flash.includes("Ярлык") && <Check className="h-3 w-3" />}
            {flash}
          </span>
        )}
      </div>
    </motion.div>
  );
}
