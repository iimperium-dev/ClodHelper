import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, ArrowUpDown, Loader2 } from "lucide-react";
import { Button } from "../components/Button";
import { CountryFlag } from "../components/CountryFlag";
import { checkProxy } from "../lib/api";
import { isParseReject, parseProxyInput, type ParsedProxy } from "../lib/proxyParse";
import type { ProxyCheckResult } from "../lib/types";
import { cn } from "../lib/cn";

interface Props {
  initialUrl: string | null;
  onNext: (url: string) => void;
}

/*
  Шаг 1 нового мастера: приветствие + умный ввод прокси.
  Парсер — тот же parseProxyInput, что в разделе «Прокси» (любые форматы
  шопов, эскалация при неоднозначности). Гео-проверка информативная, НЕ
  блокирует «Дальше»: настоящая проверка соединения будет на шаге установки.
*/
export function StepProxy({ initialUrl, onNext }: Props) {
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

  return (
    <div className="mx-auto max-w-2xl pt-4">
      <h2 className="text-[22px] font-semibold tracking-tight text-vb-silver">
        Вставьте прокси
      </h2>
      <p className="mt-1.5 text-[13px] text-vb-silver-dim">
        Скопируйте строку целиком из письма продавца — формат любой, мы сами
        разберём её на части. Дальше всё настроится автоматически.
      </p>

      <button
        type="button"
        onClick={() =>
          import("@tauri-apps/plugin-opener").then((m) =>
            m.openUrl("https://proxy6.net/?r=692907"),
          )
        }
        className="mt-3 inline-flex items-center gap-1.5 text-[12px] text-vb-silver-dim transition-colors hover:text-vb-silver"
      >
        Где купить прокси?
        <span className="text-vb-emerald/80">proxy6.net →</span>
      </button>

      <div className="surface-card mt-5 p-5">
        <input
          type="text"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          value={input}
          onChange={(e) => {
            setInput(e.target.value);
            setSwapped(false);
          }}
          placeholder="Например: 1.2.3.4:8000:логин:пароль"
          className="w-full rounded-lg border border-vb-border bg-vb-surface-2 px-3.5 py-2.5 font-mono text-[13px] text-vb-fg outline-none transition-colors focus:border-vb-emerald/60"
          autoFocus
        />

        {input.trim() &&
          (parsed ? (
            <div
              className={cn(
                "mt-3 rounded-lg border p-3",
                needsReview
                  ? "border-vb-warn/35 bg-vb-warn/[0.05]"
                  : "border-vb-emerald/25 bg-vb-emerald/[0.04]",
              )}
            >
              {needsReview && (
                <p className="mb-2 text-[12px] text-vb-warn">
                  {ambiguous
                    ? "Формат неоднозначный — проверьте, где хост, а где логин:"
                    : "Не до конца уверен в разборе — проверьте поля:"}
                </p>
              )}
              <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px]">
                <span className="text-vb-silver-dim">
                  Хост <span className="font-mono text-vb-fg">{parsed.host}</span>
                </span>
                <span className="text-vb-silver-dim">
                  Порт <span className="tnum font-mono text-vb-fg">{parsed.port}</span>
                </span>
                {parsed.username !== undefined && (
                  <>
                    <span className="text-vb-silver-dim">
                      Логин <span className="font-mono text-vb-fg">{parsed.username}</span>
                    </span>
                    <span className="text-vb-silver-dim">
                      Пароль{" "}
                      {parsed.password ? (
                        <span className="font-mono text-vb-fg">····</span>
                      ) : (
                        <span className="text-vb-warn">пусто</span>
                      )}
                    </span>
                  </>
                )}
              </div>
              {ambiguous && (
                <button
                  type="button"
                  onClick={() => setSwapped((v) => !v)}
                  className="mt-2 flex items-center gap-1.5 rounded-lg border border-vb-border px-2.5 py-1 text-[12px] font-medium text-vb-silver transition-colors hover:border-vb-border-strong hover:bg-vb-surface-2 active:scale-[0.97]"
                >
                  <ArrowUpDown className="h-3 w-3" />
                  Поменять местами
                </button>
              )}

              {/* Гео-строка: информация, не блокировка */}
              <div className="mt-2.5 border-t border-vb-border/50 pt-2.5">
                <AnimatePresence mode="wait">
                  {check.state === "checking" ? (
                    <motion.div
                      key="checking"
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      className="flex items-center gap-2 text-[12px] text-vb-silver-dim"
                    >
                      <Loader2 className="h-3 w-3 animate-spin" />
                      Проверяю доступность…
                    </motion.div>
                  ) : geo?.country_code ? (
                    <motion.div
                      key="geo"
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      className="flex items-center gap-2 text-[12px] text-vb-silver"
                    >
                      <CountryFlag code={geo.country_code} />
                      {geo.country_name ?? geo.country_code}
                      {geo.latency_ms != null && (
                        <span className="tnum text-vb-silver-faint">
                          · {geo.latency_ms} мс
                        </span>
                      )}
                    </motion.div>
                  ) : check.state === "done" ? (
                    <motion.div
                      key="nogeo"
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      className="text-[12px] text-vb-silver-dim"
                    >
                      Быстрая проверка не прошла — не страшно: настоящая
                      проверка будет на следующем шаге.
                    </motion.div>
                  ) : null}
                </AnimatePresence>
                {isRu && (
                  <div className="mt-2 flex items-start gap-2 rounded-lg border border-vb-warn/30 bg-vb-warn/5 px-3 py-2 text-[12px] text-vb-warn">
                    <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                    Этот прокси в России. Claude его не примет — нужен зарубежный.
                  </div>
                )}
              </div>
            </div>
          ) : parseError ? (
            <p className="mt-2.5 text-[12px] text-vb-warn">{parseError}</p>
          ) : (
            <p className="mt-2.5 text-[12px] text-vb-loss">
              Не получилось разобрать. Примеры:{" "}
              <span className="font-mono">1.2.3.4:8000:login:pass</span>,{" "}
              <span className="font-mono">login:pass@1.2.3.4:8000</span>
            </p>
          ))}
      </div>

      <div className="mt-8 flex items-center justify-end">
        <Button onClick={() => parsed && onNext(parsed.url)} disabled={!parsed}>
          Дальше
        </Button>
      </div>
    </div>
  );
}
