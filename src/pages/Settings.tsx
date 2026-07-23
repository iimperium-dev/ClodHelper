import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import { Loader2, Power } from "lucide-react";
import {
  disable as autostartDisable,
  enable as autostartEnable,
  isEnabled as autostartIsEnabled,
} from "@tauri-apps/plugin-autostart";
import { getVersion } from "@tauri-apps/api/app";
import { Toggle } from "../components/Toggle";
import { InfoTip } from "../components/InfoTip";
import { useStatus } from "../lib/status";
import { fadeInUp, staggerContainer } from "../lib/motion";

/*
  Настройки: автозапуск GUI и версии. Прокси-пул и назначения живут в
  своём разделе «Прокси» (сайдбар) — здесь только системное.
*/
export function Settings() {
  const { health, task } = useStatus();

  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [autostartError, setAutostartError] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string>("");

  const load = useCallback(async () => {
    setAutostart(await autostartIsEnabled().catch(() => false));
    setAppVersion(await getVersion().catch(() => ""));
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleAutostartToggle = async () => {
    setAutostartBusy(true);
    setAutostartError(null);
    try {
      if (autostart) {
        await autostartDisable();
        setAutostart(false);
      } else {
        await autostartEnable();
        setAutostart(true);
      }
    } catch {
      setAutostartError(
        "Не получилось изменить автозапуск (нет прав в Windows). Можно вручную через Пуск → Автозагрузка.",
      );
    } finally {
      setAutostartBusy(false);
    }
  };

  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[620px] flex-col px-8 py-7"
    >
      <motion.header variants={fadeInUp}>
        <h1 className="text-[24px] font-bold leading-tight tracking-[-0.02em] text-vb-fg">
          Настройки
        </h1>
        <p className="mt-0.5 text-[13px] text-vb-silver-dim">
          Автозапуск и сведения о приложении
        </p>
      </motion.header>

      {/* ── Автозапуск GUI ── */}
      <motion.section variants={fadeInUp} className="surface-card mt-5 p-4">
        <div className="flex items-center justify-between gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-vb-surface-2 text-vb-silver-dim">
              <Power className="h-4 w-4" strokeWidth={1.9} />
            </div>
            <div className="flex min-w-0 items-center gap-2">
              <div className="text-[14px] font-semibold text-vb-fg">
                Запускать с Windows
              </div>
              <InfoTip>
                Clod Helper стартует свёрнутым в трей при входе в Windows.
                Сам мост от этого не зависит — он автостартует отдельной
                задачей Планировщика и работает даже без открытого приложения.
              </InfoTip>
            </div>
          </div>
          {autostart === null ? (
            <Loader2 className="h-4 w-4 shrink-0 animate-spin text-vb-silver-dim" />
          ) : (
            <Toggle
              checked={autostart}
              onChange={handleAutostartToggle}
              disabled={autostartBusy}
              label="Автозапуск"
            />
          )}
        </div>
        {autostartError && (
          <p className="mt-3 pl-11 text-[12px] text-vb-warn">{autostartError}</p>
        )}
      </motion.section>

      {/* ── О приложении ── */}
      <motion.section variants={fadeInUp} className="surface-card mt-4 divide-y divide-vb-border/70">
        <div className="flex items-center justify-between px-4 py-3">
          <span className="text-[13px] text-vb-silver-dim">Clod Helper</span>
          <span className="font-mono text-[12px] text-vb-silver">
            {appVersion ? `v${appVersion}` : "—"}
          </span>
        </div>
        <div className="flex items-center justify-between px-4 py-3">
          <span className="text-[13px] text-vb-silver-dim">Мост (clod-bridge)</span>
          <span className="font-mono text-[12px] text-vb-silver">
            {health ? `v${health.version}` : "—"}
          </span>
        </div>
        <div className="flex items-center justify-between px-4 py-3">
          <span className="text-[13px] text-vb-silver-dim">Автозапуск моста</span>
          <span className="font-mono text-[12px] text-vb-silver">
            {task === null ? "—" : task.registered ? "Планировщик задач" : "легаси (.vbs)"}
          </span>
        </div>
      </motion.section>
    </motion.div>
  );
}
