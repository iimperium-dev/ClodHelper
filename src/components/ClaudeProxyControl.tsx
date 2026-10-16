import { useCallback, useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "./Button";
import { claudeProxyDisable, claudeProxyEnable, claudeProxyStatus } from "../lib/api";
import type { ClaudeProxyStatus } from "../lib/types";
import { isMac } from "../lib/platform";

export function ClaudeProxyControl() {
  const [status, setStatus] = useState<ClaudeProxyStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    try {
      setStatus(await claudeProxyStatus());
      setError(null);
    } catch (e) {
      setStatus(null);
      setError(String(e));
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const change = async (enable: boolean) => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const next = await (enable ? claudeProxyEnable() : claudeProxyDisable());
      setStatus(next);
      setMessage(next.policy.notice ?? (enable
        ? isMac
          ? "Настройка сохранена. Полностью закройте Claude (Cmd+Q) и откройте снова."
          : "Настройка сохранена. Полностью закройте Claude через трей и откройте снова."
        : "Прежняя настройка восстановлена. Перезапустите Claude, чтобы применить её."));
    } catch (e) {
      // A later stage can fail after the registry was written. Refresh facts,
      // but retain the error instead of presenting the operation as successful.
      setStatus(await claudeProxyStatus().catch(() => null));
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const configured = status?.policy.configured === true;
  return (
    <section className="mt-6 border-t border-vb-border pt-5" aria-labelledby="claude-proxy-title" aria-busy={busy}>
      <h2 id="claude-proxy-title" className="text-[15px] font-semibold text-vb-fg">Claude Desktop и агенты</h2>
      <p className="mt-1 text-[13px] leading-relaxed text-vb-silver-dim">
        Штатная настройка передаёт прокси приложению, агенту и локальному Cowork.
        Отдельный вход Helper направляет все полученные запросы через ваш прокси.
      </p>
      {status && (
        <dl className="my-3 space-y-1.5 text-[12px]">
          <div className="flex flex-wrap justify-between gap-x-4 gap-y-1">
            <dt className="text-vb-silver-dim">Настройка Claude</dt>
            <dd className="text-vb-silver">{configured ? "Сохранена, применяется при запуске" : "Не включена"}</dd>
          </div>
          <div className="flex flex-wrap justify-between gap-x-4 gap-y-1">
            <dt className="text-vb-silver-dim">Мост без DIRECT</dt>
            <dd className="text-vb-silver">{status.bridge_ready ? "Готов" : "Недоступен"}</dd>
          </div>
          <div className="flex flex-wrap justify-between gap-x-4 gap-y-1">
            <dt className="text-vb-silver-dim">Блокировка обхода всеми инструментами</dt>
            <dd className="text-vb-silver">Не обеспечена</dd>
          </div>
        </dl>
      )}
      <p className="my-3 text-[12px] leading-relaxed text-vb-silver-dim">
        Программы, игнорирующие прокси, и отдельные службы пока могут выйти напрямую.
        Эта настройка не означает полную защиту от утечек.
      </p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="secondary" disabled={busy || !status || !!status.policy.conflict || (configured && status.bridge_ready)} onClick={() => change(true)}>
          {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />}
          {configured && !status?.bridge_ready ? "Восстановить мост Claude" : "Настроить Claude"}
        </Button>
        {status?.policy.managed && <Button size="sm" variant="ghost" disabled={busy} onClick={() => change(false)}>Вернуть прежние настройки</Button>}
        <Button size="sm" variant="ghost" disabled={busy} onClick={refresh}>Проверить</Button>
      </div>
      <p className="mt-2 text-[11px] text-vb-silver-dim">
        {isMac
          ? "Для изменения настройки Claude macOS запросит пароль администратора."
          : "Для изменения настройки Claude Windows запросит права администратора."}
      </p>
      <div aria-live="polite">
        {status?.policy.conflict && <p className="mt-2 text-[12px] text-vb-loss">{status.policy.conflict}</p>}
        {status?.bridge_error && <p className="mt-2 text-[12px] text-vb-silver-dim">{status.bridge_error}</p>}
        {error && <p className="mt-2 text-[12px] text-vb-loss">{error}</p>}
        {message && <p className="mt-2 text-[12px] text-vb-silver">{message}</p>}
      </div>
    </section>
  );
}
