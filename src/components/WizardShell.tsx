import { type ReactNode } from "react";
import type { WizardStep } from "../lib/types";
import { cn } from "../lib/cn";

const STEPS: { id: WizardStep; label: string }[] = [
  { id: "welcome", label: "Старт" },
  { id: "mode", label: "Что ставим" },
  { id: "proxy", label: "Прокси" },
  { id: "deps", label: "Проверка" },
  { id: "install", label: "Установка" },
];

interface Props {
  step: WizardStep;
  children: ReactNode;
  footer?: ReactNode;
}

export function WizardShell({ step, children, footer }: Props) {
  const idx = STEPS.findIndex((s) => s.id === step);
  return (
    <div className="flex h-full flex-col bg-vb-bg text-vb-silver">
      <header className="px-10 pt-8 pb-6">
        <div className="flex items-center justify-between">
          <div>
            <div className="text-[11px] uppercase tracking-[0.18em] text-vb-silver-dim">
              Clod Helper
            </div>
            <div className="mt-0.5 text-[13px] text-vb-silver-dim">
              Установка Claude через прокси
            </div>
          </div>
          <div className="text-[11px] font-mono text-vb-silver-dim">v0.3.0</div>
        </div>
        <div className="mt-7 flex items-center gap-2">
          {STEPS.map((s, i) => (
            <div key={s.id} className="flex items-center gap-2 flex-1">
              <div className="flex items-center gap-2 flex-1">
                <div
                  className={cn(
                    "h-1.5 flex-1 rounded-full transition-colors duration-300",
                    i < idx
                      ? "bg-vb-emerald"
                      : i === idx
                        ? "bg-vb-silver"
                        : "bg-vb-border",
                  )}
                />
              </div>
            </div>
          ))}
        </div>
        <div className="mt-2 flex justify-between">
          {STEPS.map((s, i) => (
            <div
              key={s.id}
              className={cn(
                "text-[10px] uppercase tracking-wider transition-colors",
                i === idx ? "text-vb-silver" : "text-vb-silver-dim/60",
              )}
            >
              {s.label}
            </div>
          ))}
        </div>
      </header>
      <main className="flex-1 overflow-y-auto px-10 pb-6">{children}</main>
      {footer && (
        <footer className="px-10 pb-8 pt-3 border-t border-vb-border/60">
          {footer}
        </footer>
      )}
    </div>
  );
}
