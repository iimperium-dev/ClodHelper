import { Toggle } from "./Toggle";
import { cn } from "../lib/cn";
import type { PrivacyKey } from "../lib/types";

export const PRIVACY_ITEMS: { key: PrivacyKey; title: string; hint: string }[] = [
  {
    key: "auto_update",
    title: "Обновлять автоматически",
    hint: "Новые версии ставятся сами, когда программа свёрнута",
  },
  {
    key: "crash_reports",
    title: "Отправлять отчёты об ошибках",
    hint: "Если программа упадёт, мы узнаем и починим",
  },
  {
    key: "usage_stats",
    title: "Отправлять анонимную статистику",
    hint: "Версия и система раз в день. Без адресов и настроек прокси",
  },
];

interface Props {
  values: Record<PrivacyKey, boolean>;
  onChange: (key: PrivacyKey, on: boolean) => void;
  /** Какие переключатели показать (по умолчанию все три). */
  keys?: PrivacyKey[];
  /** Префикс id: на экране может быть несколько списков. */
  idPrefix?: string;
  /** Плотнее: для мастера, где место поделено с кнопкой шага. */
  dense?: boolean;
  className?: string;
}

/*
  Три переключателя списком: заголовок и пояснение строкой ниже. Вся строка
  кликабельна (label), пояснение связано через aria-describedby.
*/
export function PrivacySwitches({ values, onChange, keys, idPrefix = "privacy", dense, className }: Props) {
  const items = PRIVACY_ITEMS.filter((i) => !keys || keys.includes(i.key));
  return (
    <ul className={cn("divide-y divide-vb-border/60", className)}>
      {items.map((i) => {
        const id = `${idPrefix}-${i.key}`;
        return (
          <li key={i.key} className={cn("flex items-center gap-3 first:pt-0 last:pb-0", dense ? "py-1.5" : "py-2.5")}>
            <label htmlFor={id} className="min-w-0 flex-1 cursor-pointer">
              <span className="block text-[13px] text-vb-fg">{i.title}</span>
              <span id={`${id}-hint`} className="block text-[12px] leading-snug text-vb-silver-dim">
                {i.hint}
              </span>
            </label>
            <Toggle id={id} describedBy={`${id}-hint`} checked={values[i.key]} onChange={(on) => onChange(i.key, on)} />
          </li>
        );
      })}
    </ul>
  );
}
