import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, ArrowRight, ArrowUpDown, Loader2 } from "lucide-react";
import { Button } from "../components/Button";
import { CountryFlag } from "../components/CountryFlag";
import { WizardFooter, WizardTitle } from "../components/WizardShell";
import { checkProxy } from "../lib/api";
import { isParseReject, parseProxyInput, type ParsedProxy } from "../lib/proxyParse";
import type { ProxyCheckResult } from "../lib/types";
import { cn } from "../lib/cn";

/** Инструкция «Свой прокси» на сайте проекта. */
const OWN_PROXY_GUIDE = "https://iimperium-dev.github.io/ClodHelper/proxy/";

interface Props {
  initialUrl: string | null;
  onNext: (url: string) => void;
  /** «Сначала очистка»: открыть программу на «Инструментах» без прокси. */
  onSkipToTools?: () => void;
}

const fade = {
  initial: { opacity: 0 },
  animate: { opacity: 1 },
  exit: { opacity: 0 },
  transition: { duration: 0.16 },
};

/*
  Шаг 1: умный ввод прокси.
  Парсер — тот же parseProxyInput, что в разделе «Прокси» (любые форматы
  шопов, эскалация при неоднозначности). Гео-проверка информативная, НЕ
  блокирует «Дальше»: настоящая проверка соединения — на шаге настройки.
  Enter в поле = «Дальше». Ссылки «Нет прокси?» и «Сначала очистка» — тихо
  в подвале слева, чтобы не спорить с полем ввода.
*/
export function StepProxy({ initialUrl, onNext, onSkipToTools }: Props) {
  const [input, setInput] = useState(initialUrl ?? "");
  const [swapped, setSwapped] = useState(false);

  const parsedResult = useMemo(() => parseProxyInput(input), [input]);
  const parseError = isParseReject(parsedResult) ? parsedResult.error : null;
  const parsedBase: ParsedProxy | null = isParseReject(parsedResult) ? null : parsedResult;
  const parsed: ParsedProxy | null =
    parsedBase && swapped && parsedBase.alt ? parsedBase.alt : parsedBase;
  const ambiguous = !!parsedBase?.alt;
  const needsReview = ambiguous || parsed?.confidence === "low";

  // Гео-проверка с дебаунсом; устаревший ответ не применяем.
  const [check, setCheck] = useState<
    { state: "idle" } | { state: "checking" } | { state: "done"; result: ProxyCheckResult | null }
  >({ state: "idle" });
  const checkUrl = useRef<string | null>(null);
  useEffect(() => {
    if (!parsed) {
      checkUrl.current = null;
      setCheck({ state: "idle" });
      return;
    }
    const url = parsed.url;
    checkUrl.current = url;
    setCheck({ state: "checking" });
    const t = window.setTimeout(() => {
      checkProxy(url)
        .then((r) => {
          if (checkUrl.current === url) setCheck({ state: "done", result: r });
        })
        .catch(() => {
          if (checkUrl.current === url) setCheck({ state: "done", result: null });
        });
    }, 700);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [parsed?.url]);

  const geo = check.state === "done" ? check.result : null;
  const isRu = geo?.country_code?.toUpperCase() === "RU";
  const submit = () => parsed && onNext(parsed.url);
  const hasInput = !!input.trim();

  return (
    <>
      <WizardTitle sub="Строку от продавца — целиком, в любом формате. Дальше всё настроится само.">
        Вставьте прокси
      </WizardTitle>

      <div className="mt-5">
        <label htmlFor="wizard-proxy" className="sr-only">
          Строка прокси
        </label>
        <input
          id="wizard-proxy"
          type="text"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          value={input}
          onChange={(e) => {
            setInput(e.target.value);
            setSwapped(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
          aria-invalid={hasInput && !parsed}
          aria-describedby="wizard-proxy-result"
          placeholder="1.2.3.4:8000:логин:пароль"
          className={cn(
            "w-full rounded-xl border bg-vb-surface px-4 py-3 font-mono text-[14px] text-vb-fg outline-none",
            "placeholder:text-vb-silver-faint transition-colors duration-150",
            hasInput && !parsed
              ? "border-vb-warn/50 focus:border-vb-warn/70"
              : "border-vb-border hover:border-vb-border-strong focus:border-vb-emerald/60",
          )}
          autoFocus
        />
      </div>

      {/* Разбор строки — живой ответ под полем, объявляется читалкой. */}
      <div id="wizard-proxy-result" aria-live="polite" className="mt-3 min-h-[20px]">
        {hasInput &&
          (parsed ? (
            <div
              className={cn(
                "rounded-xl border px-4 py-3",
                needsReview ? "border-vb-warn/35 bg-vb-warn/[0.05]" : "border-vb-border bg-vb-surface",
              )}
            >
              {needsReview && (
                <p className="mb-2 text-[12px] text-vb-warn">
                  {ambiguous
                    ? "Формат неоднозначный — проверьте, где адрес, а где логин"
                    : "Не до конца уверен в разборе — проверьте поля"}
                </p>
              )}
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-[12px] sm:grid-cols-[auto_1fr_auto_1fr]">
                <dt className="text-vb-silver-faint">Адрес</dt>
                <dd className="truncate font-mono text-vb-fg">{parsed.host}</dd>
                <dt className="text-vb-silver-faint">Порт</dt>
                <dd className="tnum font-mono text-vb-fg">{parsed.port}</dd>
                {parsed.username !== undefined && (
                  <>
                    <dt className="text-vb-silver-faint">Логин</dt>
                    <dd className="truncate font-mono text-vb-fg">{parsed.username}</dd>
                    <dt className="text-vb-silver-faint">Пароль</dt>
                    <dd>
                      {parsed.password ? (
                        <span className="font-mono text-vb-fg">····</span>
                      ) : (
                        <span className="text-vb-warn">пусто</span>
                      )}
                    </dd>
                  </>
                )}
              </dl>
              {ambiguous && (
                <button
                  type="button"
                  onClick={() => setSwapped((v) => !v)}
                  className="mt-2.5 flex items-center gap-1.5 rounded-lg border border-vb-border px-2.5 py-1 text-[12px] font-medium text-vb-silver transition-[background-color,border-color,transform] duration-150 hover:border-vb-border-strong hover:bg-vb-surface-2 active:scale-[0.97]"
                >
                  <ArrowUpDown className="h-3 w-3" aria-hidden />
                  Поменять местами
                </button>
              )}

              {/* Гео-строка: информация, не блокировка */}
              <div className="mt-2.5 flex min-h-[20px] items-center border-t border-vb-border/60 pt-2.5">
                <AnimatePresence mode="wait" initial={false}>
                  {check.state === "checking" ? (
                    <motion.div key="checking" {...fade} className="flex items-center gap-2 text-[12px] text-vb-silver-dim">
                      <Loader2 className="h-3 w-3 animate-spin" aria-hidden />
                      Проверяю, где он находится…
                    </motion.div>
                  ) : geo?.country_code ? (
                    <motion.div key="geo" {...fade} className="flex items-center gap-2 text-[12px] text-vb-silver">
                      <CountryFlag code={geo.country_code} />
                      {geo.country_name ?? geo.country_code}
                      {geo.latency_ms != null && (
                        <span className="tnum text-vb-silver-faint">· {geo.latency_ms} мс</span>
                      )}
                    </motion.div>
                  ) : check.state === "done" ? (
                    <motion.div key="nogeo" {...fade} className="text-[12px] text-vb-silver-dim">
                      Быстрая проверка не ответила — не страшно, настоящая будет на следующем шаге
                    </motion.div>
                  ) : null}
                </AnimatePresence>
              </div>
              {isRu && (
                <div className="mt-2 flex items-start gap-2 rounded-lg border border-vb-warn/30 bg-vb-warn/5 px-3 py-2 text-[12px] text-vb-warn">
                  <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                  Этот прокси в России. Claude его не примет — нужен зарубежный.
                </div>
              )}
            </div>
          ) : parseError ? (
            <p className="text-[12px] text-vb-warn">{parseError}</p>
          ) : (
            <p className="text-[12px] text-vb-warn">
              Не получилось разобрать. Подойдёт, например,{" "}
              <span className="font-mono text-vb-silver">1.2.3.4:8000:login:pass</span> или{" "}
              <span className="font-mono text-vb-silver">login:pass@1.2.3.4:8000</span>
            </p>
          ))}
      </div>

      <WizardFooter
        left={
          <>
            <button
              type="button"
              onClick={() =>
                import("@tauri-apps/plugin-opener").then((m) => m.openUrl(OWN_PROXY_GUIDE))
              }
              className="rounded text-[12px] text-vb-silver-dim transition-colors hover:text-vb-silver"
            >
              Нет прокси? <span className="text-vb-emerald">Свой за 15 минут</span>
            </button>
            {onSkipToTools && (
              <button
                type="button"
                onClick={onSkipToTools}
                className="rounded text-[12px] text-vb-silver-dim transition-colors hover:text-vb-silver"
              >
                Был другой аккаунт Claude? <span className="text-vb-emerald">Сначала очистка</span>
              </button>
            )}
          </>
        }
      >
        <Button onClick={submit} disabled={!parsed}>
          Дальше
          <ArrowRight className="h-4 w-4" aria-hidden />
        </Button>
      </WizardFooter>
    </>
  );
}
