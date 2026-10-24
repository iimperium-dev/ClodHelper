import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import {
  AlertTriangle,
  Check,
  ChevronDown,
  Eraser,
  Fingerprint,
  Globe2,
  HardDrive,
  KeyRound,
  Loader2,
  MessagesSquare,
  Monitor,
  ScrollText,
  ShieldCheck,
  SquareTerminal,
  UserRound,
} from "lucide-react";
import { Button } from "../components/Button";
import { Toggle } from "../components/Toggle";
import { appsLaunch, claudeWipeRun, claudeWipeScan } from "../lib/api";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { playWipeSound, unlockWipeSound } from "../lib/wipeSound";
import { isMac } from "../lib/platform";
import { cn } from "../lib/cn";
import type { WipeGroup, WipeResult, WipeScan } from "../lib/types";

/** Размер по-человечески: 236 МБ, 4,2 МБ, 85 КБ. */
function formatSize(bytes: number): string {
  if (bytes < 1024) return bytes > 0 ? "< 1 КБ" : "";
  const units = ["КБ", "МБ", "ГБ"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  const n = v >= 10 ? Math.round(v).toString() : v.toFixed(1).replace(".", ",");
  return `${n} ${units[i]}`;
}

function plural(n: number, one: string, few: string, many: string): string {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}

const GROUPS: Record<WipeGroup, { label: string; sub: string; icon: typeof KeyRound; step: string }> = {
  login: { label: "Вход в аккаунт", sub: "Куки и токены — Claude попросит войти заново", icon: KeyRound, step: "Удаляю вход и куки" },
  device: {
    label: "Идентификаторы устройства",
    sub: "Постоянные id установки, по которым узнают этот компьютер",
    icon: Fingerprint,
    step: "Удаляю идентификаторы устройства",
  },
  web: { label: "Данные сайта claude.ai", sub: "Хранилище страницы внутри программы", icon: Globe2, step: "Очищаю данные claude.ai" },
  account: {
    label: "Привязка к аккаунту",
    sub: "Лимиты, плагины организации, удалённый доступ",
    icon: UserRound,
    step: "Отвязываю аккаунт",
  },
  logs: { label: "Журналы и отчёты об ошибках", sub: "В них почта и id аккаунта", icon: ScrollText, step: "Удаляю журналы" },
  cache: { label: "Кэш", sub: "Страницы и графика — создастся заново", icon: HardDrive, step: "Очищаю кэш" },
  chats: {
    label: "Локальные чаты и проекты",
    sub: "Переписка Claude Code и задачи Cowork — безвозвратно",
    icon: MessagesSquare,
    step: "Удаляю локальные чаты",
  },
};
const ORDER: WipeGroup[] = ["login", "device", "web", "account", "logs", "cache"];

const EASE = [0.22, 1, 0.36, 1] as const;
/** Раскрытие блока по высоте (общая анимация для всех выезжающих частей). */
const reveal = {
  initial: { height: 0, opacity: 0 },
  animate: { height: "auto", opacity: 1, transition: { height: { duration: 0.32, ease: EASE }, opacity: { duration: 0.2, delay: 0.06 } } },
  exit: { height: 0, opacity: 0, transition: { height: { duration: 0.24, ease: EASE }, opacity: { duration: 0.12 } } },
};

/** Подсказку с подробностями открываем при первом нажатии ластика — дальше можно не раскрывать. */
const SEEN_KEY = "clod.wipe.seen";
const seenBefore = () => {
  try {
    return localStorage.getItem(SEEN_KEY) === "1";
  } catch {
    return true;
  }
};
const markSeen = () => {
  try {
    localStorage.setItem(SEEN_KEY, "1");
  } catch {
    /* приватный режим — просто откроем подробности ещё раз */
  }
};

type Phase = "idle" | "confirm" | "working" | "done" | "error";
type Panel = "remove" | "keep" | "limits";

/** Строка-переключатель «что очищать». */
function TargetRow({
  icon: Icon,
  title,
  sub,
  checked,
  disabled,
  danger,
  onChange,
}: {
  icon: typeof Monitor;
  title: string;
  sub: string;
  checked: boolean;
  disabled?: boolean;
  /** Включение необратимо (удаление чатов) — красный акцент. */
  danger?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className={cn("flex items-center gap-3 px-3.5 py-3", disabled && "opacity-50")}>
      <span
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border transition-colors duration-200",
          checked && !disabled
            ? danger
              ? "border-vb-loss/35 bg-vb-loss/10 text-vb-loss"
              : "border-vb-emerald/30 bg-vb-emerald/10 text-vb-emerald"
            : "border-vb-border bg-vb-surface-2 text-vb-silver-faint",
        )}
      >
        <Icon className="h-4 w-4" strokeWidth={1.9} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] font-medium leading-tight text-vb-fg">{title}</span>
        <span className="mt-0.5 block truncate text-[11px] leading-tight text-vb-silver-faint">{sub}</span>
      </span>
      <Toggle
        checked={checked && !disabled}
        onChange={onChange}
        disabled={disabled}
        tone={danger ? "loss" : "emerald"}
        label={`Очистить ${title}`}
      />
    </div>
  );
}

