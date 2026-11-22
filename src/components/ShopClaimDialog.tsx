import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Check, Loader2, ShoppingBag } from "lucide-react";
import { Button } from "./Button";
import { CountryFlag } from "./CountryFlag";
import { checkProxy, proxiesGet, shopAdd, shopRemember, type ShopAddItem } from "../lib/api";
import { useStatus } from "../lib/status";
import { loadGeoCache, saveGeoCache } from "../pages/Proxies";
import {
  type Claim,
  countryCode,
  countryName,
  expiryText,
  hostPortOf,
  normalizeUrl,
  POOL_CHANGED_EVENT,
} from "../lib/shopClaim";
import type { ProxyCheckResult } from "../lib/types";

/** Что окно показывает: данные ещё идут, магазин ответил ошибкой, или покупка. */
export type ShopClaimState =
  | { kind: "loading" }
  | { kind: "error"; message: string; /** Повтор не поможет (битая ссылка). */ final?: boolean }
  | { kind: "ready"; claim: Claim; key: string };

interface Props {
  state: ShopClaimState | null;
  /**
   * Идёт первая настройка: «Добавить» подставляет прокси в поле мастера,
   * а не пишет в пул (моста ещё нет).
   */
  wizard: boolean;
  onWizardFill: (url: string) => void;
  onRetry: () => void;
  onClose: () => void;
  /** «К прокси» после добавления (нет - кнопки нет). */
  onOpenProxies?: () => void;
}

type ProxyCheck = { state: "checking" } | { state: "done"; result: ProxyCheckResult | null };

