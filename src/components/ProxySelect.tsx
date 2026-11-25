import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Check, ChevronDown } from "lucide-react";
import { CountryFlag } from "./CountryFlag";
import { checkProxy } from "../lib/api";
import { useStatus } from "../lib/status";
import { hostOnly, hostPort, loadGeoCache, saveGeoCache } from "../lib/proxyGeo";
import { cn } from "../lib/cn";

/* ── Кастомный пикер прокси (нативный <select> выбивался из стиля) ── */
interface SelectProps {
  value: number; // -1 = Авто
  pool: string[];
  labels: Record<string, string>;
  disabled?: boolean;
  onChange: (v: number) => void;
}

export function ProxySelect({ value, pool, labels, disabled, onChange }: SelectProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  const { proxies } = useStatus();
  // Флаг и страна - из того же кэша гео, что на экране «Прокси»
  // (перечитываем при открытии: там могли проверить новый прокси).
  const [geoTick, setGeoTick] = useState(0);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const geo = useMemo(() => loadGeoCache(), [open, geoTick]);
  // Прокси без страны в кэше (экран «Прокси» ещё не открывали) - узнаём сейчас.
  useEffect(() => {
    if (!open) return;
    let alive = true;
    for (const url of pool) {
      if (geo[hostPort(url)]?.country_code) continue;
      void checkProxy(url)
        .then((r) => {
          if (!alive || !r?.country_code) return;
          saveGeoCache({ ...loadGeoCache(), [hostPort(url)]: r });
          setGeoTick((n) => n + 1);
        })
        .catch(() => {});
    }
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  // IP без порта; один IP на разных портах - с портом, иначе неотличимы.
  const shown = (url: string) =>
    pool.filter((u) => hostOnly(u) === hostOnly(url)).length > 1 ? hostPort(url) : hostOnly(url);
  const titleOf = (url: string) => labels[url] || shown(url);
  const flagOf = (url?: string) => (url ? geo[hostPort(url)]?.country_code : undefined);
  const current = value >= 0 ? pool[value] : undefined;

  const items = [{ v: -1, url: pool[0], auto: true }, ...pool.map((url, i) => ({ v: i, url, auto: false }))];

  return (
    <div className="relative shrink-0" ref={ref}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex min-w-[150px] items-center justify-between gap-2 rounded-lg border border-vb-border bg-vb-surface-2 px-2.5 py-1.5 text-left text-[12px] text-vb-silver",
          "transition-colors duration-150 hover:border-vb-border-strong active:scale-[0.98]",
          "disabled:cursor-not-allowed disabled:opacity-40",
          open && "border-vb-emerald/50",
        )}
      >
        <span className="flex min-w-0 items-center gap-1.5">
          {current && flagOf(current) && <CountryFlag code={flagOf(current)!} />}
          <span className="truncate font-medium">{current ? titleOf(current) : "Авто"}</span>
        </span>
        <ChevronDown
          className={cn(
            "h-3.5 w-3.5 shrink-0 opacity-60 transition-transform duration-200",
            open && "rotate-180",
          )}
        />
      </button>
      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, scale: 0.97, y: -4 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.98, y: -2 }}
            transition={{ duration: 0.14, ease: [0.22, 1, 0.36, 1] }}
            style={{ transformOrigin: "top right" }}
            className="z-dropdown absolute right-0 top-full mt-1 w-64 overflow-hidden rounded-lg border border-vb-border bg-vb-surface shadow-[0_12px_32px_rgba(0,0,0,0.55)]"
          >
            {items.map((it) => {
              const g = it.url ? geo[hostPort(it.url)] : undefined;
              const st = it.url ? proxies.find((p) => p.url === it.url) : undefined;
              const down = st?.state === "down";
              const slow = st?.state === "slow" || st?.state === "unstable";
              const selected = it.v === value;
              return (
                <button
                  key={it.v}
                  type="button"
                  onClick={() => {
                    setOpen(false);
                    if (it.v !== value) onChange(it.v);
                  }}
                  className={cn(
                    "flex w-full items-center gap-2.5 px-3 py-2 text-left transition-colors hover:bg-vb-surface-2",
                    selected && "bg-vb-surface-2/60",
                    it.auto && "border-b border-vb-border/60",
                  )}
                >
                  <div className="min-w-0 flex-1">
                    <div className={cn("truncate text-[13px] font-medium", selected ? "text-vb-fg" : "text-vb-silver")}>
                      {it.auto ? "Авто" : titleOf(it.url!)}
                    </div>
                    {/* Как на экране «Прокси»: флаг, страна, IP, пинг. У «Авто» -
                        через какой прокси он идёт сейчас (основной). */}
                    <div className="mt-1 flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[11px] leading-none text-vb-silver-faint">
                      {it.auto && <span className="shrink-0">сейчас</span>}
                      {g?.country_code && (
                        <span className="inline-flex shrink-0 items-center gap-1.5">
                          <CountryFlag code={g.country_code} />
                          {!it.auto && (g.country_name ?? g.country_code)}
                        </span>
                      )}
                      {it.url && (it.auto || labels[it.url]) && (
                        <span className="truncate font-mono">{shown(it.url)}</span>
                      )}
                      {!it.auto && it.url && (down || typeof st?.ms === "number") && (
                        <>
                          <span aria-hidden="true">·</span>
                          <span className={cn("tnum shrink-0 font-mono", down ? "text-vb-loss" : slow ? "text-vb-warn" : "")}>
                            {down ? "не работает" : `${st!.ms} мс`}
                          </span>
                        </>
                      )}
                    </div>
                  </div>
                  {selected && <Check className="h-3.5 w-3.5 shrink-0 text-vb-emerald" />}
                </button>
              );
            })}
            <div className="border-t border-vb-border/60 px-3 py-1.5 text-[10px] leading-snug text-vb-silver-faint">
              «Авто» - основной, при падении - запасные. Конкретный прокси - строго он, IP не меняется
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
