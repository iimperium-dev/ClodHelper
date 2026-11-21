import { useMemo, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertCircle, ArrowUpDown, Check, ClipboardPaste, KeyRound, Link2, X } from "lucide-react";
import { isParseReject, parseProxyInput, type ParsedProxy } from "../lib/proxyParse";
import { cn } from "../lib/cn";

/** Состояние поля «прокси в любом формате»: ввод, разбор, «поменять местами». */
export function useProxyInput() {
  const [value, setRaw] = useState("");
  const [swapped, setSwapped] = useState(false);
  const result = useMemo(() => parseProxyInput(value), [value]);
  const error = isParseReject(result) ? result.error : null;
  const base: ParsedProxy | null = isParseReject(result) ? null : result;
  const parsed: ParsedProxy | null = base && swapped && base.alt ? base.alt : base;
  return {
    value,
    setValue: (v: string) => {
      setRaw(v);
      setSwapped(false);
    },
    reset: () => {
      setRaw("");
      setSwapped(false);
    },
    toggleSwap: () => setSwapped((v) => !v),
    parsed,
    /** Есть второй вариант разбора - пользователь должен выбрать. */
    ambiguous: !!base?.alt,
    error,
  };
}

export type ProxyInputState = ReturnType<typeof useProxyInput>;

