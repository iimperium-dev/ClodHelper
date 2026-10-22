import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AppWindow,
  Globe,
  House,
  LayoutGrid,
  Link2,
  PanelLeftClose,
  PanelLeftOpen,
  Settings2,
} from "lucide-react";
import { StatusOrb } from "./StatusOrb";
import { BrandMark } from "./BrandMark";
import { formatUptime, useStatus } from "../lib/status";
import { cn } from "../lib/cn";
import type { AppView } from "../lib/types";

export const NAV_ORDER: AppView[] = ["overview", "proxies", "apps", "sites", "settings"];

const NAV_ITEMS: { id: AppView; label: string; icon: typeof LayoutGrid }[] = [
  { id: "overview", label: "Обзор", icon: House },
  { id: "proxies", label: "Прокси", icon: Globe },
  { id: "apps", label: "Приложения", icon: AppWindow },
  { id: "sites", label: "Сайты", icon: Link2 },
  { id: "settings", label: "Настройки", icon: Settings2 },
];

const COLLAPSED_W = 60;
const EXPANDED_W = 200;
const SPRING = { type: "spring", bounce: 0, duration: 0.38 } as const;
const STORAGE_KEY = "clod.sidebar.expanded";

function readExpanded(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

/** Подпись, проявляющаяся при раскрытии панели (сдвиг + прозрачность).
    Снаружи компонента: иначе пересоздавалась бы на каждом опросе статуса. */
function Label({
  show,
  children,
  className,
}: {
  show: boolean;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <AnimatePresence initial={false}>
      {show && (
        <motion.span
          initial={{ opacity: 0, x: -6 }}
          animate={{ opacity: 1, x: 0, transition: { delay: 0.08, duration: 0.2 } }}
          exit={{ opacity: 0, x: -4, transition: { duration: 0.1 } }}
          className={cn("relative min-w-0 truncate whitespace-nowrap", className)}
        >
          {children}
        </motion.span>
      )}
    </AnimatePresence>
  );
}

interface Props {
  view: AppView;
  onNavigate: (view: AppView) => void;
}

/*
  Сайдбар по умолчанию свёрнут в узкую полосу иконок — окну не нужна
  постоянная колонка подписей. Навигация работает и в свёрнутом виде
  (иконки + подсказки). Логотип — ручка панели: при наведении он
  перетекает в иконку «раскрыть», по клику панель выезжает пружиной,
  подписи проявляются со сдвигом. Выбор запоминается.
*/
export function Sidebar({ view, onNavigate }: Props) {
  const { status, health, healthErr, exeLegacy, orb, quality } = useStatus();
  const [expanded, setExpanded] = useState(readExpanded);
  const [logoHover, setLogoHover] = useState(false);

  const toggle = () => {
    setExpanded((v) => {
      try {
        localStorage.setItem(STORAGE_KEY, v ? "0" : "1");
      } catch {
        /* приватный режим — просто не запоминаем */
      }
      return !v;
    });
  };

  const loading = status === null;
  const running = status?.running ?? false;
  const statusLabel = loading
    ? "Проверяю…"
    : !running
      ? "Мост остановлен"
      : quality.some((q) => q.state === "down")
        ? "Прокси не отвечает"
        : orb === "warn"
          ? "Прокси нестабилен"
          : "Мост активен";
  const statusSub =
    !loading && running
      ? health
        ? `${formatUptime(health.uptime_sec)} · ${health.mode === "smart" ? "smart" : "весь трафик"}`
        : exeLegacy || healthErr
          ? "легаси-мост"
          : formatUptime(status?.uptime_sec)
      : null;

  return (
    <motion.aside
      initial={false}
      animate={{ width: expanded ? EXPANDED_W : COLLAPSED_W }}
      transition={SPRING}
      className="flex shrink-0 flex-col overflow-hidden border-r border-vb-border/60"
    >
      {/* Логотип — ручка панели */}
      <div className="mx-2.5 mb-3 mt-2.5 flex h-10 items-center gap-2">
        <button
          type="button"
          onClick={toggle}
          onMouseEnter={() => setLogoHover(true)}
          onMouseLeave={() => setLogoHover(false)}
          aria-label={expanded ? "Свернуть панель" : "Развернуть панель"}
          aria-expanded={expanded}
          title={expanded ? "Свернуть" : "Развернуть"}
          className="relative flex h-10 w-10 shrink-0 items-center justify-center rounded-xl transition-colors duration-150 hover:bg-vb-surface-2/60 active:scale-[0.94]"
        >
          <AnimatePresence initial={false} mode="popLayout">
            {logoHover && !expanded ? (
              <motion.span
                key="open"
                initial={{ opacity: 0, scale: 0.6, rotate: -90 }}
                animate={{ opacity: 1, scale: 1, rotate: 0 }}
                exit={{ opacity: 0, scale: 0.6, rotate: 90 }}
                transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
                className="text-vb-emerald"
              >
                <PanelLeftOpen className="h-[18px] w-[18px]" strokeWidth={1.9} />
              </motion.span>
            ) : (
              <motion.span
                key="logo"
                initial={{ opacity: 0, scale: 0.7 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.7, filter: "blur(2px)" }}
                transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
              >
                <BrandMark size={30} />
              </motion.span>
            )}
          </AnimatePresence>
        </button>
        <Label show={expanded} className="flex-1">
          <span className="block text-[13px] font-semibold leading-tight tracking-[-0.01em] text-vb-fg">
            Clod Helper
          </span>
          <span className="block text-[10px] leading-tight text-vb-silver-faint">AI-прокси мост</span>
        </Label>
        <AnimatePresence initial={false}>
          {expanded && (
            <motion.button
              type="button"
              onClick={toggle}
              aria-label="Свернуть панель"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1, transition: { delay: 0.12 } }}
              exit={{ opacity: 0, transition: { duration: 0.08 } }}
              className="shrink-0 rounded-md p-1 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver"
            >
              <PanelLeftClose className="h-4 w-4" strokeWidth={1.9} />
            </motion.button>
          )}
        </AnimatePresence>
      </div>

      {/* Навигация */}
      <nav className="flex flex-col gap-0.5 px-2.5" role="tablist" aria-label="Разделы">
        {NAV_ITEMS.map(({ id, label, icon: Icon }) => {
          const active = view === id;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={active}
              aria-label={label}
              title={expanded ? undefined : label}
              onClick={() => onNavigate(id)}
              className={cn(
                "relative flex h-9 items-center gap-2.5 rounded-lg px-[11px] text-left text-[13px] font-medium",
                "transition-colors duration-150 active:scale-[0.97]",
                active
                  ? "text-vb-fg"
                  : "text-vb-silver-dim hover:bg-vb-surface-2/50 hover:text-vb-silver",
              )}
            >
              {active && (
                <motion.span
                  layoutId="nav-pill"
                  className="absolute inset-0 rounded-lg border border-vb-border bg-vb-surface-2"
                  transition={SPRING}
                  aria-hidden
                />
              )}
              <Icon
                className={cn("relative h-4 w-4 shrink-0", active ? "text-vb-emerald" : "opacity-80")}
                strokeWidth={1.9}
              />
              <Label show={expanded}>{label}</Label>
            </button>
          );
        })}
      </nav>

      <div className="flex-1" />

      {/* Живой статус моста — всегда на виду (в свёрнутом виде — точка) */}
      <button
        type="button"
        onClick={() => onNavigate("overview")}
        title={expanded ? undefined : statusLabel}
        className={cn(
          "mx-2.5 mb-3 flex h-11 items-center gap-2.5 rounded-lg px-[15px] text-left",
          "transition-colors duration-150 hover:bg-vb-surface-2/40 active:scale-[0.97]",
          expanded && "border border-vb-border/70",
        )}
        aria-label={`${statusLabel} — открыть Обзор`}
      >
        <span className="relative shrink-0">
          <StatusOrb state={orb} size={8} />
        </span>
        <Label show={expanded}>
          <span className="block truncate text-[12px] font-medium leading-tight text-vb-silver">
            {statusLabel}
          </span>
          {statusSub && (
            <span className="tnum block truncate text-[10px] leading-tight text-vb-silver-faint">
              {statusSub}
            </span>
          )}
        </Label>
      </button>
    </motion.aside>
  );
}
