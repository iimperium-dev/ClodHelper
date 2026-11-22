import { useCallback, useEffect, useRef, useState } from "react";
import { motion, useReducedMotion } from "framer-motion";
import { AlertTriangle, ArrowRight, Check, Download, Loader2 } from "lucide-react";
import { Button } from "../components/Button";
import { WizardFooter, WizardTitle } from "../components/WizardShell";
import { wizardApps, wizardConnectClaude, wizardOpenStore } from "../lib/api";
import type { WizardAppId, WizardApps, WizardClaude } from "../lib/types";
import { cn } from "../lib/cn";
import { isMac } from "../lib/platform";

interface Props {
  /** Что было при входе на шаг: показываем только недостающее и уже стоящее. */
  initial: WizardApps;
  /** Итог Claude Desktop для шага «Готово» (только что поставили и подключили). */
  onClaude: (claude: WizardClaude, running: boolean) => void;
  onDone: () => void;
}

/** Как часто проверять, закончилась ли установка. */
export const WIZARD_APPS_POLL_MS = 3000;

/*
  Состояние строки:
  - have - стояло до этого шага;
  - missing - нет, кнопка «Установить»;
  - wait - открыли магазин (или сайт на Mac), опрашиваем детект;
  - connect - Claude Desktop нашёлся, подключаем к мосту (окно системы);
  - ok - установлено (и для Claude - подключено);
  - soft - Claude Desktop установлен, но не подключился (можно позже).
*/
type Row = "have" | "missing" | "wait" | "connect" | "ok" | "soft";

const APPS: { id: WizardAppId; name: string; what: string }[] = [
  { id: "claude", name: "Claude Desktop", what: "Приложение Claude от Anthropic" },
  { id: "codex", name: "ChatGPT Codex", what: "Приложение Codex от OpenAI" },
];