/** Раскрывающийся пункт: «Что удалится», «Что останется», «Чего очистка не меняет». */
function Disclosure({
  title,
  hint,
  open,
  onToggle,
  children,
}: {
  title: string;
  hint?: string;
  open: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <div>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-3.5 py-2.5 text-left transition-colors hover:bg-vb-surface-2/60"
      >
        <span className="flex-1 text-[12.5px] font-medium text-vb-silver">{title}</span>
        {hint && <span className="tnum text-[11px] text-vb-silver-faint">{hint}</span>}
        <ChevronDown
          className={cn("h-3.5 w-3.5 text-vb-silver-faint transition-transform duration-300", open && "rotate-180")}
        />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div {...reveal} className="overflow-hidden">
            <div className="px-3.5 pb-3">{children}</div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Шаги для полосы прогресса: по тому, что реально будет удаляться. */
function stepsFor(scan: WipeScan | null, desktop: boolean, code: boolean, chats: boolean): string[] {
  const steps: string[] = [];
  if ((desktop || chats) && scan?.desktop_running) steps.push("Закрываю Claude");
  if (desktop) for (const id of ORDER) if (scan?.groups.some((g) => !g.code && g.id === id)) steps.push(GROUPS[id].step);
  if (code && scan?.groups.some((g) => g.code)) steps.push("Очищаю Claude Code");
  if (chats) steps.push(GROUPS.chats.step);
  if (steps.length === 0) steps.push("Очищаю");
  return steps;
}

/** Анимированная полоса: идёт по шагам, пока работает очистка; в конце — 100%. */
function WipeProgress({ steps, finished }: { steps: string[]; finished: boolean }) {
  const reduce = useReducedMotion();
  const [p, setP] = useState(0);
  useEffect(() => {
    if (finished) {
      setP(100);
      return;
    }
    // Доходим до ~92% за время, пропорциональное числу шагов, дальше — ждём ответ.
    const start = performance.now();
    const total = 650 * steps.length;
    let raf = 0;
    const tick = (t: number) => {
      const k = Math.min(1, (t - start) / total);
      setP(92 * (1 - Math.pow(1 - k, 2.2)));
      if (k < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [finished, steps.length]);
  const idx = finished ? steps.length - 1 : Math.min(steps.length - 1, Math.floor((p / 92) * steps.length));
  return (
    <div>
      <div className="flex items-baseline justify-between gap-3">
        <AnimatePresence mode="wait" initial={false}>
          <motion.span
            key={finished ? "done" : idx}
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.18 }}
            className="truncate text-[12.5px] text-vb-silver"
          >
            {finished ? "Готово" : `${steps[idx]}…`}
          </motion.span>
        </AnimatePresence>
        <span className="tnum shrink-0 text-[11px] text-vb-silver-faint">{Math.round(p)}%</span>
      </div>
      <div className="relative mt-2 h-1.5 overflow-hidden rounded-full bg-white/[0.06]">
        <motion.div
          className="absolute inset-y-0 left-0 overflow-hidden rounded-full bg-gradient-to-r from-vb-emerald-dim to-vb-emerald shadow-[0_0_12px_var(--color-vb-emerald-glow)]"
          animate={{ width: `${p}%` }}
          transition={{ duration: finished ? 0.35 : 0.12, ease: "easeOut" }}
        >
          {!reduce && !finished && (
            <motion.span
              className="absolute inset-y-0 w-1/3 bg-gradient-to-r from-transparent via-white/35 to-transparent"
              initial={{ x: "-100%" }}
              animate={{ x: "300%" }}
              transition={{ duration: 1.1, repeat: Infinity, ease: "easeInOut" }}
            />
          )}
        </motion.div>
      </div>
    </div>
  );
}

/*
  Очистка данных аккаунта Claude — чтобы следующий аккаунт вошёл как с
  нового компьютера. Карточка свёрнута: заголовок и ластик справа. Ластик —
  подтверждение, затем полоса прогресса прямо в карточке. Нажатие на саму
  карточку раскрывает настройки: что очищать (Desktop / Code) и три
  раскрывающихся пункта — что удалится, что останется, чего очистка не
  меняет. При первом нажатии ластика подробности раскрываются сами.
*/
function ClaudeWipeCard({ onOpenApps }: { onOpenApps: () => void }) {
  const [scan, setScan] = useState<WipeScan | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [panel, setPanel] = useState<Panel | null>(null);
  const [desktopOn, setDesktopOn] = useState(true);
  const [codeOn, setCodeOn] = useState(true);
  // Локальные чаты по умолчанию остаются: удаление — только осознанным выбором.
  const [chatsOn, setChatsOn] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [steps, setSteps] = useState<string[]>([]);
  const [finished, setFinished] = useState(false);
  const [result, setResult] = useState<WipeResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [launching, setLaunching] = useState(false);
  const [launchError, setLaunchError] = useState<string | null>(null);
  const alive = useRef(true);

  const load = useCallback(async () => {
    try {
      setScan(await claudeWipeScan());
      setScanError(null);
    } catch (e) {
      setScanError(String(e));
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void load();
    // Запущен ли Claude — меняется, пока человек читает.
    const t = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, 5000);
    return () => {
      alive.current = false;
      window.clearInterval(t);
    };
  }, [load]);

  const desktopGroups = scan?.groups.filter((g) => !g.code && g.id !== "chats") ?? [];
  const chatsGroup = scan?.groups.find((g) => g.id === "chats");
  const chatsBytes = chatsGroup?.bytes ?? 0;
  const codeGroups = scan?.groups.filter((g) => g.code) ?? [];
  const desktopBytes = desktopGroups.reduce((s, g) => s + g.bytes, 0);
  const codeBytes = codeGroups.reduce((s, g) => s + g.bytes, 0);
  const hasDesktop = !!scan && scan.desktop_dirs.length > 0;
  const hasCode = !!scan?.code_found;
  const useDesktop = desktopOn && hasDesktop;
  const useCode = codeOn && hasCode;
  const useChats = chatsOn && !!chatsGroup;
  const total = (useDesktop ? desktopBytes : 0) + (useCode ? codeBytes : 0) + (useChats ? chatsBytes : 0);
  const nothing =
    !!scan &&
    (useDesktop ? desktopGroups.length === 0 : true) &&
    (useCode ? codeGroups.length === 0 : true) &&
    !useChats;
  const canWipe = !!scan && (useDesktop || useCode || useChats) && !nothing;
  const busy = phase === "working";

  const onEraser = () => {
    if (busy) return;
    if (phase === "confirm") {
      setPhase("idle");
      return;
    }
    if (!seenBefore()) {
      markSeen();
      setExpanded(true);
      setPanel("remove");
    }
    setError(null);
    setPhase("confirm");
  };

  const run = async () => {
    // Звук «готово» сыграет позже — аудио разблокируем сейчас, на нажатии.
    unlockWipeSound();
    const plan = stepsFor(scan, useDesktop, useCode, useChats);
    setSteps(plan);
    setFinished(false);
    setResult(null);
    setError(null);
    setPhase("working");
    const started = Date.now();
    try {
      const r = await claudeWipeRun(useDesktop, useCode, useChats);
      // Полоса не должна проскочить за долю секунды — даём ей пройти шаги.
      const minMs = 650 * plan.length * 0.8;
      const wait = Math.max(0, minMs - (Date.now() - started));
      await new Promise((ok) => setTimeout(ok, wait));
      if (!alive.current) return;
      setResult(r);
      setFinished(true);
      playWipeSound();
      await new Promise((ok) => setTimeout(ok, 650));
      if (!alive.current) return;
      setPhase("done");
    } catch (e) {
      setError(String(e));
      setPhase("error");
    } finally {
      void load();
    }
  };

  const openClaude = async () => {
    setLaunching(true);
    setLaunchError(null);
    try {
      await appsLaunch("claude-desktop");
    } catch (e) {
      setLaunchError(String(e));
    } finally {
      setLaunching(false);
    }
  };

  const confirmText = (() => {
    const what =
      useDesktop && useCode ? "в Claude Desktop и Claude Code" : useDesktop ? "в Claude Desktop" : "в Claude Code";
    const close = (useDesktop || useChats) && scan?.desktop_running ? "Claude закроется, и вы выйдете" : "Вы выйдете";
    const chats = useChats ? " Локальные чаты и проекты удалятся безвозвратно." : "";
    if (!useDesktop && !useCode) {
      const shut = scan?.desktop_running ? "Claude закроется. " : "";
      return `${shut}Локальные чаты и проекты удалятся безвозвратно. Отменить это нельзя.`;
    }
    return `${close} из аккаунта ${what}.${chats} Отменить это нельзя.`;
  })();

  return (
    <motion.section variants={fadeInUp} className="surface-card overflow-hidden">
      {/* ── Заголовок: по клику раскрывает настройки; справа — ластик ── */}
      <div className="flex items-center gap-3 p-3.5">
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          aria-expanded={expanded}
          className="group flex min-w-0 flex-1 items-center gap-3 rounded-lg text-left"
        >
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-vb-border bg-vb-surface-2 text-vb-silver transition-colors group-hover:border-vb-border-strong">
            <UserRound className="h-[18px] w-[18px]" strokeWidth={1.8} />
          </span>
          <span className="min-w-0 flex-1">
            <span className="flex items-center gap-1.5">
              <span className="text-[14px] font-semibold leading-tight text-vb-fg">Очистка аккаунта Claude</span>
              <ChevronDown
                className={cn(
                  "h-3.5 w-3.5 text-vb-silver-faint transition-transform duration-300 group-hover:text-vb-silver",
                  expanded && "rotate-180",
                )}
              />
            </span>
            <span className="mt-0.5 block truncate text-[12px] leading-snug text-vb-silver-faint">
              {!scan
                ? "Ищу данные Claude…"
                : nothing
                  ? "Уже чисто — следов прошлого аккаунта нет"
                  : `Следующий аккаунт войдёт как с нового компьютера · ${formatSize(total)}`}
            </span>
          </span>
        </button>
        <motion.button
          type="button"
          onClick={onEraser}
          disabled={!canWipe || busy}
          whileTap={{ scale: 0.92 }}
          title="Очистить"
          aria-label="Очистить данные аккаунта Claude"
          className={cn(
            "relative flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border transition-colors duration-200",
            "disabled:cursor-not-allowed disabled:opacity-35",
            phase === "confirm"
              ? "border-vb-loss/60 bg-vb-loss/20 text-vb-loss"
              : "border-vb-loss/30 bg-vb-loss/10 text-vb-loss hover:border-vb-loss/50 hover:bg-vb-loss/15",
          )}
        >
          {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Eraser className="h-4 w-4" strokeWidth={2} />}
        </motion.button>
      </div>

      {/* ── Подтверждение / прогресс / результат — выезжают под заголовком ── */}
      <AnimatePresence initial={false} mode="wait">
        {phase === "confirm" && (
          <motion.div key="confirm" {...reveal} className="overflow-hidden">
            <div className="mx-3.5 mb-3.5 rounded-xl border border-vb-loss/30 bg-vb-loss/[0.06] p-3">
              <div className="flex gap-2.5">
                <AlertTriangle className="mt-px h-4 w-4 shrink-0 text-vb-loss" />
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium text-vb-fg">Точно очистить?</div>
                  <p className="mt-0.5 text-[12px] leading-snug text-vb-silver">{confirmText}</p>
                </div>
              </div>
              <div className="mt-3 flex gap-2 pl-[26px]">
                <Button variant="danger" size="sm" onClick={() => void run()}>
                  <Eraser className="h-3.5 w-3.5" />
                  Да, очистить{total > 0 ? ` ${formatSize(total)}` : ""}
                </Button>
                <Button variant="ghost" size="sm" onClick={() => setPhase("idle")}>
                  Отмена
                </Button>
              </div>
              {useCode && scan && scan.cli_running > 0 && (
                <p className="mt-2.5 flex gap-1.5 pl-[26px] text-[11.5px] leading-snug text-vb-warn">
                  <AlertTriangle className="mt-px h-3.5 w-3.5 shrink-0" />
                  Claude Code запущен отдельно ({isMac ? "в Терминале" : "в терминале"} или редакторе) — закройте
                  его, иначе он запишет данные аккаунта обратно.
                </p>
              )}
            </div>
          </motion.div>
        )}

        {phase === "working" && (
          <motion.div key="working" {...reveal} className="overflow-hidden">
            <div className="mx-3.5 mb-3.5 rounded-xl border border-vb-border bg-vb-surface-2/50 p-3">
              <WipeProgress steps={steps} finished={finished} />
            </div>
          </motion.div>
        )}

        {phase === "done" && result && (
          <motion.div key="done" {...reveal} className="overflow-hidden">
            <div className="mx-3.5 mb-3.5 rounded-xl border border-vb-emerald/25 bg-vb-emerald/[0.06] p-3">
              <div className="flex items-center gap-2.5">
                <motion.span
                  initial={{ scale: 0.6, opacity: 0 }}
                  animate={{ scale: 1, opacity: 1 }}
                  transition={{ type: "spring", bounce: 0.35, duration: 0.5 }}
                  className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-vb-emerald/15 text-vb-emerald"
                >
                  <Check className="h-4 w-4" strokeWidth={2.6} />
                </motion.span>
                <div className="min-w-0">
                  <div className="text-[13px] font-semibold text-vb-fg">
                    Готово{result.removed_bytes > 0 ? ` — удалено ${formatSize(result.removed_bytes)}` : ""}
                  </div>
                  <div className="text-[11.5px] text-vb-silver-dim">
                    {result.closed_desktop ? "Claude был закрыт. " : ""}Откройте его и войдите в новый аккаунт
                  </div>
                </div>
              </div>

              {result.failed.length > 0 && (
                <div className="mt-2.5 rounded-lg border border-vb-warn/30 bg-vb-warn/[0.06] p-2.5 text-[11.5px] leading-snug text-vb-silver">
                  <span className="font-medium text-vb-warn">
                    Не удалось удалить {result.failed.length}{" "}
                    {plural(result.failed.length, "элемент", "элемента", "элементов")}
                  </span>{" "}
                  — их держит запущенная программа. Закройте Claude и Claude Code и очистите ещё раз.
                </div>
              )}

              <p className="mt-2.5 text-[11.5px] leading-snug text-vb-silver-dim">
                Чтобы новый аккаунт не совпадал с прошлым и по IP, перед входом выберите для Claude другой прокси.{" "}
                <button
                  type="button"
                  onClick={onOpenApps}
                  className="font-medium text-vb-emerald underline-offset-2 hover:underline"
                >
                  Выбрать
                </button>
              </p>

              <div className="mt-3 flex flex-wrap gap-2">
                {useDesktop && (
                  <Button variant="primary" size="sm" onClick={() => void openClaude()} disabled={launching}>
                    {launching && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                    Открыть Claude
                  </Button>
                )}
                {result.failed.length > 0 ? (
                  <Button variant="secondary" size="sm" onClick={() => void run()}>
                    Очистить ещё раз
                  </Button>
                ) : (
                  <Button variant="ghost" size="sm" onClick={() => setPhase("idle")}>
                    Закрыть
                  </Button>
                )}
              </div>
              {launchError && <p className="mt-2 text-[12px] leading-snug text-vb-loss">{launchError}</p>}
            </div>
          </motion.div>
        )}

        {phase === "error" && error && (
          <motion.div key="error" {...reveal} className="overflow-hidden">
            <div className="mx-3.5 mb-3.5 rounded-xl border border-vb-loss/30 bg-vb-loss/[0.06] p-3 text-[12px] leading-snug text-vb-silver">
              <div className="font-medium text-vb-loss">Не получилось</div>
              <p className="mt-0.5">{error}</p>
              <div className="mt-2.5 flex gap-2">
                <Button variant="secondary" size="sm" onClick={() => void run()}>
                  Попробовать ещё раз
                </Button>
                <Button variant="ghost" size="sm" onClick={() => setPhase("idle")}>
                  Закрыть
                </Button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Подробности: что очищать и что будет ── */}
      <AnimatePresence initial={false}>
        {expanded && (
          <motion.div {...reveal} className="overflow-hidden">
            <div className="space-y-2.5 border-t border-vb-border/70 p-3.5">
              {scanError ? (
                <p className="text-[12px] text-vb-loss">{scanError}</p>
              ) : !scan ? (
                <div className="flex items-center gap-2 py-2 text-[12px] text-vb-silver-dim">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" /> Ищу данные Claude…
                </div>
              ) : (
                <>
                  {/* Что очищать */}
                  <div className="divide-y divide-vb-border/60 overflow-hidden rounded-xl bg-vb-surface-2/50">
                    <TargetRow
                      icon={Monitor}
                      title="Claude Desktop"
                      sub={
                        !hasDesktop
                          ? "Не найден на этом компьютере"
                          : `Вход, id устройства, данные claude.ai, кэш${desktopBytes ? ` · ${formatSize(desktopBytes)}` : ""}`
                      }
                      checked={desktopOn}
                      disabled={!hasDesktop || busy}
                      onChange={setDesktopOn}
                    />
                    <TargetRow
                      icon={SquareTerminal}
                      title="Claude Code"
                      sub={
                        !hasCode
                          ? "Не найден на этом компьютере"
                          : `Вход, id установки, телеметрия${codeBytes ? ` · ${formatSize(codeBytes)}` : ""}`
                      }
                      checked={codeOn}
                      disabled={!hasCode || busy}
                      onChange={setCodeOn}
                    />
                    <TargetRow
                      icon={MessagesSquare}
                      title="Локальные чаты и проекты"
                      sub={
                        !chatsGroup
                          ? "Нет на этом компьютере"
                          : `Переписка Code и Cowork — безвозвратно${chatsBytes ? ` · ${formatSize(chatsBytes)}` : ""}`
                      }
                      checked={chatsOn}
                      disabled={!chatsGroup || busy}
                      danger
                      onChange={setChatsOn}
                    />
                  </div>

                  {/* Подробности */}
                  <div className="divide-y divide-vb-border/60 overflow-hidden rounded-xl bg-vb-surface-2/50">
                    <Disclosure
                      title="Что удалится"
                      hint={total > 0 ? formatSize(total) : undefined}
                      open={panel === "remove"}
                      onToggle={() => setPanel((p) => (p === "remove" ? null : "remove"))}
                    >
                      {!useDesktop && !useCode && !useChats ? (
                        <p className="text-[12px] text-vb-silver-dim">Ничего не выбрано.</p>
                      ) : (
                        <ul className="space-y-0.5">
                          {useDesktop &&
                            ORDER.map((id) => {
                              const g = desktopGroups.find((x) => x.id === id);
                              const meta = GROUPS[id];
                              const Icon = meta.icon;
                              return (
                                <li key={id} className={cn("flex items-center gap-2.5 py-1.5", !g && "opacity-45")}>
                                  <Icon className="h-3.5 w-3.5 shrink-0 text-vb-silver-dim" strokeWidth={1.8} />
                                  <div className="min-w-0 flex-1">
                                    <div className="text-[12px] leading-tight text-vb-silver">{meta.label}</div>
                                    <div className="truncate text-[11px] leading-tight text-vb-silver-faint">
                                      {g ? meta.sub : "уже чисто"}
                                    </div>
                                  </div>
                                  {g && g.bytes > 0 && (
                                    <span className="tnum shrink-0 text-[11px] text-vb-silver-faint">
                                      {formatSize(g.bytes)}
                                    </span>
                                  )}
                                </li>
                              );
                            })}
                          {useCode && (
                            <li className={cn("flex items-center gap-2.5 py-1.5", codeGroups.length === 0 && "opacity-45")}>
                              <SquareTerminal className="h-3.5 w-3.5 shrink-0 text-vb-silver-dim" strokeWidth={1.8} />
                              <div className="min-w-0 flex-1">
                                <div className="text-[12px] leading-tight text-vb-silver">Claude Code</div>
                                <div className="truncate text-[11px] leading-tight text-vb-silver-faint">
                                  {codeGroups.length ? "Вход, id установки и телеметрия" : "уже чисто"}
                                </div>
                              </div>
                              {codeBytes > 0 && (
                                <span className="tnum shrink-0 text-[11px] text-vb-silver-faint">
                                  {formatSize(codeBytes)}
                                </span>
                              )}
                            </li>
                          )}
                          {useChats && (
                            <li className="flex items-center gap-2.5 py-1.5">
                              <MessagesSquare className="h-3.5 w-3.5 shrink-0 text-vb-loss" strokeWidth={1.8} />
                              <div className="min-w-0 flex-1">
                                <div className="text-[12px] leading-tight text-vb-loss">Локальные чаты и проекты</div>
                                <div className="truncate text-[11px] leading-tight text-vb-silver-faint">
                                  Переписка Claude Code и задачи Cowork — безвозвратно
                                </div>
                              </div>
                              {chatsBytes > 0 && (
                                <span className="tnum shrink-0 text-[11px] text-vb-silver-faint">
                                  {formatSize(chatsBytes)}
                                </span>
                              )}
                            </li>
                          )}
                        </ul>
                      )}
                    </Disclosure>

                    <Disclosure
                      title="Что останется"
                      hint="чаты, проекты, настройки"
                      open={panel === "keep"}
                      onToggle={() => setPanel((p) => (p === "keep" ? null : "keep"))}
                    >
                      <ul className="space-y-1.5 text-[12px] leading-snug text-vb-silver-dim">
                        {!useChats && (
                          <li className="flex gap-2">
                            <SquareTerminal className="mt-px h-3.5 w-3.5 shrink-0 text-vb-emerald" strokeWidth={1.9} />
                            <span>
                              <span className="text-vb-silver">
                                Локальные чаты и проекты{scan.kept_sessions > 0 ? ` (${scan.kept_sessions})` : ""}
                              </span>{" "}
                              — они хранятся на этом компьютере и не удаляются
                            </span>
                          </li>
                        )}
                        <li className="flex gap-2">
                          <MessagesSquare className="mt-px h-3.5 w-3.5 shrink-0 text-vb-emerald" strokeWidth={1.9} />
                          <span>
                            <span className="text-vb-silver">Облачные чаты и проекты</span> — они хранятся в самом
                            аккаунте у Anthropic и остаются в нём
                          </span>
                        </li>
                        <li className="flex gap-2">
                          <ShieldCheck className="mt-px h-3.5 w-3.5 shrink-0 text-vb-emerald" strokeWidth={1.9} />
                          <span>
                            <span className="text-vb-silver">Задачи Cowork</span> — остаются на компьютере и вернутся
                            при входе в прошлый аккаунт
                          </span>
                        </li>
                        <li className="flex gap-2">
                          <Check className="mt-px h-3.5 w-3.5 shrink-0 text-vb-emerald" strokeWidth={2.2} />
                          <span>
                            <span className="text-vb-silver">Настройки MCP, расширения, история Claude Code</span> и
                            сама программа
                          </span>
                        </li>
                      </ul>
                    </Disclosure>

                    <Disclosure
                      title="Чего очистка не меняет"
                      open={panel === "limits"}
                      onToggle={() => setPanel((p) => (p === "limits" ? null : "limits"))}
                    >
                      <ul className="space-y-1.5 text-[12px] leading-snug text-vb-silver-dim">
                        <li>
                          <span className="text-vb-silver">IP-адрес.</span> Для нового аккаунта выберите в настройках
                          Claude другой прокси.
                        </li>
                        <li>
                          <span className="text-vb-silver">Сам компьютер.</span> Имя компьютера, видеокарта, экран,
                          процессор и система — их файлами не сменить. Отчёты Claude об ошибках с этими данными
                          отсекает «Не отправлять телеметрию» в настройках Claude Desktop.
                        </li>
                        <li>
                          <span className="text-vb-silver">Почта, телефон и карта.</span> Если они те же, Anthropic
                          свяжет аккаунты на своей стороне.
                        </li>
                      </ul>
                    </Disclosure>
                  </div>
                </>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.section>
  );
}

/*
  Инструменты: разовые действия, которые не относятся к мосту напрямую.
  Первый — очистка данных аккаунта Claude для смены аккаунта.
*/
export function Tools({ onOpenApps }: { onOpenApps: () => void }) {
  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[560px] flex-col px-6 py-6"
    >
      <motion.header variants={fadeInUp}>
        <h1 className="text-[20px] font-semibold leading-tight tracking-[-0.02em] text-vb-fg">Инструменты</h1>
        <p className="mt-1 text-[12.5px] text-vb-silver-faint">Разовые действия для Claude</p>
      </motion.header>
      <div className="mt-5">
        <ClaudeWipeCard onOpenApps={onOpenApps} />
      </div>
    </motion.div>
  );
}
