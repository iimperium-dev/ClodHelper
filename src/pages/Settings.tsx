import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import { Loader2 } from "lucide-react";
import { getVersion } from "@tauri-apps/api/app";
import { Toggle } from "../components/Toggle";
import { useStatus } from "../lib/status";
import { autostartGet, autostartSet } from "../lib/api";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { isMac } from "../lib/platform";
import { cn } from "../lib/cn";

/*
  Настройки: автозапуск GUI и версии. Прокси-пул и назначения живут в
  своём разделе «Прокси» (сайдбар) — здесь только системное.
*/
export function Settings() {
  const { health, task } = useStatus();

  const [autostart, setAutostart] = useState<boolean | null>(null);
  /** macOS: объект входа добавлен, но ждёт разрешения в Системных настройках. */
  const [needsApproval, setNeedsApproval] = useState(false);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [autostartError, setAutostartError] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string>("");

  const load = useCallback(async () => {
    const st = await autostartGet().catch(() => null);
    setAutostart(st?.enabled ?? false);
    setNeedsApproval(st?.needs_approval ?? false);
    setAppVersion(await getVersion().catch(() => ""));
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleAutostartToggle = async () => {
    setAutostartBusy(true);
    setAutostartError(null);
    try {
      const st = await autostartSet(!autostart);
      setAutostart(st.enabled);
      setNeedsApproval(st.needs_approval);
    } catch {
      setAutostartError(
        isMac
          ? "Не получилось изменить автозапуск. Можно вручную: Системные настройки → Основные → Объекты входа."
          : "Не получилось изменить автозапуск (нет прав в Windows). Можно вручную через Пуск → Автозагрузка.",
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
      className="mx-auto flex w-full max-w-[520px] flex-col px-6 py-6"
    >
      <motion.header variants={fadeInUp}>
        <h1 className="text-[20px] font-semibold leading-tight tracking-[-0.02em] text-vb-fg">
          Настройки
        </h1>
      </motion.header>

      {/* ── Автозапуск GUI ── */}
      <motion.section variants={fadeInUp} className="surface-card mt-5 p-4">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div className="text-[14px] font-medium text-vb-fg">Открывать при входе</div>
            <div className="text-[12px] text-vb-silver-faint">
              Значок в {isMac ? "строке меню" : "трее"}. Мост работает и без этого
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
          <p className="mt-3 text-[12px] text-vb-warn">{autostartError}</p>
        )}
        {!autostartError && needsApproval && (
          <p className="mt-3 text-[12px] leading-snug text-vb-warn">
            Разрешите в Системных настройках → Объекты входа
          </p>
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
          <span className="text-[13px] text-vb-silver-dim">Мост</span>
          <span className="font-mono text-[12px] text-vb-silver">
            {health ? `v${health.version}` : "—"}
          </span>
        </div>
        <div className="flex items-center justify-between px-4 py-3">
          <span className="text-[13px] text-vb-silver-dim">Мост запускается сам</span>
          <span className={cn("text-[12px]", task?.registered ? "text-vb-emerald" : "text-vb-warn")}>
            {task === null ? "—" : task.registered ? "да" : "нет"}
          </span>
        </div>
      </motion.section>
    </motion.div>
  );
}