/*
  Шаг «Приложения» - только если Claude Desktop или Codex нет на компьютере.
  «Установить» открывает Microsoft Store (на Mac - страницу загрузки), мастер
  сам замечает установку: опрос раз в несколько секунд и при возврате в окно.
  Нашлось - «Установлено», Claude Desktop сразу подключается к мосту, и когда
  ставить больше нечего, мастер идёт дальше сам. Всё можно пропустить.
*/
export function StepApps({ initial, onClaude, onDone }: Props) {
  const reduce = useReducedMotion();
  const [rows, setRows] = useState<Record<WizardAppId, Row>>({
    claude: initial.claude ? "have" : "missing",
    codex: initial.codex ? "have" : "missing",
  });
  const [openErr, setOpenErr] = useState<string | null>(null);
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  const installedHere = useRef(false);

  const set = (id: WizardAppId, r: Row) => setRows((s) => ({ ...s, [id]: r }));

  const connectClaude = useCallback(async () => {
    set("claude", "connect");
    const r = await wizardConnectClaude().catch(() => ({ ok: false, running: false }));
    set("claude", r.ok ? "ok" : "soft");
    onClaude(r.ok ? "ok" : "err", r.running);
  }, [onClaude]);

  // Опрос детекта, пока что-то ставится. Плюс сразу при возврате в окно:
  // человек закрыл магазин - ответ без ожидания следующего тика.
  const waiting = rows.claude === "wait" || rows.codex === "wait";
  const check = useCallback(async () => {
    let now: WizardApps;
    try {
      now = await wizardApps();
    } catch {
      return;
    }
    const cur = rowsRef.current;
    if (cur.codex === "wait" && now.codex) {
      installedHere.current = true;
      set("codex", "ok");
    }
    if (cur.claude === "wait" && now.claude) {
      installedHere.current = true;
      void connectClaude();
    }
  }, [connectClaude]);
  useEffect(() => {
    if (!waiting) return;
    const t = window.setInterval(() => void check(), WIZARD_APPS_POLL_MS);
    const onFocus = () => void check();
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  }, [waiting, check]);

  // Ставить больше нечего и всё, что ставили здесь, готово - дальше сам.
  const settled = APPS.every(({ id }) => rows[id] === "have" || rows[id] === "ok" || rows[id] === "soft");
  useEffect(() => {
    if (!settled || !installedHere.current) return;
    const t = window.setTimeout(onDone, 1400);
    return () => window.clearTimeout(t);
  }, [settled, onDone]);

  const install = async (id: WizardAppId) => {
    setOpenErr(null);
    try {
      await wizardOpenStore(id);
      set(id, "wait");
    } catch (e) {
      setOpenErr(String(e));
    }
  };

  const store = isMac ? "с сайта разработчика" : "из Microsoft Store";
  const anyMissing = APPS.some(({ id }) => rows[id] === "missing" || rows[id] === "wait");

  return (
    <>
      <WizardTitle
        sub={`Clod Helper подключит их к мосту. Каких нет, можно поставить сейчас ${store}. Мастер сам заметит, когда установка закончится.`}
      >
        Claude Desktop и Codex
      </WizardTitle>

      <ul className="mt-5 divide-y divide-vb-border/60 rounded-xl border border-vb-border bg-vb-surface">
        {APPS.map(({ id, name, what }) => {
          const r = rows[id];
          return (
            <li key={id} className="flex min-h-[60px] items-center gap-3 px-4 py-3" data-app={id}>
              <span className="min-w-0 flex-1">
                <span className="block text-[13px] font-medium text-vb-fg">{name}</span>
                <span className="block text-[12px] text-vb-silver-faint">{what}</span>
              </span>
              <span className="shrink-0" aria-live="polite">
                {r === "missing" ? (
                  <Button size="sm" variant="secondary" onClick={() => void install(id)}>
                    <Download className="h-3.5 w-3.5" aria-hidden />
                    Установить
                  </Button>
                ) : r === "wait" ? (
                  <span className="flex items-center gap-3">
                    <span className="flex items-center gap-1.5 text-[12px] text-vb-silver">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />
                      Жду установку…
                    </span>
                    <button
                      type="button"
                      onClick={() => void install(id)}
                      className="rounded text-[12px] text-vb-silver-faint underline-offset-2 hover:text-vb-silver hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-vb-emerald/50"
                    >
                      Открыть снова
                    </button>
                  </span>
                ) : r === "connect" ? (
                  <span className="flex items-center gap-1.5 text-[12px] text-vb-silver">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />
                    {isMac ? "Подключаю, macOS спросит пароль" : "Подключаю, Windows спросит разрешение"}
                  </span>
                ) : r === "soft" ? (
                  <span className="flex items-center gap-1.5 text-[12px] text-vb-warn">
                    <AlertTriangle className="h-3.5 w-3.5" aria-hidden />
                    Установлено, подключить можно в «Приложениях»
                  </span>
                ) : (
                  <motion.span
                    initial={r === "ok" && !reduce ? { opacity: 0, scale: 0.9 } : false}
                    animate={{ opacity: 1, scale: 1 }}
                    transition={{ type: "spring", bounce: 0, duration: 0.3 }}
                    className={cn(
                      "flex items-center gap-1.5 text-[12px]",
                      r === "ok" ? "text-vb-emerald" : "text-vb-silver-dim",
                    )}
                  >
                    <Check className="h-3.5 w-3.5" strokeWidth={3} aria-hidden />
                    {r === "ok" ? (id === "claude" ? "Установлено и подключено" : "Установлено") : "Уже есть"}
                  </motion.span>
                )}
              </span>
            </li>
          );
        })}
      </ul>

      {openErr && (
        <p role="alert" className="mt-3 text-[12px] text-vb-loss">
          {openErr}
        </p>
      )}

      <WizardFooter>
        <Button variant={anyMissing ? "ghost" : "primary"} onClick={onDone} autoFocus={!anyMissing}>
          {anyMissing ? "Пропустить" : "Дальше"}
          <ArrowRight className="h-4 w-4" aria-hidden />
        </Button>
      </WizardFooter>
    </>
  );
}
