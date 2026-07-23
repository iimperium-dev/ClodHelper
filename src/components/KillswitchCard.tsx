import { useState } from "react";
import { motion, useReducedMotion } from "framer-motion";
import { Loader2, Shield, ShieldAlert, ShieldCheck } from "lucide-react";
import { Toggle } from "./Toggle";
import { cn } from "../lib/cn";
import type { KillswitchStatus } from "../lib/types";

interface Props {
  status: KillswitchStatus | null;
  bridgeRunning: boolean;
  enabledAppCount: number;
  onEnable: () => Promise<void>;
  onDisable: () => Promise<void>;
}

/*
  Killswitch — центральный элемент безопасности.
  Три состояния, недвусмысленно:
  - Защищён (active + мост жив): изумруд, трафик только через мост.
  - Мост мёртв (active + !bridge): amber-предупреждение — приложения без сети
    (это правильный fail-closed, но пользователь должен понимать почему).
  - Открыт (не active): серый/красный — трафик может утечь напрямую.
*/
export function KillswitchCard({
  status,
  bridgeRunning,
  enabledAppCount,
  onEnable,
  onDisable,
}: Props) {
  const reduce = useReducedMotion();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = status?.active ?? false;

  const state: "protected" | "bridge_dead" | "open" = active
    ? bridgeRunning
      ? "protected"
      : "bridge_dead"
    : "open";

  const toggle = async () => {
    setBusy(true);
    setError(null);
    try {
      if (active) {
        await onDisable();
      } else {
        await onEnable();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const meta = {
    protected: {
      icon: ShieldCheck,
      accent: "text-vb-emerald",
      ring: "border-vb-emerald/30",
      glow: "bg-vb-emerald/[0.05]",
      title: "Killswitch включён",
      sub: "Трафик выбранных приложений идёт только через мост. Утечка напрямую невозможна.",
    },
    bridge_dead: {
      icon: ShieldAlert,
      accent: "text-vb-warn",
      ring: "border-vb-warn/30",
      glow: "bg-vb-warn/[0.05]",
      title: "Killswitch включён · мост недоступен",
      sub: "Приложения сейчас без сети — это защита. Запустите мост, чтобы вернуть доступ.",
    },
    open: {
      icon: Shield,
      accent: "text-vb-silver-dim",
      ring: "border-vb-border",
      glow: "",
      title: "Killswitch выключен",
      sub: "Приложения могут выйти в сеть напрямую, если прокси отвалится. Включите для гарантии.",
    },
  }[state];

  const Icon = meta.icon;

  return (
    <section
      className={cn(
        "surface-card relative overflow-hidden p-5 transition-colors duration-300",
        meta.ring,
      )}
    >
      {meta.glow && (
        <div className={cn("pointer-events-none absolute inset-0", meta.glow)} />
      )}
      <div className="relative flex items-start gap-4">
        <div
          className={cn(
            "flex h-11 w-11 shrink-0 items-center justify-center rounded-lg bg-vb-surface-2",
            meta.accent,
          )}
        >
          {active && !reduce ? (
            <motion.span
              animate={{ scale: [1, 1.08, 1] }}
              transition={{ duration: 2.4, repeat: Infinity, ease: "easeInOut" }}
            >
              <Icon className="h-6 w-6" strokeWidth={1.75} />
            </motion.span>
          ) : (
            <Icon className="h-6 w-6" strokeWidth={1.75} />
          )}
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className={cn("text-[15px] font-semibold", meta.accent)}>
              {meta.title}
            </h2>
          </div>
          <p className="mt-1 text-[13px] leading-relaxed text-vb-silver-dim">
            {meta.sub}
          </p>

          {active && status && status.blocked_processes.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1">
              {status.blocked_processes.map((p) => (
                <span
                  key={p}
                  className="rounded-md bg-vb-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-vb-silver-faint"
                >
                  {p}
                </span>
              ))}
            </div>
          )}

          {error && (
            <p className="mt-2 text-[12px] text-vb-loss">{error}</p>
          )}
        </div>

        <div className="flex shrink-0 items-center">
          {busy ? (
            <Loader2 className="h-5 w-5 animate-spin text-vb-silver-dim" />
          ) : (
            <Toggle
              checked={active}
              onChange={toggle}
              tone={active ? "emerald" : "loss"}
              disabled={!active && enabledAppCount === 0}
              label="Killswitch"
            />
          )}
        </div>
      </div>

      {!active && enabledAppCount === 0 && (
        <p className="relative mt-3 text-[12px] text-vb-silver-faint">
          Включите хотя бы одно приложение выше, чтобы активировать killswitch.
        </p>
      )}
    </section>
  );
}
