import { useEffect, useRef, type ReactNode } from "react";
import { Check } from "lucide-react";
import type { WizardStep } from "../lib/types";
import { cn } from "../lib/cn";

const STEPS: { id: WizardStep; label: string }[] = [
  { id: "proxy", label: "Прокси" },
  { id: "install", label: "Настройка" },
  { id: "done", label: "Готово" },
];

interface Props {
  step: WizardStep;
  children: ReactNode;
}

/*
  Каркас мастера первого запуска.
  - Шапка в одну строку: имя программы слева, «Шаг N из 3» справа; под ней
    три сегмента прогресса с подписями. Заливка сегмента - transform
    (scaleX), не ширина: без перерисовки раскладки.
  - Шаг въезжает CSS-анимацией (.wizard-enter) со стороны движения: вперёд -
    справа, назад - слева. Не framer AnimatePresence: в App.tsx описано, как
    он изредка оставлял экран в opacity 0. Reduced motion - только появление
    без сдвига (index.css).
  - После смены шага фокус - на заголовок шага (tabIndex -1): читалка
    экрана объявляет новый шаг, Tab идёт с начала шага.
  - Кнопки шага - в <WizardFooter>: прилипают к низу прокрутки, на
    маленьком окне всегда видны.
*/
export function WizardShell({ step, children }: Props) {
  const idx = STEPS.findIndex((s) => s.id === step);
  const prev = useRef(idx);
  const dir = idx >= prev.current ? 1 : -1;
  const mainRef = useRef<HTMLElement>(null);

  useEffect(() => {
    prev.current = idx;
    const main = mainRef.current;
    if (!main) return;
    main.scrollTop = 0;
    // Шаг сам поставил фокус (autoFocus поля или главной кнопки) - не трогаем.
    if (main.contains(document.activeElement)) return;
    main.querySelector<HTMLElement>("[data-wizard-title]")?.focus({ preventScroll: true });
  }, [idx]);

  return (
    <div className="flex h-full flex-col bg-vb-bg text-vb-silver">
      <header className="shrink-0 px-8 pt-5 pb-4">
        <div className="flex items-baseline justify-between gap-4">
          <span className="text-[13px] font-medium text-vb-silver">Настройка Clod Helper</span>
          <span className="tnum text-[12px] text-vb-silver-faint" aria-live="polite">
            Шаг {idx + 1} из {STEPS.length}
          </span>
        </div>
        <ol aria-label="Шаги настройки" className="mt-3 grid grid-cols-3 gap-2">
          {STEPS.map((s, i) => {
            const done = i < idx;
            const current = i === idx;
            return (
              <li key={s.id} aria-current={current ? "step" : undefined}>
                <div className="h-1 overflow-hidden rounded-full bg-vb-border">
                  <div
                    className={cn(
                      "h-full origin-left rounded-full",
                      "transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]",
                      done ? "bg-vb-emerald-dim" : "bg-vb-emerald",
                    )}
                    style={{ transform: `scaleX(${done || current ? 1 : 0})` }}
                  />
                </div>
                <div
                  className={cn(
                    "mt-1.5 flex items-center gap-1 text-[12px] transition-colors duration-200",
                    current ? "text-vb-silver" : done ? "text-vb-silver-dim" : "text-vb-silver-faint",
                  )}
                >
                  {done && <Check className="h-3 w-3 text-vb-emerald-dim" aria-hidden />}
                  {s.label}
                  {done && <span className="sr-only">, готово</span>}
                </div>
              </li>
            );
          })}
        </ol>
      </header>
      <main
        ref={mainRef}
        className="relative flex min-h-0 flex-1 flex-col overflow-y-auto border-t border-vb-border/60"
      >
        <div
          key={step}
          className="wizard-enter flex flex-1 flex-col px-8"
          style={{ "--wizard-dir": dir } as React.CSSProperties}
        >
          {children}
        </div>
      </main>
    </div>
  );
}

/** Заголовок шага: получает фокус при смене шага. */
export function WizardTitle({ children, sub }: { children: ReactNode; sub?: ReactNode }) {
  return (
    <div className="pt-6">
      <h2
        data-wizard-title
        tabIndex={-1}
        className="text-[20px] font-semibold leading-tight tracking-[-0.01em] text-vb-fg outline-none"
      >
        {children}
      </h2>
      {sub && <p className="mt-1.5 max-w-[52ch] text-[13px] leading-relaxed text-vb-silver-dim">{sub}</p>}
    </div>
  );
}

/** Кнопки шага: прилипают к низу окна, слева - тихие ссылки, справа - действие. */
export function WizardFooter({ left, children }: { left?: ReactNode; children?: ReactNode }) {
  return (
    <div className="sticky bottom-0 -mx-8 mt-auto flex items-center justify-between gap-3 border-t border-vb-border/60 bg-vb-bg/95 px-8 py-4 backdrop-blur-sm">
      <div className="flex min-w-0 flex-col items-start gap-1">{left}</div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}
