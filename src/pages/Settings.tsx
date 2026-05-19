import { useCallback, useEffect, useState } from "react";
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
import { Button } from "../components/Button";
import { CountryFlag } from "../components/CountryFlag";
import {
  checkProxy,
  parseProxy,
  settingsGet,
  settingsSetProxy,
} from "../lib/api";
import type { ProxyCheckResult } from "../lib/types";
import { cn } from "../lib/cn";

interface Props {
  onBack: () => void;
}

export function Settings({ onBack }: Props) {
  const [currentProxy, setCurrentProxy] = useState<string>("");
  const [editProxy, setEditProxy] = useState<string>("");
  const [editing, setEditing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveOk, setSaveOk] = useState(false);

  const [pinging, setPinging] = useState(false);
  const [pingResult, setPingResult] = useState<ProxyCheckResult | null>(null);

  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [autostartError, setAutostartError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const s = await settingsGet();
      if (s.proxy_url) {
        setCurrentProxy(s.proxy_url);
        setEditProxy(s.proxy_url);
      }
    } catch {
      // ignore
    }
    try {
      const enabled = await autostartIsEnabled();
      setAutostart(enabled);
    } catch {
      setAutostart(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleSaveProxy = async () => {
    setSaving(true);
    setSaveError(null);
    setSaveOk(false);
    try {
      // Pre-validate (даёт быструю обратную связь при опечатке).
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
      const r = await checkProxy(currentProxy);
      setPingResult(r);
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
    } catch (e) {
      // На некоторых корпоративных Windows нет прав в HKCU\Run.
      // Не падаем — показываем подсказку.
      setAutostartError(
        "Не получилось добавить в автозапуск (нет прав в Windows). " +
          "Можно вручную: ярлык в Пуск → Автозагрузка.",
      );
    } finally {
      setAutostartBusy(false);
    }
  };

  return (
    <div className="flex h-full flex-col gap-6 p-8 text-vb-silver">
      <header className="flex items-center gap-3">
        <button
          type="button"
          onClick={onBack}
          className="rounded-md p-1 text-vb-silver-dim hover:bg-vb-surface hover:text-vb-silver"
          aria-label="Назад"
        >
          <ArrowLeft className="h-5 w-5" />
        </button>
        <h1 className="text-2xl font-semibold tracking-tight text-white">
          Настройки
        </h1>
      </header>

      {/* Proxy */}
      <section className="rounded-2xl border border-white/[0.04] bg-[rgba(10,10,10,0.55)] p-6 backdrop-blur-xl">
        <h2 className="text-base font-semibold text-white">Прокси</h2>
        <p className="mt-1 text-xs text-vb-silver-dim">
          URL вашего купленного HTTP-прокси (не РФ IP).
        </p>

        {!editing ? (
          <div className="mt-4 flex items-center gap-3">
            <code className="flex-1 truncate rounded-md border border-vb-border bg-vb-surface px-3 py-2 font-mono text-xs text-white">
              {currentProxy || "—"}
            </code>
            <Button variant="secondary" onClick={() => setEditing(true)}>
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
              className="w-full rounded-md border border-vb-border bg-vb-surface px-3 py-2 font-mono text-sm text-white outline-none focus:border-vb-emerald/60"
              autoFocus
            />
            <div className="flex gap-2">
              <Button
                variant="primary"
                onClick={handleSaveProxy}
                disabled={saving || !editProxy.trim()}
              >
                {saving ? (
                  <>
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Сохранение…
                  </>
                ) : (
                  "Сохранить и перезапустить"
                )}
              </Button>
              <Button
                variant="ghost"
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
            {saveError && (
              <p className="text-xs text-red-300">{saveError}</p>
            )}
          </div>
        )}

        {saveOk && (
          <p className="mt-3 flex items-center gap-1.5 text-xs text-vb-emerald">
            <CheckCircle2 className="h-3.5 w-3.5" />
            Сохранено, шим перезапущен с новым прокси.
          </p>
        )}

        {/* Ping */}
        <div className="mt-6 border-t border-white/[0.04] pt-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium text-white">Проверка прокси</h3>
            <Button
              variant="secondary"
              onClick={handlePing}
              disabled={pinging || !currentProxy}
            >
              {pinging ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Пинг…
                </>
              ) : (
                <>
                  <RefreshCw className="h-4 w-4" />
                  Пинг
                </>
              )}
            </Button>
          </div>

          {pingResult && (
            <div
              className={cn(
                "mt-3 rounded-lg border p-3 text-xs",
                pingResult.reachable
                  ? "border-vb-emerald/30 bg-vb-emerald/[0.04]"
                  : "border-red-500/30 bg-red-500/[0.04]",
              )}
            >
              <div className="flex items-center gap-2">
                <span
                  className={cn(
                    "h-1.5 w-1.5 rounded-full",
                    pingResult.reachable ? "bg-vb-emerald" : "bg-red-500",
                  )}
                />
                <span className="font-medium text-white">
                  {pingResult.reachable ? "Прокси доступен" : "Прокси недоступен"}
                </span>
                {pingResult.latency_ms !== undefined && (
                  <span className="ml-auto font-mono text-vb-silver-dim">
                    {pingResult.latency_ms} ms
                  </span>
                )}
              </div>
              <div className="mt-2 space-y-1 font-mono text-vb-silver-dim">
                {pingResult.ip && (
                  <div>IP: <span className="text-white">{pingResult.ip}</span></div>
                )}
                {pingResult.country_code && (
                  <div className="flex items-center gap-1.5">
                    Страна:
                    <CountryFlag code={pingResult.country_code} />
                    <span className="text-white">
                      {pingResult.country_name ?? pingResult.country_code}
                    </span>
                  </div>
                )}
                {pingResult.isp && (
                  <div>ISP: <span className="text-white">{pingResult.isp}</span></div>
                )}
                {pingResult.error && (
                  <div className="text-red-300/80">{pingResult.error}</div>
                )}
              </div>
            </div>
          )}
        </div>
      </section>

      {/* Autostart */}
      <section className="rounded-2xl border border-white/[0.04] bg-[rgba(10,10,10,0.55)] p-6 backdrop-blur-xl">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="flex items-center gap-2 text-base font-semibold text-white">
              <Power className="h-4 w-4" />
              Запускать с Windows
            </h2>
            <p className="mt-1 text-xs text-vb-silver-dim">
              Clod Helper будет стартовать в трее при входе в Windows.
              Шим (прокси-мост) автостартует отдельно через свой .vbs.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={autostart ?? false}
            onClick={handleAutostartToggle}
            disabled={autostartBusy || autostart === null}
            className={cn(
              "relative inline-flex h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-50",
              autostart ? "bg-vb-emerald" : "bg-vb-border",
            )}
          >
            <span
              className={cn(
                "inline-block h-5 w-5 translate-y-0.5 rounded-full bg-white shadow transition-transform",
                autostart ? "translate-x-5" : "translate-x-0.5",
              )}
            />
          </button>
        </div>
        {autostartError && (
          <p className="mt-3 text-xs text-yellow-300/80">{autostartError}</p>
        )}
      </section>
    </div>
  );
}