/*
  Поле «Вставьте прокси в любом формате» с разбором на лету: хост, порт,
  логин, пароль; неоднозначный формат - жёлтая подсказка и «Поменять
  местами». Слева значок (зеленеет, когда адрес разобран), справа -
  «Вставить» из буфера или «очистить». Разбор - таблетками, по одной.
  Одно и то же для пула и для резервов.
*/
export function ProxyInputField({
  input,
  autoFocus,
  onEnter,
  compact,
}: {
  input: ProxyInputState;
  autoFocus?: boolean;
  onEnter?: () => void;
  /** Внутри строки прокси - поменьше. */
  compact?: boolean;
}) {
  const { value, parsed, ambiguous, error } = input;
  const ref = useRef<HTMLInputElement>(null);
  const needsReview = ambiguous || parsed?.confidence === "low";
  const has = value.trim().length > 0;
  const state: "idle" | "ok" | "review" | "bad" = !has ? "idle" : parsed ? (needsReview ? "review" : "ok") : "bad";
  const scheme = parsed ? (parsed.url.split("://")[0] === "socks5" ? "SOCKS5" : "HTTP") : "";

  const paste = async () => {
    try {
      const t = await navigator.clipboard.readText();
      if (t.trim()) input.setValue(t.trim());
    } catch {
      /* буфер недоступен - пользователь вставит сам */
    }
    ref.current?.focus();
  };

  return (
    <div>
      <div
        className={cn(
          "flex items-center gap-2 rounded-xl border bg-vb-bg/50 pl-3 pr-1 transition-[border-color,box-shadow] duration-200",
          state === "bad"
            ? "focus-within:shadow-[0_0_0_4px_rgba(239,68,68,0.14)]"
            : state === "review"
              ? "focus-within:shadow-[0_0_0_4px_rgba(234,179,8,0.14)]"
              : "focus-within:shadow-[0_0_0_4px_var(--color-vb-emerald-glow)]",
          state === "ok"
            ? "border-vb-emerald/45"
            : state === "review"
              ? "border-vb-warn/45"
              : state === "bad"
                ? "border-vb-loss/40"
                : "border-vb-border focus-within:border-vb-emerald/50",
        )}
      >
        <span className="relative flex h-4 w-4 shrink-0 items-center justify-center">
          <AnimatePresence initial={false} mode="popLayout">
            <motion.span
              key={state === "ok" ? "ok" : "idle"}
              initial={{ scale: 0.4, opacity: 0, rotate: -30 }}
              animate={{ scale: 1, opacity: 1, rotate: 0 }}
              exit={{ scale: 0.4, opacity: 0 }}
              transition={{ type: "spring", bounce: 0.45, duration: 0.4 }}
              className="flex"
            >
              {state === "ok" ? (
                <Check className="h-4 w-4 text-vb-emerald" strokeWidth={2.5} />
              ) : (
                <Link2 className={cn("h-4 w-4", state === "idle" ? "text-vb-silver-faint" : "text-vb-silver-dim")} />
              )}
            </motion.span>
          </AnimatePresence>
        </span>
        <input
          ref={ref}
          type="text"
          value={value}
          onChange={(e) => input.setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && parsed) onEnter?.();
          }}
          placeholder="Вставьте прокси в любом формате"
          aria-label="Адрес прокси"
          className={cn(
            "min-w-0 flex-1 bg-transparent font-mono text-vb-fg outline-none placeholder:font-sans placeholder:text-vb-silver-faint",
            compact ? "py-2 text-[12px]" : "py-2.5 text-[13px]",
          )}
          autoFocus={autoFocus}
          spellCheck={false}
          autoComplete="off"
        />
        {has ? (
          <button
            type="button"
            onClick={() => {
              input.reset();
              ref.current?.focus();
            }}
            aria-label="Очистить"
            title="Очистить"
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.94]"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        ) : (
          <button
            type="button"
            onClick={() => void paste()}
            className="flex shrink-0 items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px] font-medium text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-fg active:scale-[0.96]"
          >
            <ClipboardPaste className="h-3.5 w-3.5" />
            Вставить
          </button>
        )}
      </div>

      <AnimatePresence initial={false} mode="wait">
        {has && (
          <motion.div
            key={state === "bad" ? "bad" : "parsed"}
            initial={{ opacity: 0, y: -4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
            className="mt-2.5"
          >
            {parsed ? (
              <>
                {needsReview && (
                  <p className="mb-2 flex items-center gap-1.5 text-[12px] text-vb-warn">
                    <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                    {ambiguous ? "Формат неоднозначный - проверьте, где хост, а где логин" : "Не до конца уверен в разборе - проверьте"}
                  </p>
                )}
                <div className="flex flex-wrap items-center gap-1.5">
                  <Pill k={0} accent>
                    {scheme}
                  </Pill>
                  <Pill k={1} mono>
                    {parsed.host}
                    <span className="text-vb-silver-faint">:</span>
                    {parsed.port}
                  </Pill>
                  {parsed.username !== undefined && (
                    <Pill k={2}>
                      <KeyRound className="h-3 w-3 text-vb-silver-faint" />
                      <span className="max-w-[140px] truncate font-mono">{parsed.username}</span>
                      {parsed.password ? (
                        <span className="font-mono text-vb-silver-faint">· ••••</span>
                      ) : (
                        <span className="text-vb-warn">· без пароля</span>
                      )}
                    </Pill>
                  )}
                  {ambiguous && (
                    <motion.button
                      type="button"
                      initial={{ opacity: 0, scale: 0.9 }}
                      animate={{ opacity: 1, scale: 1 }}
                      transition={{ delay: 0.15 }}
                      onClick={input.toggleSwap}
                      className="flex items-center gap-1.5 rounded-full border border-vb-warn/40 px-2.5 py-1 text-[11px] font-medium text-vb-warn transition-colors hover:bg-vb-warn/10 active:scale-[0.96]"
                    >
                      <ArrowUpDown className="h-3 w-3" />
                      Поменять местами
                    </motion.button>
                  )}
                </div>
              </>
            ) : error ? (
              <p className="flex items-center gap-1.5 text-[12px] text-vb-warn">
                <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                {error}
              </p>
            ) : (
              <p className="text-[12px] leading-relaxed text-vb-silver-faint">
                <span className="text-vb-loss">Не получилось разобрать.</span> Например:{" "}
                <span className="font-mono text-vb-silver-dim">1.2.3.4:8000:login:pass</span> или{" "}
                <span className="font-mono text-vb-silver-dim">login:pass@1.2.3.4:8000</span>
              </p>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Таблетка разбора - появляются по очереди. */
function Pill({ children, k, mono, accent }: { children: ReactNode; k: number; mono?: boolean; accent?: boolean }) {
  return (
    <motion.span
      initial={{ opacity: 0, y: 4, scale: 0.96 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ type: "spring", bounce: 0.3, duration: 0.4, delay: k * 0.05 }}
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2.5 py-1 text-[11px] leading-none",
        accent
          ? "border-vb-emerald/30 bg-vb-emerald/[0.08] font-semibold tracking-wide text-vb-emerald"
          : "border-vb-border bg-vb-surface-2/70 text-vb-fg",
        mono && "font-mono",
      )}
    >
      {children}
    </motion.span>
  );
}
