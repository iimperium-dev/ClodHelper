import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { FolderOpen, Loader2, Plus } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "../components/Button";
import { AppRow } from "../components/AppRow";
import { InfoTip } from "../components/InfoTip";
import {
  appsAddCustom,
  appsCreateShortcut,
  appsLaunch,
  appsList,
  appsRemove,
  appsSet,
} from "../lib/api";
import type { AppProfileStatus, ShortcutTarget } from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";

/*
  Приложения: список профилей + добавление своего .exe В ОДНОМ месте
  (раньше форма жила в Настройках, а список на Dashboard — context switch).
*/
export function Apps() {
  const [apps, setApps] = useState<AppProfileStatus[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [customName, setCustomName] = useState("");
  const [customPath, setCustomPath] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setApps(await appsList().catch(() => []));
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const toggleApp = async (id: string, enabled: boolean) => {
    if (!apps) return;
    const next = apps.map((a) => (a.id === id ? { ...a, enabled } : a));
    setApps(next);
    try {
      const saved = await appsSet(
        next.map((a) => ({
          id: a.id,
          name: a.name,
          kind: a.kind,
          // Пути/AppID не шлём: location — runtime-снимок, он пустеет, когда
          // exe временно недоступен (отключённый диск), и затирал бы
          // сохранённый путь навсегда. Бэкенд бережёт сохранённые значения.
          exe_path: undefined,
          app_id: undefined,
          process_names: a.process_names,
          enabled: a.enabled,
          builtin: a.builtin,
        })),
      );
      setApps(saved);
    } catch {
      await load();
    }
  };

  const removeApp = async (id: string) => {
    try {
      setApps(await appsRemove(id));
    } catch {
      await load();
    }
  };

  const pickExe = async () => {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: "Программа", extensions: ["exe"] }],
    });
    if (typeof selected === "string") {
      setCustomPath(selected);
      if (!customName) {
        const base = selected.split(/[\\/]/).pop()?.replace(/\.exe$/i, "") ?? "";
        setCustomName(base);
      }
    }
  };

  const handleAdd = async () => {
    setAddBusy(true);
    setAddError(null);
    try {
      setApps(await appsAddCustom(customName.trim(), customPath.trim()));
      setCustomName("");
      setCustomPath("");
      setAdding(false);
    } catch (e) {
      setAddError(String(e));
    } finally {
      setAddBusy(false);
    }
  };

  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[620px] flex-col px-8 py-7"
    >
      <motion.header variants={fadeInUp} className="flex items-end justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-[24px] font-bold leading-tight tracking-[-0.02em] text-vb-fg">
            Приложения
            <InfoTip>
              Зачем это, если есть системное проксирование? GUI-приложения на
              Chromium/Electron (Claude Desktop, Cursor) <b>игнорируют</b>{" "}
              переменные HTTP_PROXY — прокси им передаётся только флагом при
              запуске. «Открыть» запускает приложение с этим флагом сейчас,
              ярлык — делает такой запуск постоянным. CLI-инструменты (Claude
              Code, git, node) в этом не нуждаются — их покрывает системное
              проксирование.
            </InfoTip>
          </h1>
          <p className="mt-0.5 text-[13px] text-vb-silver-dim">
            GUI-приложения, которым нужен запуск с прокси-флагом
          </p>
        </div>
        <Button variant="secondary" size="sm" onClick={() => setAdding((v) => !v)}>
          <Plus className="h-3.5 w-3.5" />
          Добавить
        </Button>
      </motion.header>

      {/* Форма добавления — inline-раскрытие, не модалка */}
      <AnimatePresence>
        {adding && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <div className="surface-card mt-4 p-4">
              <div className="space-y-2.5">
                <input
                  type="text"
                  value={customName}
                  onChange={(e) => setCustomName(e.target.value)}
                  placeholder="Название (например, WebStorm)"
                  className="w-full rounded-lg border border-vb-border bg-vb-surface-2 px-3 py-2 text-[13px] text-vb-fg outline-none transition-colors focus:border-vb-emerald/60"
                  autoFocus
                />
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={customPath}
                    onChange={(e) => setCustomPath(e.target.value)}
                    placeholder="Путь к .exe"
                    className="min-w-0 flex-1 truncate rounded-lg border border-vb-border bg-vb-surface-2 px-3 py-2 font-mono text-[12px] text-vb-fg outline-none transition-colors focus:border-vb-emerald/60"
                  />
                  <Button variant="secondary" size="sm" onClick={pickExe}>
                    <FolderOpen className="h-3.5 w-3.5" />
                    Выбрать
                  </Button>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={handleAdd}
                    disabled={addBusy || !customName.trim() || !customPath.trim()}
                  >
                    {addBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                    Добавить
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setAdding(false);
                      setAddError(null);
                    }}
                  >
                    Отмена
                  </Button>
                </div>
                {addError && <p className="text-[12px] text-vb-loss">{addError}</p>}
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Список — одна поверхность, hairline-разделители */}
      <motion.section variants={fadeInUp} className="mt-5">
        {apps === null ? (
          <div className="surface-card flex items-center justify-center gap-2 p-8 text-[13px] text-vb-silver-dim">
            <Loader2 className="h-4 w-4 animate-spin" />
            Загружаю приложения…
          </div>
        ) : apps.length === 0 ? (
          <div className="surface-card p-8 text-center text-[13px] text-vb-silver-dim">
            Приложений пока нет — добавьте своё через кнопку выше.
          </div>
        ) : (
          <div className="surface-card divide-y divide-vb-border/70 overflow-visible">
            <AnimatePresence mode="popLayout">
              {apps.map((app) => (
                <AppRow
                  key={app.id}
                  app={app}
                  onToggle={(en) => toggleApp(app.id, en)}
                  onLaunch={() => appsLaunch(app.id)}
                  onShortcut={(t: ShortcutTarget) => appsCreateShortcut(app.id, t)}
                  onRemove={app.builtin ? undefined : () => removeApp(app.id)}
                />
              ))}
            </AnimatePresence>
          </div>
        )}
      </motion.section>

    </motion.div>
  );
}
