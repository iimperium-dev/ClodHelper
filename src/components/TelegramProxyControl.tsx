import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "./Button";
import { telegramSetup, telegramStatus } from "../lib/api";
import type { TelegramStatus } from "../lib/types";

/** После «Настроить» пользователь подтверждает прокси в самом Telegram —
    следим за соединениями чаще, пока не увидим их через мост. */
const WATCH_INTERVAL_MS = 3_000;
const WATCH_TICKS = 20;

/*
  Telegram для macOS умеет только SOCKS5/MTProto-прокси — не HTTP и не
  системные переменные. Мост держит для него SOCKS-вход 127.0.0.1:8891
  (только через ваш прокси). Настройка — штатное окно Telegram «Добавить
  прокси» по ссылке tg://socks; статус — по живым соединениям процесса.
*/
export function TelegramProxyControl() {
  const [status, setStatus] = useState<TelegramStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hint, setHint] = useState<string | null>(null);
  const watch = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    try {
      const st = await telegramStatus();
      setStatus(st);
      return st;
    } catch (e) {
      setError(String(e));
      return null;
    }
  }, []);

  const stopWatch = () => {
    if (watch.current !== null) window.clearInterval(watch.current);
    watch.current = null;
  };

  useEffect(() => {
    void refresh();
    return stopWatch;
  }, [refresh]);

  const setup = async () => {
    setBusy(true);
    setError(null);
    setHint(null);
    try {
      await telegramSetup();
      setHint("В Telegram откроется окно «Прокси» — нажмите «Подключить» («Connect»).");
      stopWatch();
      let ticks = 0;
      watch.current = window.setInterval(async () => {
        const st = await refresh();
        if (++ticks >= WATCH_TICKS || (st && st.via_bridge > 0 && st.direct === 0)) {
          stopWatch();
          if (st && st.via_bridge > 0) setHint(null);
        }
      }, WATCH_INTERVAL_MS);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!status?.supported) return null;

  const installed = status.app_path !== null;
  const viaBridge = status.via_bridge > 0;
  const leaking = status.direct > 0;
  const state = !installed
    ? "Не установлен"
    : !status.running
      ? "Не запущен — откройте Telegram, чтобы проверить"
      : viaBridge && !leaking
        ? `Через мост · соединений: ${status.via_bridge}`
        : viaBridge && leaking
          ? `Частично через мост (${status.via_bridge}), напрямую: ${status.direct}`
          : leaking
            ? "Напрямую, мимо моста"
            : "Нет активных соединений";
  const stateTone =
    viaBridge && !leaking ? "text-vb-emerald" : leaking ? "text-vb-warn" : "text-vb-silver";

  return (
    <section
      className="mt-6 border-t border-vb-border pt-5"
      aria-labelledby="telegram-proxy-title"
      aria-busy={busy}
    >
      <h2 id="telegram-proxy-title" className="text-[15px] font-semibold text-vb-fg">
        Telegram
      </h2>
      <p className="mt-1 text-[13px] leading-relaxed text-vb-silver-dim">
        Telegram не читает системный прокси, поэтому мост держит для него отдельный
        SOCKS5-вход. Всё, что туда приходит, идёт только через ваш прокси.
      </p>
      <dl className="my-3 space-y-1.5 text-[12px]">
        <div className="flex flex-wrap justify-between gap-x-4 gap-y-1">
          <dt className="text-vb-silver-dim">Сейчас</dt>
          <dd className={stateTone}>{state}</dd>
        </div>
        <div className="flex flex-wrap justify-between gap-x-4 gap-y-1">
          <dt className="text-vb-silver-dim">Вход моста</dt>
          <dd className="font-mono text-vb-silver">
            {status.socks_ready ? "127.0.0.1:8891 · SOCKS5" : "нет — обновите мост в Обзоре"}
          </dd>
        </div>
      </dl>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="secondary"
          disabled={busy || !installed || !status.socks_ready}
          onClick={setup}
        >
          {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />}
          {viaBridge ? "Настроить заново" : "Настроить Telegram"}
        </Button>
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => void refresh()}>
          Проверить
        </Button>
      </div>
      <p className="mt-2 text-[11px] leading-relaxed text-vb-silver-dim">
        Звонки Telegram через прокси не пойдут: мост передаёт только TCP. Отключить —
        в Telegram: Настройки → Данные и память → Прокси.
      </p>
      <div aria-live="polite">
        {error && <p className="mt-2 text-[12px] text-vb-loss">{error}</p>}
        {hint && <p className="mt-2 text-[12px] text-vb-silver">{hint}</p>}
      </div>
    </section>
  );
}
