import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Loader2 } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "./Button";
import { updateCheck } from "../lib/api";
import { openUpdateDialog } from "../lib/updateBus";
import type { UpdateInfo } from "../lib/types";
import { cn } from "../lib/cn";

const SESSION_KEY = "clod.update.checked";

function remember(r: UpdateInfo | null) {
  try {
    sessionStorage.setItem(SESSION_KEY, JSON.stringify(r));
  } catch {
    /* ignore */
  }
}

/** Одна проверка за сеанс приложения (GitHub API не дёргаем на каждый заход).
    Ошибку не кэшируем и не выдаём за «последнюю версию» — показываем её. */
async function checkOnce(force: boolean): Promise<{ info: UpdateInfo | null; error: string | null }> {
  try {
    const cached = sessionStorage.getItem(SESSION_KEY);
    if (!force && cached) return { info: JSON.parse(cached) as UpdateInfo | null, error: null };
  } catch {
    /* нет sessionStorage — просто проверим */
  }
  try {
    const r = await updateCheck();
    remember(r);
    return { info: r, error: null };
  } catch (e) {
    return { info: null, error: String(e) };
  }
}

/*
  Обновление приложения: «Доступна версия X» → «Обновить» открывает окно
  обновления (UpdateDialog): что нового и живой ход установки.
  compact — плашка для Обзора; иначе — блок для Настроек с ручной проверкой.
*/
export function UpdateBanner({ compact }: { compact?: boolean }) {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [checked, setChecked] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void checkOnce(false).then((r) => {
      setInfo(r.info);
      setError(r.error);
      setChecked(true);
    });
    // Фоновая проверка (раз в несколько часов) нашла новую версию.
    const un = listen<UpdateInfo>("update:available", (e) => {
      remember(e.payload);
      setInfo(e.payload);
      setError(null);
      setChecked(true);
    });
    return () => void un.then((f) => f());
  }, []);

  const check = async () => {
    setBusy(true);
    setError(null);
    const r = await checkOnce(true);
    setInfo(r.info);
    setError(r.error);
    setChecked(true);
    setBusy(false);
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
            <div className={cn("text-[12px]", error && !info ? "text-vb-warn" : "truncate text-vb-silver-faint")}>
              {info
                ? "Посмотрите, что нового, — обновление в один клик"
                : busy
                  ? "Проверяю…"
                  : error
                    ? `${error.replace(/^Не удалось проверить обновления:\s*/, "Не удалось проверить: ")}`
                    : checked
                      ? "У вас последняя версия"
                      : "Проверяю…"}
            </div>
          </div>
          {info ? (
            <Button variant="primary" size="sm" onClick={() => openUpdateDialog(info)}>
              Обновить
            </Button>
          ) : (
            !compact && (
              <Button variant="secondary" size="sm" onClick={check} disabled={busy}>
                {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                {error ? "Повторить" : "Проверить"}
              </Button>
            )
          )}
        </div>
      </motion.div>
    </AnimatePresence>
  );
}
