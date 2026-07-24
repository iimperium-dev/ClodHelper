import { AnimatePresence, motion } from "framer-motion";
import { AppWindow, Globe, House, LayoutGrid, Settings2 } from "lucide-react";
import { StatusOrb } from "./StatusOrb";
import { BrandMark } from "./BrandMark";
import { formatUptime, useStatus } from "../lib/status";
import { cn } from "../lib/cn";
import type { AppView } from "../lib/types";

export const NAV_ORDER: AppView[] = ["overview", "proxies", "apps", "settings"];

/* Обзор в списке не нужен: главным меню служит бренд-блок сверху. */
const NAV_ITEMS: { id: AppView; label: string; icon: typeof LayoutGrid }[] = [
  { id: "proxies", label: "Прокси", icon: Globe },
  { id: "apps", label: "Приложения", icon: AppWindow },
  { id: "settings", label: "Настройки", icon: Settings2 },
];

interface Props {
  view: AppView;
  onNavigate: (view: AppView) => void;
}

/*
  Сайдбар-навигация app shell:
  - активный пункт — «пилюля», перетекающая спрингом (layoutId);
  - подвал — живой статус моста, виден из любого раздела (wayfinding:
    статус никогда не прячется за навигацией), клик ведёт в Обзор.
*/
export function Sidebar({ view, onNavigate }: Props) {
  const { status, health, healthErr, exeLegacy } = useStatus();

  const loading = status === null;
  const running = status?.running ?? false;
  const upstreamDown =
    health !== null && health.upstreams.some((u) => !u.healthy);
  const orb = loading
    ? ("loading" as const)
    : !running
      ? ("down" as const)
      : upstreamDown
        ? ("warn" as const)
        : ("ok" as const);

  const statusLabel = loading
    ? "Проверяю…"
    : !running
      ? "Мост остановлен"
      : upstreamDown
        ? "Upstream лежит"
        : "Мост активен";

  const statusSub = !loading && running
    ? health
      ? `${formatUptime(health.uptime_sec)} · ${health.mode === "smart" ? "smart" : "весь трафик"}`
      : exeLegacy || healthErr
        ? "легаси-мост"
        : formatUptime(status?.uptime_sec)
    : null;

  return (
    <aside className="flex w-[196px] shrink-0 flex-col border-r border-vb-border/60">
      {/* Бренд-блок = главное меню (Обзор): выделяется пилюлей как пункт
          навигации, пилюля перетекает между ним и пунктами ниже (layoutId).
          Кликабельность не должна быть «скрытым знанием»: вне Обзора справа
          проявляется домик-подсказка (ярче при наведении) + tooltip. */}
      <button
        type="button"
        onClick={() => onNavigate("overview")}
        aria-label="Clod Helper — Обзор"
        aria-current={view === "overview" ? "page" : undefined}
        title={view !== "overview" ? "На главную" : undefined}
        className={cn(
          "group relative mx-2.5 mb-3 mt-2.5 flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left",
          "transition-colors duration-150 active:scale-[0.98]",
          view !== "overview" && "hover:bg-vb-surface-2/50",
        )}
      >
        {view === "overview" && (
          <motion.span
            layoutId="nav-pill"
            className="absolute inset-0 rounded-lg border border-vb-border bg-vb-surface-2"
            transition={{ type: "spring", bounce: 0, duration: 0.35 }}
            aria-hidden
          />
        )}
        <BrandMark size={30} className="relative" />
        <div className="relative min-w-0 flex-1">
          <div className="text-[13px] font-semibold leading-tight tracking-[-0.01em] text-vb-fg">
            Clod Helper
          </div>
          <div className="text-[10px] leading-tight text-vb-silver-faint">
            AI-прокси мост
          </div>
        </div>
        <AnimatePresence>
          {view !== "overview" && (
            <motion.span
              initial={{ opacity: 0, scale: 0.8 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.8 }}
              transition={{ duration: 0.16, ease: [0.22, 1, 0.36, 1] }}
              className="relative shrink-0 text-vb-silver-faint transition-colors duration-150 group-hover:text-vb-silver"
              aria-hidden
            >
              <House className="h-3.5 w-3.5" strokeWidth={1.9} />
            </motion.span>
          )}
        </AnimatePresence>
      </button>

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
              onClick={() => onNavigate(id)}
              className={cn(
                "relative flex items-center gap-2.5 rounded-lg px-3 py-2 text-left text-[13px] font-medium",
                "transition-colors duration-150 active:scale-[0.98]",
                active
                  ? "text-vb-fg"
                  : "text-vb-silver-dim hover:bg-vb-surface-2/50 hover:text-vb-silver",
              )}
            >
              {active && (
                <motion.span
                  layoutId="nav-pill"
                  className="absolute inset-0 rounded-lg border border-vb-border bg-vb-surface-2"
                  transition={{ type: "spring", bounce: 0, duration: 0.35 }}
                  aria-hidden
                />
              )}
              <Icon
                className={cn(
                  "relative h-4 w-4",
                  active ? "text-vb-emerald" : "opacity-80",
                )}
                strokeWidth={1.9}
              />
              <span className="relative">{label}</span>
            </button>
          );
        })}
      </nav>

      <div className="flex-1" />

      {/* Живой статус моста — всегда на виду */}
      <button
        type="button"
        onClick={() => onNavigate("overview")}
        className={cn(
          "mx-2.5 mb-3 flex items-center gap-2.5 rounded-lg border border-vb-border/70 px-3 py-2.5 text-left",
          "transition-colors duration-150 hover:border-vb-border-strong hover:bg-vb-surface-2/40 active:scale-[0.98]",
        )}
        aria-label="Статус моста — открыть Обзор"
      >
        <StatusOrb state={orb} size={8} />
        <div className="min-w-0">
          <div className="truncate text-[12px] font-medium leading-tight text-vb-silver">
            {statusLabel}
          </div>
          {statusSub && (
            <div className="tnum truncate text-[10px] leading-tight text-vb-silver-faint">
              {statusSub}
            </div>
          )}
        </div>
      </button>
    </aside>
  );
}
