import { Terminal, MessageSquare, MousePointer2, Check } from "lucide-react";
import { Button } from "../components/Button";
import type { InstallMode } from "../lib/types";
import { cn } from "../lib/cn";

interface Props {
  value: InstallMode | null;
  cursor: boolean;
  onChange: (v: InstallMode | null) => void;
  onCursorChange: (v: boolean) => void;
  onBack: () => void;
  onNext: () => void;
}

export function StepMode({
  value,
  cursor,
  onChange,
  onCursorChange,
  onBack,
  onNext,
}: Props) {
  const items: {
    key: InstallMode;
    icon: React.ReactNode;
    title: string;
    body: string;
  }[] = [
    {
      key: "code",
      icon: <Terminal className="h-5 w-5" strokeWidth={1.5} />,
      title: "Claude в терминале и VS Code",
      body: "Для программистов и автоматизации. Команда `claude` в командной строке.",
    },
    {
      key: "desktop",
      icon: <MessageSquare className="h-5 w-5" strokeWidth={1.5} />,
      title: "Claude Desktop",
      body: "Обычное приложение с чатом — как Telegram, только Claude.",
    },
    {
      key: "both",
      icon: (
        <div className="flex">
          <Terminal className="h-5 w-5" strokeWidth={1.5} />
          <MessageSquare
            className="-ml-1 h-5 w-5"
            strokeWidth={1.5}
          />
        </div>
      ),
      title: "И то, и другое",
      body: "Поставим обе версии. Прокси будет общий.",
    },
  ];
  return (
    <div className="mx-auto max-w-2xl pt-4">
      <h2 className="text-[22px] font-semibold tracking-tight text-vb-silver">
        Что вам нужно?
      </h2>
      <p className="mt-1.5 text-[13px] text-vb-silver-dim">
        Можно выбрать одно из трёх — потом переустановка не нужна, можно
        добавить вторую часть позже.
      </p>

      <div className="mt-7 space-y-3">
        {items.map((it) => {
          const selected = value === it.key;
          return (
            <button
              key={it.key}
              onClick={() => onChange(selected ? null : it.key)}
              className={cn(
                "block w-full rounded-[10px] border p-5 text-left transition-all",
                selected
                  ? "border-vb-emerald bg-vb-emerald/5 shadow-[0_0_0_1px_rgba(16,185,129,0.4)]"
                  : "border-vb-border bg-vb-surface/40 hover:border-vb-silver-dim",
              )}
            >
              <div className="flex items-start gap-4">
                <div
                  className={cn(
                    "flex h-10 w-10 shrink-0 items-center justify-center rounded-lg",
                    selected
                      ? "bg-vb-emerald/15 text-vb-emerald"
                      : "bg-vb-bg text-vb-silver-dim",
                  )}
                >
                  {it.icon}
                </div>
                <div className="flex-1">
                  <div className="text-[15px] font-medium text-vb-silver">
                    {it.title}
                  </div>
                  <div className="mt-1 text-[13px] leading-relaxed text-vb-silver-dim">
                    {it.body}
                  </div>
                </div>
                <div
                  className={cn(
                    "mt-0.5 h-4 w-4 shrink-0 rounded-full border transition-colors",
                    selected
                      ? "border-vb-emerald bg-vb-emerald"
                      : "border-vb-border",
                  )}
                />
              </div>
            </button>
          );
        })}
      </div>

      <div className="mt-6 flex items-center gap-3">
        <div className="h-px flex-1 bg-vb-border" />
        <span className="text-[11px] font-medium uppercase tracking-[0.14em] text-vb-silver-dim/70">
          Редактор кода
        </span>
        <div className="h-px flex-1 bg-vb-border" />
      </div>

      <button
        onClick={() => onCursorChange(!cursor)}
        className={cn(
          "mt-4 block w-full rounded-[10px] border p-5 text-left transition-all",
          cursor
            ? "border-vb-emerald bg-vb-emerald/5 shadow-[0_0_0_1px_rgba(16,185,129,0.4)]"
            : "border-vb-border bg-vb-surface/40 hover:border-vb-silver-dim",
        )}
      >
        <div className="flex items-start gap-4">
          <div
            className={cn(
              "flex h-10 w-10 shrink-0 items-center justify-center rounded-lg",
              cursor
                ? "bg-vb-emerald/15 text-vb-emerald"
                : "bg-vb-bg text-vb-silver-dim",
            )}
          >
            <MousePointer2 className="h-5 w-5" strokeWidth={1.5} />
          </div>
          <div className="flex-1">
            <div className="text-[15px] font-medium text-vb-silver">
              Cursor IDE через прокси
            </div>
            <div className="mt-1 text-[13px] leading-relaxed text-vb-silver-dim">
              Добавим ярлык «Cursor (proxy)» на рабочий стол — Cursor пойдёт
              через тот же мост. Можно вместе с Claude или отдельно.
            </div>
          </div>
          <div
            className={cn(
              "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-[5px] border transition-colors",
              cursor
                ? "border-vb-emerald bg-vb-emerald text-black"
                : "border-vb-border",
            )}
          >
            {cursor && <Check className="h-3 w-3" strokeWidth={3} />}
          </div>
        </div>
      </button>

      <div className="mt-8 flex items-center justify-between">
        <Button variant="ghost" onClick={onBack}>
          Назад
        </Button>
        <Button onClick={onNext} disabled={!value && !cursor}>
          Дальше
        </Button>
      </div>
    </div>
  );
}
