import { useMemo, useState } from "react";
import { ArrowUpDown } from "lucide-react";
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
    /** Есть второй вариант разбора — пользователь должен выбрать. */
    ambiguous: !!base?.alt,
    error,
  };
}

export type ProxyInputState = ReturnType<typeof useProxyInput>;

/*
  Поле «Вставьте прокси в любом формате» с разбором на лету: хост, порт,
  логин, пароль; неоднозначный формат — жёлтая панель и «Поменять местами».
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
  /** Внутри строки прокси — поменьше. */
  compact?: boolean;
}) {
  const { value, parsed, ambiguous, error } = input;
  const needsReview = ambiguous || parsed?.confidence === "low";
  return (
    <>
      <input
        type="text"
        value={value}
        onChange={(e) => input.setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && parsed) onEnter?.();
        }}
        placeholder="Вставьте прокси в любом формате"
        className={cn(
          "w-full rounded-lg border border-vb-border bg-vb-surface-2 px-3 font-mono text-vb-fg outline-none transition-colors focus:border-vb-emerald/60",
          compact ? "py-1.5 text-[12px]" : "py-2 text-[13px]",
        )}
        autoFocus={autoFocus}
        spellCheck={false}
        autoComplete="off"
      />

      {value.trim() &&
        (parsed ? (
          <div
            className={cn(
              "mt-2.5 rounded-lg border p-3",
              needsReview ? "border-vb-warn/35 bg-vb-warn/[0.05]" : "border-vb-emerald/25 bg-vb-emerald/[0.04]",
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
                onClick={input.toggleSwap}
                className="mt-2 flex items-center gap-1.5 rounded-lg border border-vb-border px-2.5 py-1 text-[12px] font-medium text-vb-silver transition-colors hover:border-vb-border-strong hover:bg-vb-surface-2 active:scale-[0.97]"
              >
                <ArrowUpDown className="h-3 w-3" />
                Поменять местами
              </button>
            )}
          </div>
        ) : error ? (
          <p className="mt-2 text-[12px] text-vb-warn">{error}</p>
        ) : (
          <p className="mt-2 text-[12px] text-vb-loss">
            Не получилось разобрать. Примеры: <span className="font-mono">1.2.3.4:8000:login:pass</span>,{" "}
            <span className="font-mono">login:pass@1.2.3.4:8000</span>
          </p>
        ))}
    </>
  );
}
