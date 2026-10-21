import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Loader2 } from "lucide-react";
import { Button } from "./Button";
import { updateCheck, updateInstall } from "../lib/api";
import type { UpdateInfo } from "../lib/types";

const SESSION_KEY = "clod.update.checked";
const RELEASES = "https://github.com/iimperium-dev/ClodHelper/releases/latest";

/** Одна проверка за сеанс приложения (GitHub API не дёргаем на каждый заход). */
async function checkOnce(force: boolean): Promise<UpdateInfo | null> {
  try {
    const cached = sessionStorage.getItem(SESSION_KEY);
    if (!force && cached) return JSON.parse(cached) as UpdateInfo | null;
  } catch {
    /* нет sessionStorage — просто проверим */
  }
  const r = await updateCheck().catch(() => null);
  try {
    sessionStorage.setItem(SESSION_KEY, JSON.stringify(r));
  } catch {
    /* ignore */
  }
  return r;
}

/*
  Обновление приложения: «Доступна версия X» → «Обновить». Скачивание и
  проверка контрольной суммы идут в бэкенде; приложение перезапустится само.
  compact — плашка для Обзора; иначе — блок для Настроек с ручной проверкой.
*/
export function UpdateBanner({ compact }: { compact?: boolean }) {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [checked, setChecked] = useState(false);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void checkOnce(false).then((r) => {
      setInfo(r);
      setChecked(true);
    });
  }, []);

  const install = async () => {
    if (!info) return;
    setBusy("install");
    setError(null);
    try {
      await updateInstall(info);
    } catch (e) {
      setError(String(e));
      setBusy(null);
    }
  };

  const check = async () => {
    setBusy("check");
    setError(null);
    setInfo(await checkOnce(true));
    setChecked(true);
    setBusy(null);
  };

  if (compact && !info) return null;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0, y: 4 }}
        animate={{ opacity: 1, y: 0 }}
        className={compact ? "mt-3" : ""}
      >
        <div className="flex items-center justify-between gap-3 rounded-xl border border-vb-border bg-vb-surface px-4 py-3">
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-vb-fg">
              {info ? `Доступна версия ${info.version}` : "Обновления"}
            </div>
            <div className="truncate text-[12px] text-vb-silver-faint">
              {busy === "install"
                ? "Скачиваю и проверяю — приложение перезапустится само"
                : info
                  ? "Скачается с GitHub, целостность проверяется по SHA-256"
                  : checked
                    ? "У вас последняя версия"
                    : "Проверяю…"}
            </div>
          </div>
          {info ? (
            <Button variant="primary" size="sm" onClick={install} disabled={busy !== null}>
              {busy === "install" && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              Обновить
            </Button>
          ) : (
            !compact && (
              <Button variant="secondary" size="sm" onClick={check} disabled={busy !== null}>
                {busy === "check" && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Проверить
              </Button>
            )
          )}
        </div>
        {error && (
          <p className="mt-2 text-[12px] text-vb-loss">
            {error}.{" "}
            <button
              type="button"
              onClick={() => import("@tauri-apps/plugin-opener").then((m) => m.openUrl(RELEASES))}
              className="text-vb-emerald hover:underline"
            >
              Скачать вручную
            </button>
          </p>
        )}
      </motion.div>
    </AnimatePresence>
  );
}
