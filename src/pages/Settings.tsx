import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  ArrowLeft,
  CheckCircle2,
  Loader2,
  Power,
  RefreshCw,
} from "lucide-react";
import {
  disable as autostartDisable,
  enable as autostartEnable,
  isEnabled as autostartIsEnabled,
} from "@tauri-apps/plugin-autostart";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "../components/Button";
import { Toggle } from "../components/Toggle";
import { CountryFlag } from "../components/CountryFlag";
import {
  appsAddCustom,
  appsList,
  checkProxy,
  parseProxy,
  settingsGet,
  settingsSetProxy,
} from "../lib/api";
import type { AppProfileStatus, ProxyCheckResult } from "../lib/types";
import { fadeInUp } from "../lib/motion";
import { cn } from "../lib/cn";

interface Props {
  onBack: () => void;
}

export function Settings({ onBack }: Props) {
  const [currentProxy, setCurrentProxy] = useState("");
  const [editProxy, setEditProxy] = useState("");
  const [editing, setEditing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveOk, setSaveOk] = useState(false);

  const [pinging, setPinging] = useState(false);
  const [pingResult, setPingResult] = useState<ProxyCheckResult | null>(null);

  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [autostartError, setAutostartError] = useState<string | null>(null);

  const [apps, setApps] = useState<AppProfileStatus[]>([]);
  const [customName, setCustomName] = useState("");
  const [customPath, setCustomPath] = useState("");
  const [addingCustom, setAddingCustom] = useState(false);
  const [customError, setCustomError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const s = await settingsGet().catch(() => null);
    if (s?.proxy_url) {
      setCurrentProxy(s.proxy_url);
      setEditProxy(s.proxy_url);
    }
    setAutostart(await autostartIsEnabled().catch(() => false));
    setApps(await appsList().catch(() => []));
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleSaveProxy = async () => {
    setSaving(true);
    setSaveError(null);
    setSaveOk(false);
    try {
      await parseProxy(editProxy);
      await settingsSetProxy(editProxy);
      setCurrentProxy(editProxy);
      setEditing(false);
      setSaveOk(true);
      window.setTimeout(() => setSaveOk(false), 2500);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handlePing = async () => {
    if (!currentProxy) return;
    setPinging(true);
    setPingResult(null);
    try {
      setPingResult(await checkProxy(currentProxy));
    } finally {
      setPinging(false);
    }
  };

  const handleAutostartToggle = async () => {
    setAutostartBusy(true);
    setAutostartError(null);
    try {
      if (autostart) {
        await autostartDisable();
        setAutostart(false);
      } else {
        await autostartEnable();
        setAutostart(true);
      }
    } catch {
      setAutostartError(
        "Не получилось изменить автозапуск (нет прав в Windows). Можно вручную через Пуск → Автозагрузка.",
      );
    } finally {
      setAutostartBusy(false);
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

  const handleAddCustom = async () => {
    setAddingCustom(true);
    setCustomError(null);
    try {
      const updated = await appsAddCustom(customName.trim(), customPath.trim());
      setApps(updated);
      setCustomName("");
      setCustomPath("");
    } catch (e) {
      setCustomError(String(e));
    } finally {
      setAddingCustom(false);
    }
  };

  const customApps = apps.filter((a) => !a.builtin);

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="mx-auto flex w-full max-w-2xl flex-col gap-5 p-7">
        <header className="flex items-center gap-3">
          <button
            type="button"
            onClick={onBack}
            className="rounded-lg p-1.5 text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-silver"
            aria-label="Назад"
          >
            <ArrowLeft className="h-5 w-5" />
          </button>
          <h1 className="text-[22px] font-bold tracking-tight text-vb-fg">Настройки</h1>
        </header>

        {/* Proxy */}
        <motion.section variants={fadeInUp} initial="hidden" animate="show" className="surface-card p-5">
          <h2 className="text-[15px] font-semibold text-vb-fg">Прокси</h2>
          <p className="mt-1 text-[12px] text-vb-silver-dim">
            URL купленного HTTP-прокси (не РФ IP).
          </p>

          {!editing ? (
            <div className="mt-4 flex items-center gap-3">
              <code className="flex-1 truncate rounded-lg border border-vb-border bg-vb-surface-2 px-3 py-2 font-mono text-[12px] text-vb-fg">
                {currentProxy || "—"}
              </code>
              <Button variant="secondary" size="sm" onClick={() => setEditing(true)}>
                Изменить
              </Button>
            </div>
          ) : (
            <div className="mt-4 space-y-3">
              <input
                type="text"
                value={editProxy}
                onChange={(e) => setEditProxy(e.target.value)}
                placeholder="http://login:password@1.2.3.4:8000"
                className="w-full rounded-lg border border-vb-border bg-vb-surface-2 px-3 py-2 font-mono text-[13px] text-vb-fg outline-none transition-colors focus:border-vb-emerald/60"
                autoFocus
              />
              <div className="flex gap-2">
                <Button variant="primary" size="sm" onClick={handleSaveProxy} disabled={saving || !editProxy.trim()}>
                  {saving ? (
                    <>
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      Сохранение…
                    </>
                  ) : (
                    "Сохранить и перезапустить"
                  )}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setEditing(false);
                    setEditProxy(currentProxy);
                    setSaveError(null);
                  }}
                  disabled={saving}
                >
                  Отмена
                </Button>
              </div>
              {saveError && <p className="text-[12px] text-vb-loss">{saveError}</p>}
            </div>
          )}

          {saveOk && (
            <p className="mt-3 flex items-center gap-1.5 text-[12px] text-vb-emerald">
              <CheckCircle2 className="h-3.5 w-3.5" />
              Сохранено, мост перезапущен с новым прокси.
            </p>
          )}

          {/* Ping */}
          <div className="mt-5 border-t border-vb-border pt-4">
            <div className="flex items-center justify-between">
              <h3 className="text-[13px] font-medium text-vb-fg">Проверка прокси</h3>
              <Button variant="secondary" size="sm" onClick={handlePing} disabled={pinging || !currentProxy}>
                {pinging ? (
                  <>
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    Пинг…
                  </>
                ) : (
                  <>
                    <RefreshCw className="h-3.5 w-3.5" />
                    Пинг
                  </>
                )}
              </Button>
            </div>

            <AnimatePresence>
              {pingResult && (
                <motion.div
                  initial={{ opacity: 0, height: 0 }}
                  animate={{ opacity: 1, height: "auto" }}
                  exit={{ opacity: 0, height: 0 }}
                  transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
                  className="overflow-hidden"
                >
                  <div
                    className={cn(
                      "mt-3 rounded-lg border p-3 text-[12px]",
                      pingResult.reachable
                        ? "border-vb-emerald/30 bg-vb-emerald/[0.04]"
                        : "border-vb-loss/30 bg-vb-loss/[0.04]",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <span
                        className={cn(
                          "h-1.5 w-1.5 rounded-full",
                          pingResult.reachable ? "bg-vb-emerald" : "bg-vb-loss",
                        )}
                      />
                      <span className="font-medium text-vb-fg">
                        {pingResult.reachable ? "Прокси доступен" : "Прокси недоступен"}
                      </span>
                      {pingResult.latency_ms !== undefined && (
                        <span className="tnum ml-auto font-mono text-vb-silver-dim">
                          {pingResult.latency_ms} ms
                        </span>
                      )}
                    </div>
                    <div className="mt-2 space-y-1 font-mono text-vb-silver-dim">
                      {pingResult.ip && (
                        <div>
                          IP: <span className="text-vb-fg">{pingResult.ip}</span>
                        </div>
                      )}
                      {pingResult.country_code && (
                        <div className="flex items-center gap-1.5">
                          Страна:
                          <CountryFlag code={pingResult.country_code} />
                          <span className="text-vb-fg">
                            {pingResult.country_name ?? pingResult.country_code}
                          </span>
                        </div>
                      )}
                      {pingResult.isp && (
                        <div>
                          ISP: <span className="text-vb-fg">{pingResult.isp}</span>
                        </div>
                      )}
                      {pingResult.error && <div className="text-vb-loss/90">{pingResult.error}</div>}
                    </div>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        </motion.section>

        {/* Добавить своё приложение */}
        <motion.section variants={fadeInUp} initial="hidden" animate="show" className="surface-card p-5">
          <h2 className="text-[15px] font-semibold text-vb-fg">Добавить приложение</h2>
          <p className="mt-1 text-[12px] text-vb-silver-dim">
            Любой .exe, который надо гнать через мост и killswitch.
          </p>

          <div className="mt-4 space-y-2.5">
            <input
              type="text"
              value={customName}
              onChange={(e) => setCustomName(e.target.value)}
              placeholder="Название (например, WebStorm)"
              className="w-full rounded-lg border border-vb-border bg-vb-surface-2 px-3 py-2 text-[13px] text-vb-fg outline-none transition-colors focus:border-vb-emerald/60"
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
                Выбрать
              </Button>
            </div>
            <Button
              variant="primary"
              size="sm"
              onClick={handleAddCustom}
              disabled={addingCustom || !customName.trim() || !customPath.trim()}
            >
              {addingCustom ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
              Добавить
            </Button>
            {customError && <p className="text-[12px] text-vb-loss">{customError}</p>}
          </div>

          {customApps.length > 0 && (
            <div className="mt-4 space-y-1.5 border-t border-vb-border pt-3">
              {customApps.map((a) => (
                <div key={a.id} className="flex items-center gap-2 text-[12px]">
                  <span className="truncate text-vb-silver">{a.name}</span>
                  <span className="truncate font-mono text-[11px] text-vb-silver-faint">
                    {a.location}
                  </span>
                  {!a.installed && <span className="text-vb-loss">нет файла</span>}
                </div>
              ))}
            </div>
          )}
        </motion.section>

        {/* Autostart */}
        <motion.section variants={fadeInUp} initial="hidden" animate="show" className="surface-card p-5">
          <div className="flex items-start justify-between gap-4">
            <div>
              <h2 className="flex items-center gap-2 text-[15px] font-semibold text-vb-fg">
                <Power className="h-4 w-4" />
                Запускать с Windows
              </h2>
              <p className="mt-1 text-[12px] text-vb-silver-dim">
                Clod Helper стартует в трее при входе. Мост автостартует отдельно —
                задачей Планировщика (с автоперезапуском), GUI ему не нужен.
              </p>
            </div>
            {autostart === null ? (
              <Loader2 className="h-5 w-5 animate-spin text-vb-silver-dim" />
            ) : (
              <Toggle
                checked={autostart}
                onChange={handleAutostartToggle}
                disabled={autostartBusy}
                label="Автозапуск"
              />
            )}
          </div>
          {autostartError && (
            <p className="mt-3 text-[12px] text-vb-warn">{autostartError}</p>
          )}
        </motion.section>
      </div>
    </div>
  );
}
