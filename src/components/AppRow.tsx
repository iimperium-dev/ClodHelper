import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  Check,
  CheckCircle2,
  Loader2,
  MonitorDown,
  MoreHorizontal,
  Play,
} from "lucide-react";
import { Toggle } from "./Toggle";
import { Button } from "./Button";
import { isMac } from "../lib/platform";
import { appBrand, BrandTile } from "./BrandLogos";
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
  /** null — статуса нет (Windows / ещё не узнали): тихая подпись типа. */
  status: RowStatus | null;
  /** Главная кнопка, когда нужно действие («Подключить»). Иначе — «Открыть». */
  action?: RowAction | null;
  /** Пункты меню «…» (ярлыки, отключение, удаление). */
  menu: RowAction[];
  onLaunch: () => Promise<void>;
  /** Настоящая иконка программы (macOS, свои приложения). */
  icon?: string;
  /** Тумблер — только Windows (там он управляет killswitch). */
  onToggle?: (enabled: boolean) => void;
}

/*
  Строка приложения — одна для всех (Claude, Telegram, VS Code, свои):
  логотип, имя, ОДНА строка статуса по живым соединениям, ОДНА кнопка —
  «Подключить», если трафик идёт мимо моста, иначе «Открыть». Всё
  второстепенное — в меню «…».
*/
export function AppRow({ app, status, action, menu, onLaunch, onToggle, icon }: Props) {
  const [busy, setBusy] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [flash, setFlash] = useState<{ ok: boolean; text: string } | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  // Light dismiss: клик вне меню закрывает его.
  useEffect(() => {
    if (!menuOpen) return;
    const close = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menuOpen]);

  const run = async (fn: () => Promise<unknown> | void, okText?: string) => {
    setBusy(true);
    setMenuOpen(false);
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

  const brand = appBrand(app.name);
  const kindLabel =
    app.kind === "msix" ? "Microsoft Store" : app.kind === "custom" ? "Своё приложение" : "Приложение";
  const sub: RowStatus = !app.installed
    ? { tone: "muted", text: "Не установлен" }
    : (status ?? { tone: "muted", text: kindLabel });

  return (
    <motion.div layout className="px-4 py-3">
      <div className="flex items-center gap-3">
        {icon && !brand ? (
          <img
            src={icon}
            alt=""
            draggable={false}
            className={cn("h-8 w-8 shrink-0", !app.installed && "opacity-50 saturate-50")}
          />
        ) : brand ? (
          <BrandTile glow={brand.glow} disabled={!app.installed}>
            {brand.glyph}
          </BrandTile>
        ) : (
          <div
            className={cn(
              "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg",
              app.installed ? "bg-vb-emerald/12 text-vb-emerald" : "bg-vb-surface-2 text-vb-silver-dim",
            )}
          >
            <MonitorDown className="h-[18px] w-[18px]" strokeWidth={1.75} />
          </div>
        )}

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

        {app.installed && (
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

            {menu.length > 0 && (
              <div className="relative" ref={menuRef}>
                <button
                  type="button"
                  onClick={() => setMenuOpen((v) => !v)}
                  disabled={busy}
                  aria-label={`Ещё — ${app.name}`}
                  className="rounded-lg p-1.5 text-vb-silver-faint transition-colors duration-150 hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.97]"
                >
                  <MoreHorizontal className="h-4 w-4" />
                </button>
                <AnimatePresence>
                  {menuOpen && (
                    <motion.div
                      initial={{ opacity: 0, scale: 0.96, y: -4 }}
                      animate={{ opacity: 1, scale: 1, y: 0 }}
                      exit={{ opacity: 0, scale: 0.98, y: -2 }}
                      transition={{ duration: 0.14, ease: [0.22, 1, 0.36, 1] }}
                      style={{ transformOrigin: "top right" }}
                      className="z-dropdown absolute right-0 top-full mt-1 w-56 overflow-hidden rounded-lg border border-vb-border bg-vb-surface py-1 shadow-[0_8px_24px_rgba(0,0,0,0.5)]"
                    >
                      {menu.map((m) => (
                        <button
                          key={m.label}
                          type="button"
                          onClick={() => run(m.run)}
                          className={cn(
                            "flex w-full items-center px-3 py-2 text-left text-[13px] transition-colors hover:bg-vb-surface-2",
                            m.danger ? "text-vb-loss" : "text-vb-silver",
                          )}
                        >
                          {m.label}
                        </button>
                      ))}
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            )}
          </div>
        )}

        {!isMac && onToggle && (
          <Toggle
            checked={app.enabled}
            onChange={onToggle}
            disabled={!app.installed}
            label={`Проксировать ${app.name}`}
          />
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
