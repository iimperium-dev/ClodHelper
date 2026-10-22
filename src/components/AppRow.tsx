import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  Check,
  CheckCircle2,
  Loader2,
  GripVertical,
  Minus,
  Play,
  Settings2,
} from "lucide-react";
import { Button } from "./Button";
import { AppIcon } from "./AppTile";
import { cn } from "../lib/cn";
import type { AppProfileStatus } from "../lib/types";

/** Статус-строка под названием: что реально происходит с трафиком. */
export interface RowStatus {
  tone: "ok" | "warn" | "muted";
  text: string;
}

/** Действие строки (главная кнопка или пункт меню «…»). */
export interface RowAction {
  label: string;
  run: () => Promise<unknown> | void;
  danger?: boolean;
}

interface Props {
  app: AppProfileStatus;
  /** null — статуса нет (ещё не узнали): тихая подпись типа. */
  status: RowStatus | null;
  /** Главная кнопка, когда нужно действие («Подключить»). Иначе — «Открыть». */
  action?: RowAction | null;
  onLaunch: () => Promise<void>;
  /** Настоящая иконка программы. */
  icon?: string;
  /** Режим упорядочивания: «−» слева, ручка справа, без кнопок. */
  editing?: boolean;
  onRemove?: () => void;
  /** Открыть настройки приложения (шестерёнка при наведении). */
  onSettings?: () => void;
}

/*
  Строка приложения — одна для всех (Claude, Telegram, VS Code, свои):
  логотип, имя, ОДНА строка статуса по живым соединениям, ОДНА кнопка —
  «Подключить», если трафик идёт мимо моста, иначе «Открыть». Всё
  второстепенное — в настройках приложения (шестерёнка).
*/
export function AppRow({
  app,
  status,
  action,
  onLaunch,
  icon,
  editing,
  onRemove,
  onSettings,
}: Props) {
  const [hover, setHover] = useState(false);
  const [busy, setBusy] = useState(false);
  const [flash, setFlash] = useState<{ ok: boolean; text: string } | null>(null);

  // Дополнительные действия (ярлыки, «Отключить прокси Claude»…) — в окне
  // настроек приложения (шестерёнка на иконке), не в меню «…» строки.
  const run = async (fn: () => Promise<unknown> | void, okText?: string) => {
    setBusy(true);
    try {
      const res = await fn();
      const text = okText ?? (typeof res === "string" ? res : null);
      if (text) {
        setFlash({ ok: true, text });
        window.setTimeout(() => setFlash(null), 4000);
      }
    } catch (e) {
      setFlash({ ok: false, text: String(e) });
      window.setTimeout(() => setFlash(null), 6000);
    } finally {
      setBusy(false);
    }
  };

  const kindLabel =
    app.kind === "msix" ? "Microsoft Store" : app.kind === "custom" ? "Своё приложение" : "Приложение";
  const sub: RowStatus = !app.installed
    ? { tone: "muted", text: "Не установлен" }
    : (status ?? { tone: "muted", text: kindLabel });

  return (
    <motion.div
      layout
      className="px-4 py-3"
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      <div className="flex items-center gap-3">
        <AnimatePresence initial={false}>
          {editing && (
            <motion.button
              type="button"
              initial={{ width: 0, opacity: 0, scale: 0.5 }}
              animate={{ width: 22, opacity: 1, scale: 1 }}
              exit={{ width: 0, opacity: 0, scale: 0.5 }}
              transition={{ type: "spring", bounce: 0.3, duration: 0.35 }}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={onRemove}
              aria-label={`Убрать ${app.name}`}
              className="-mr-1 flex h-[22px] shrink-0 items-center justify-center rounded-full bg-vb-loss text-white"
            >
              <Minus className="h-3 w-3" strokeWidth={3} />
            </motion.button>
          )}
        </AnimatePresence>
        {/* В списке настройки — кнопкой справа; шестерёнка на иконке — только у плиток */}
        <AppIcon name={app.name} icon={icon} size={32} dim={!app.installed} />

        <div className="min-w-0 flex-1">
          <div
            className={cn(
              "truncate text-[14px] font-semibold",
              app.installed ? "text-vb-fg" : "text-vb-silver-dim",
            )}
          >
            {app.name}
          </div>
          <div
            className={cn(
              "mt-px flex items-center gap-1 truncate text-[12px]",
              sub.tone === "ok" && "text-vb-emerald",
              sub.tone === "warn" && "text-vb-warn",
              sub.tone === "muted" && "text-vb-silver-faint",
            )}
          >
            {sub.tone === "ok" && <CheckCircle2 className="h-3 w-3 shrink-0" />}
            {sub.tone === "warn" && <AlertCircle className="h-3 w-3 shrink-0" />}
            <span className="truncate">{sub.text}</span>
          </div>
        </div>

        {editing ? (
          <GripVertical className="h-4 w-4 shrink-0 cursor-grab text-vb-silver-faint" aria-hidden />
        ) : app.installed && (
          <div className="flex shrink-0 items-center gap-1">
            {action ? (
              <Button size="sm" variant="secondary" onClick={() => run(action.run)} disabled={busy}>
                {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                {action.label}
              </Button>
            ) : (
              <button
                type="button"
                onClick={() => run(onLaunch)}
                disabled={busy}
                className={cn(
                  "flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px] font-medium text-vb-silver-dim",
                  "transition-colors duration-150 hover:bg-vb-surface-2 hover:text-vb-silver",
                  "active:scale-[0.97] disabled:opacity-35",
                )}
              >
                {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                Открыть
              </button>
            )}
            {/* Настройки — там же, где раньше было «…»: в покое тихо, при
                наведении на строку заметно, на самой кнопке — зелёным. */}
            {onSettings && (
              <button
                type="button"
                onClick={onSettings}
                aria-label={`Настройки — ${app.name}`}
                title="Настройки"
                className={cn(
                  "group/gear rounded-lg p-1.5 transition-colors duration-150 hover:bg-vb-emerald/10 hover:text-vb-emerald active:scale-[0.95]",
                  hover ? "text-vb-silver" : "text-vb-silver-faint/70",
                )}
              >
                <Settings2 className="h-4 w-4 transition-transform duration-300 group-hover/gear:rotate-45" />
              </button>
            )}
          </div>
        )}
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
                "flex items-start gap-1.5 pl-11 pt-1.5 text-[12px] leading-snug",
                flash.ok ? "text-vb-emerald" : "text-vb-loss",
              )}
            >
              {flash.ok && <Check className="mt-0.5 h-3 w-3 shrink-0" />}
              {flash.text}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}
