import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Check, Loader2, X } from "lucide-react";
import { Toggle } from "./Toggle";
import { AppIcon } from "./AppTile";
import type { RowStatus } from "./AppRow";
import {
  appsCreateShortcut,
  appsSettingsSet,
  proxiesGet,
  proxyAssignmentsGet,
  proxyAssignmentsSet,
  proxyLabelsGet,
} from "../lib/api";
import type { AppFolder } from "../lib/appLayout";
import type { AppProfileStatus } from "../lib/types";
import { cn } from "../lib/cn";

/** Встроенные, чей прокси — назначение сервиса (мост роутит по доменам). */
const GROUP_OF: Record<string, string> = {
  "claude-desktop": "anthropic",
  codex: "openai",
  telegram: "telegram",
};

function hostPort(url: string): string {
  const rest = url.includes("://") ? url.split("://")[1] : url;
  return rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest;
}

interface Props {
  app: AppProfileStatus | null;
  icon?: string;
  status: RowStatus | null;
  folders: AppFolder[];
  onMoveToFolder: (folderId: string) => void;
  onRemove: () => void;
  onRelaunch: () => Promise<void>;
  onChanged: (apps?: AppProfileStatus[]) => void;
  onClose: () => void;
}

/*
  Настройки приложения — шторка в духе iOS: прокси (Авто / конкретный из
  пула), «весь трафик или только AI», папка, ярлыки, убрать из списка.
  Изменения сохраняются сразу; если приложение открыто — предлагаем
  перезапустить через мост (прокси применяется при запуске).
*/
export function AppSettingsModal({
  app,
  icon,
  status,
  folders,
  onMoveToFolder,
  onRemove,
  onRelaunch,
  onChanged,
  onClose,
}: Props) {
  const [pool, setPool] = useState<string[]>([]);
  const [labels, setLabels] = useState<Record<string, string>>({});
  const [assign, setAssign] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    if (!app) return;
    setNote(null);
    void Promise.all([
      proxiesGet().then(setPool).catch(() => {}),
      proxyLabelsGet().then(setLabels).catch(() => {}),
      proxyAssignmentsGet().then(setAssign).catch(() => {}),
    ]);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [app, onClose]);

  const group = app ? GROUP_OF[app.id] : undefined;
  const current = !app ? -1 : group ? (assign[group] ?? -1) : (app.proxy ?? -1);
  const launchable = app ? !group : false; // VS Code и свои: флаг --proxy-server

  const run = async (fn: () => Promise<unknown>, done?: string) => {
    setBusy(true);
    setNote(null);
    try {
      const r = await fn();
      onChanged(Array.isArray(r) ? (r as AppProfileStatus[]) : undefined);
      if (done) setNote(done);
    } catch (e) {
      setNote(String(e));
    } finally {
      setBusy(false);
    }
  };

  const pickProxy = (v: number) => {
    if (!app || v === current) return;
    if (group) {
      const next = { ...assign };
      if (v < 0) delete next[group];
      else next[group] = v;
      setAssign(next);
      void run(() => proxyAssignmentsSet(next), "Сохранено — мост перезапущен");
    } else {
      void run(() => appsSettingsSet(app.id, v < 0 ? null : v, app.full_proxy ?? null), "Сохранено");
    }
  };

  const options = [
    { v: -1, title: "Авто", sub: "Основной прокси" },
    ...pool.map((url, i) => ({
      v: i,
      title: labels[url] || hostPort(url),
      sub: labels[url] ? hostPort(url) : i === 0 ? "Основной" : `Запасной ${i}`,
    })),
  ];
  const currentFolder = app ? folders.find((f) => f.apps.includes(app.id))?.id : undefined;

  return createPortal(
    <AnimatePresence>
      {app && (
        <motion.div
          key="backdrop"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          onMouseDown={onClose}
          className="fixed inset-0 z-[70] flex items-center justify-center bg-black/45 p-6 backdrop-blur-[6px]"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.94, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 8 }}
            transition={{ type: "spring", bounce: 0.18, duration: 0.42 }}
            onMouseDown={(e) => e.stopPropagation()}
            role="dialog"
            aria-label={`Настройки ${app.name}`}
            className="max-h-full w-full max-w-[400px] overflow-y-auto rounded-2xl border border-vb-border bg-vb-surface shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
          >
            {/* Шапка */}
            <div className="flex items-center gap-3 p-4 pb-3">
              <AppIcon name={app.name} icon={icon} size={44} dim={!app.installed} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-[16px] font-semibold text-vb-fg">{app.name}</div>
                <div
                  className={cn(
                    "truncate text-[12px]",
                    status?.tone === "ok"
                      ? "text-vb-emerald"
                      : status?.tone === "warn"
                        ? "text-vb-warn"
                        : "text-vb-silver-faint",
                  )}
                >
                  {status?.text ?? (app.installed ? "" : "Не установлен")}
                </div>
              </div>
              <button
                type="button"
                onClick={onClose}
                aria-label="Закрыть"
                className="rounded-full bg-vb-surface-2 p-1.5 text-vb-silver-faint transition-colors hover:text-vb-silver"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>

            {/* Прокси */}
            <Section title="Прокси">
              {options.map((o) => (
                <button
                  key={o.v}
                  type="button"
                  disabled={busy}
                  onClick={() => pickProxy(o.v)}
                  className="flex w-full items-center gap-3 px-3.5 py-2.5 text-left transition-colors hover:bg-vb-surface-2/60"
                >
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px] text-vb-fg">{o.title}</span>
                    <span className="block truncate text-[11px] text-vb-silver-faint">{o.sub}</span>
                  </span>
                  {o.v === current &&
                    (busy ? (
                      <Loader2 className="h-4 w-4 animate-spin text-vb-silver-faint" />
                    ) : (
                      <Check className="h-4 w-4 text-vb-emerald" />
                    ))}
                </button>
              ))}
            </Section>

            {/* Трафик — только для запускаемых с --proxy-server */}
            {launchable && (
              <Section>
                <div className="flex items-center justify-between gap-3 px-3.5 py-2.5">
                  <span className="min-w-0">
                    <span className="block text-[13px] text-vb-fg">Весь трафик через прокси</span>
                    <span className="block text-[11px] text-vb-silver-faint">
                      {current >= 0 ? "С выбранным прокси — всегда весь" : "Иначе через прокси только AI-сайты"}
                    </span>
                  </span>
                  <Toggle
                    checked={current >= 0 || app.full_proxy}
                    disabled={busy || current >= 0}
                    onChange={() =>
                      void run(
                        () => appsSettingsSet(app.id, current < 0 ? null : current, !app.full_proxy),
                        "Сохранено",
                      )
                    }
                    label="Весь трафик через прокси"
                  />
                </div>
              </Section>
            )}

            {/* Папка */}
            {folders.length > 0 && (
              <div className="px-4 pb-3">
                <div className="mb-1.5 text-[11px] text-vb-silver-faint">Папка</div>
                <div className="flex flex-wrap gap-1.5">
                  {folders.map((f) => (
                    <button
                      key={f.id}
                      type="button"
                      onClick={() => onMoveToFolder(f.id)}
                      className={cn(
                        "rounded-full border px-2.5 py-1 text-[12px] transition-colors",
                        f.id === currentFolder
                          ? "border-vb-emerald/50 bg-vb-emerald/10 text-vb-fg"
                          : "border-vb-border text-vb-silver-dim hover:text-vb-silver",
                      )}
                    >
                      {f.name}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {/* Действия */}
            <Section>
              {app.installed && launchable && (
                <>
                  <ActionRow label="Ярлык на рабочий стол" onClick={() => void run(() => appsCreateShortcut(app.id, "desktop"), "Ярлык на рабочем столе")} />
                  <ActionRow label="Ярлык в «Программы»" onClick={() => void run(() => appsCreateShortcut(app.id, "start_menu"), "Ярлык в «Программах»")} />
                </>
              )}
              {app.installed && (
                <ActionRow label="Перезапустить через мост" onClick={() => void run(onRelaunch, "Перезапущено")} />
              )}
              <ActionRow label="Убрать из списка" danger onClick={onRemove} />
            </Section>

            <AnimatePresence>
              {note && (
                <motion.p
                  initial={{ opacity: 0, height: 0 }}
                  animate={{ opacity: 1, height: "auto" }}
                  exit={{ opacity: 0, height: 0 }}
                  className="overflow-hidden px-4 pb-3 text-center text-[12px] text-vb-silver-dim"
                >
                  {note}
                </motion.p>
              )}
            </AnimatePresence>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

function Section({ title, children }: { title?: string; children: React.ReactNode }) {
  return (
    <div className="px-4 pb-3">
      {title && <div className="mb-1.5 text-[11px] text-vb-silver-faint">{title}</div>}
      <div className="divide-y divide-vb-border/60 overflow-hidden rounded-xl bg-vb-surface-2/50">{children}</div>
    </div>
  );
}

function ActionRow({ label, danger, onClick }: { label: string; danger?: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "block w-full px-3.5 py-2.5 text-left text-[13px] transition-colors hover:bg-vb-surface-2/70",
        danger ? "text-vb-loss" : "text-vb-silver",
      )}
    >
      {label}
    </button>
  );
}