/*
  «Добавить купленный прокси?» - ссылка со страницы оплаты открыла программу.
  Ссылка сама ничего не меняет: прокси попадает в пул только по кнопке
  «Добавить», так же как ручное добавление на странице «Прокси», и сразу
  проверяется. Тот же адрес уже есть - запись обновляется, а не дублируется.
  Пароль в окне не показывается.
*/
export function ShopClaimDialog({ state, wizard, onWizardFill, onRetry, onClose, onOpenProxies }: Props) {
  const reduce = useReducedMotion();
  const { refresh, refreshPoolPing, proxies } = useStatus();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pool, setPool] = useState<string[] | null>(null);
  // «Сделать основным»: null - человек не трогал, решает состояние основного.
  const [primaryPick, setPrimaryPick] = useState<boolean | null>(null);
  // После «Добавить»: результат проверки каждого купленного прокси.
  const [checks, setChecks] = useState<ProxyCheck[] | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const yesRef = useRef<HTMLButtonElement>(null);

  const open = state !== null;
  const claim = state?.kind === "ready" ? state.claim : null;

  // Новая покупка - окно с начала.
  useEffect(() => {
    setBusy(false);
    setError(null);
    setChecks(null);
    setPrimaryPick(null);
    if (claim && !wizard) void proxiesGet().then(setPool, () => setPool(null));
  }, [claim, wizard]);

  useEffect(() => {
    if (!open) return;
    const t = window.setTimeout(() => yesRef.current?.focus(), 60);
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
      if (e.key === "Tab" && dialogRef.current) {
        const f = dialogRef.current.querySelectorAll<HTMLElement>("button:not([disabled])");
        if (!f.length) return;
        const first = f[0];
        const last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener("keydown", key);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener("keydown", key);
    };
  }, [open, busy, onClose, state, checks]);

  const urls = claim ? claim.proxies.map((p) => normalizeUrl(p.url)) : [];
  const inPool = (url: string) => !!pool?.some((u) => hostPortOf(u) === hostPortOf(url));
  const items: ShopAddItem[] =
    state?.kind === "ready"
      ? state.claim.proxies.map((p, i) => ({ url: urls[i], label: p.label, key: state.key, expires_at: p.expires_at }))
      : [];

  // Галочка нужна, когда основной есть и это не сам купленный прокси.
  const primaryUrl = pool?.[0];
  const showPrimary =
    !wizard && !!primaryUrl && urls.length > 0 && hostPortOf(primaryUrl) !== hostPortOf(urls[0]);
  // По умолчанию включена, если основной сейчас не работает: иначе покупка
  // во время сбоя не починила бы Claude.
  const primaryDown = proxies.find((p) => p.url === primaryUrl)?.state === "down";
  const makePrimary = showPrimary && (primaryPick ?? primaryDown);

  const add = async () => {
    if (!claim || busy) return;
    if (wizard) {
      // Ключ и срок - заранее: прокси в пул запишет сам мастер.
      await shopRemember(items[0]).catch(() => {});
      onWizardFill(urls[0]);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // Тот же адрес - обновить на месте, имя из магазина - если своего нет.
      await shopAdd(items, makePrimary);
      window.dispatchEvent(new Event(POOL_CHANGED_EVENT));
      // Сразу проверяем, как после ручного добавления.
      setChecks(urls.map(() => ({ state: "checking" })));
      void refresh().catch(() => {});
      void refreshPoolPing().catch(() => {});
      await Promise.all(
        urls.map(async (url, i) => {
          const r = await checkProxy(url).catch(() => null);
          if (r && (r.reachable || r.country_code)) {
            saveGeoCache({ ...loadGeoCache(), [hostPortOf(url)]: r });
          }
          setChecks((old) => old && old.map((c, k) => (k === i ? { state: "done", result: r } : c)));
        }),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const added = checks !== null;
  const many = (claim?.proxies.length ?? 0) > 1;

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          key="shop-claim"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          className="fixed inset-x-0 bottom-0 top-9 z-[90] flex items-center justify-center bg-black/55 p-5 backdrop-blur-[6px]"
        >
          <motion.div
            ref={dialogRef}
            initial={reduce ? { opacity: 0 } : { opacity: 0, y: 14, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: 6, scale: 0.98 }}
            transition={{ type: "spring", bounce: 0, duration: 0.42 }}
            role="dialog"
            aria-modal="true"
            aria-labelledby="shop-claim-title"
            className="relative flex max-h-full w-full max-w-[400px] flex-col overflow-y-auto rounded-2xl border border-vb-border bg-vb-surface px-6 pb-5 pt-5 shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
          >
            <div className="flex items-center gap-3">
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-vb-emerald/10 text-vb-emerald">
                {added ? <Check className="h-4.5 w-4.5" strokeWidth={2.5} aria-hidden /> : <ShoppingBag className="h-4.5 w-4.5" aria-hidden />}
              </span>
              <div className="min-w-0">
                <h2 id="shop-claim-title" className="text-[17px] font-semibold tracking-[-0.01em] text-vb-fg">
                  {added ? (many ? "Прокси добавлены" : "Прокси добавлен") : "Добавить купленный прокси?"}
                </h2>
                {claim?.email_masked && (
                  <p className="truncate text-[12px] text-vb-silver-dim">Покупка на {claim.email_masked}</p>
                )}
              </div>
            </div>

            {state?.kind === "loading" && (
              <div className="mt-5 flex items-center gap-2 text-[13px] text-vb-silver-dim" aria-live="polite">
                <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
                Получаю данные покупки…
              </div>
            )}

            {state?.kind === "error" && (
              <>
                <p className="mt-4 text-[13px] leading-relaxed text-vb-warn" role="alert">
                  {state.message}
                </p>
                <div className="mt-5 flex justify-end gap-2">
                  {state.final ? (
                    <Button ref={yesRef} size="sm" onClick={onClose}>
                      Понятно
                    </Button>
                  ) : (
                    <>
                      <Button variant="ghost" size="sm" onClick={onClose}>
                        Закрыть
                      </Button>
                      <Button ref={yesRef} size="sm" onClick={onRetry}>
                        Повторить
                      </Button>
                    </>
                  )}
                </div>
              </>
            )}

            {claim && (
              <>
                <ul className="mt-4 space-y-2">
                  {claim.proxies.map((p, i) => {
                    const code = countryCode(p.country);
                    const name = countryName(p.country);
                    const until = expiryText(p.expires_at);
                    const c = checks?.[i];
                    return (
                      <li
                        key={i}
                        data-testid="shop-claim-proxy"
                        className="rounded-xl border border-vb-border bg-vb-bg/40 px-3.5 py-2.5"
                      >
                        <div className="flex items-center gap-2 text-[13px] text-vb-fg">
                          {code && <CountryFlag code={code} />}
                          <span className="min-w-0 flex-1 truncate">{name ?? p.label ?? "Прокси"}</span>
                          {!wizard && !added && inPool(urls[i]) && (
                            <span className="shrink-0 text-[11px] text-vb-silver-faint">уже есть, обновится</span>
                          )}
                        </div>
                        <div className="mt-0.5 truncate font-mono text-[12px] text-vb-silver">{hostPortOf(p.url)}</div>
                        {until && <div className="mt-0.5 text-[12px] text-vb-silver-dim">Действует {until}</div>}
                        {c && (
                          <div className="mt-1.5 flex items-center gap-1.5 border-t border-vb-border/60 pt-1.5 text-[12px]" aria-live="polite">
                            {c.state === "checking" ? (
                              <span className="flex items-center gap-1.5 text-vb-silver-dim">
                                <Loader2 className="h-3 w-3 animate-spin" aria-hidden />
                                Проверяю…
                              </span>
                            ) : c.result?.reachable ? (
                              <span className="text-vb-emerald">
                                Работает{c.result.latency_ms != null ? `, ${c.result.latency_ms} мс` : ""}
                              </span>
                            ) : (
                              <span className="text-vb-warn">Пока не отвечает. Состояние видно на странице «Прокси»</span>
                            )}
                          </div>
                        )}
                      </li>
                    );
                  })}
                </ul>

                {showPrimary && !added && (
                  <label className="mt-3 flex cursor-pointer items-start gap-2.5 rounded-lg px-1 py-1 text-[13px] text-vb-silver">
                    <input
                      type="checkbox"
                      checked={makePrimary}
                      onChange={(e) => setPrimaryPick(e.target.checked)}
                      disabled={busy}
                      className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--color-vb-emerald)]"
                    />
                    <span>
                      Сделать основным
                      <span className="block text-[12px] leading-snug text-vb-silver-dim">
                        {primaryDown
                          ? "Основной прокси сейчас не отвечает. Claude и всё, что шло через него, перейдёт на купленный"
                          : "Claude и всё, что шло через основной, перейдёт на купленный"}
                      </span>
                    </span>
                  </label>
                )}
                {wizard && many && (
                  <p className="mt-3 text-[12px] leading-snug text-vb-silver-dim">
                    В мастер подставится первый. Остальные добавьте потом на странице «Прокси».
                  </p>
                )}
                {error && (
                  <p className="mt-3 text-[12px] leading-snug text-vb-warn" role="alert">
                    {error}
                  </p>
                )}

                <div className="mt-5 flex justify-end gap-2">
                  {added ? (
                    <>
                      {onOpenProxies && (
                        <Button variant="ghost" size="sm" onClick={onOpenProxies}>
                          К прокси
                        </Button>
                      )}
                      <Button ref={yesRef} size="sm" onClick={onClose}>
                        Готово
                      </Button>
                    </>
                  ) : (
                    <>
                      <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
                        Не сейчас
                      </Button>
                      <Button ref={yesRef} size="sm" onClick={() => void add()} disabled={busy}>
                        {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />}
                        Добавить
                      </Button>
                    </>
                  )}
                </div>
              </>
            )}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
