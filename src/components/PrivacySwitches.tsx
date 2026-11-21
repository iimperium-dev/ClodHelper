import { Toggle } from "./Toggle";
import { cn } from "../lib/cn";
import type { PrivacyKey } from "../lib/types";

export const PRIVACY_ITEMS: { key: PrivacyKey; title: string; hint: string }[] = [
  {
    key: "auto_update",
    title: "Обновлять автоматически",
    hint: "Когда программа свёрнута и вы не за компьютером",
  },
  {
    key: "crash_reports",
    title: "Отправлять отчёты об ошибках",
    hint: "Только текст ошибки. Без адресов и паролей",
  },
  {
    key: "usage_stats",
    title: "Отправлять анонимную статистику",
    hint: "Раз в день, без адресов и настроек прокси",
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
  /** Размер переключателей (в настройках sm). */
  toggleSize?: "md" | "sm";
  className?: string;
}

/*
  Три переключателя списком: заголовок и пояснение строкой ниже. Вся строка
  кликабельна (label), пояснение связано через aria-describedby.
*/
export function PrivacySwitches({ values, onChange, keys, idPrefix = "privacy", dense, toggleSize = "md", className }: Props) {
  const items = PRIVACY_ITEMS.filter((i) => !keys || keys.includes(i.key));
  return (
    <ul className={cn("divide-y divide-vb-border/60", className)}>
      {items.map((i) => {
        const id = `${idPrefix}-${i.key}`;
        return (
          <li key={i.key} className={cn("flex items-center gap-3 first:pt-0 last:pb-0", dense ? "py-1.5" : "py-2")}>
            <label htmlFor={id} className="min-w-0 flex-1 cursor-pointer">
              <span className="block text-[13px] text-vb-fg">{i.title}</span>
              <span id={`${id}-hint`} className="block text-[12px] leading-snug text-vb-silver-faint">
                {i.hint}
              </span>
            </label>
            <Toggle size={toggleSize} id={id} describedBy={`${id}-hint`} checked={values[i.key]} onChange={(on) => onChange(i.key, on)} />
          </li>
        );
      })}
    </ul>
  );
}
