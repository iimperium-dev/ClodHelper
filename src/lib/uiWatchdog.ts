import { invoke } from "@tauri-apps/api/core";
import { telemetryUiError } from "./api";

/*
  Сторож окна. Изредка (после часов работы) WebKit оставлял раздел
  прозрачным: анимация появления не доигрывалась, и «Обзор»/«Прокси»
  выглядели чёрными. Сторож раз в секунду проверяет, видна ли текущая
  страница; если она прозрачна дольше 2 с - пишет диагностику в журнал
  (ui.log) и доигрывает анимации, чтобы содержимое появилось. Заодно
  в журнал уходят ошибки JS.
*/

const LOW = 0.05;
let rafTicks = 0;
let lastLog = "";

function log(level: string, message: string) {
  // Одинаковые подряд не пишем - журнал не должен разрастаться.
  if (message === lastLog) return;
  lastLog = message;
  void invoke("ui_log", { level, message }).catch(() => {});
}

/** Идёт отправка отчёта: ошибки самой отправки снова не отправляем. */
let reporting = false;

/** Ошибка интерфейса - в отчёты о сбоях (бэкенд сам проверяет переключатель и лимиты). */
function reportUi(message: string, stack: string) {
  if (reporting) return;
  reporting = true;
  try {
    void telemetryUiError(message.slice(0, 2000), stack.slice(0, 8000)).catch(() => {});
  } catch {
    /* отчёт - не повод ронять интерфейс */
  } finally {
    reporting = false;
  }
}

const reasonText = (r: unknown): [string, string] =>
  r instanceof Error ? [r.message || r.name, r.stack ?? ""] : [String(r), ""];

const opacity = (el: Element) => Number.parseFloat(getComputedStyle(el).opacity || "1");

/** Прозрачна ли страница: сам контейнер или все его секции. */
function pageInvisible(page: HTMLElement): boolean {
  const chain: Element[] = [page];
  let inner = page.firstElementChild;
  if (inner) chain.push(inner);
  if (chain.some((el) => opacity(el) < LOW)) return true;
  const sections = inner ? Array.from(inner.children) : [];
  return sections.length > 0 && sections.every((el) => opacity(el) < LOW);
}

function describeAnimations(): string {
  const all = document.getAnimations();
  const head = all.slice(0, 6).map((a) => {
    const target = (a.effect as KeyframeEffect | null)?.target as HTMLElement | null;
    return `${a.playState}@${Math.round(Number(a.currentTime ?? -1))}:${target?.tagName.toLowerCase() ?? "?"}`;
  });
  return `anims=${all.length} [${head.join(", ")}]`;
}

function heal(page: HTMLElement) {
  for (const a of document.getAnimations()) {
    try {
      a.finish();
    } catch {
      /* бесконечные анимации finish не умеют - пропускаем */
    }
  }
  // Значения, оставленные анимацией в style, - к конечному виду.
  const els = [page, ...Array.from(page.querySelectorAll<HTMLElement>("*"))].slice(0, 400);
  for (const el of els) {
    if (el.style.opacity !== "" && Number.parseFloat(el.style.opacity) < LOW && opacity(el) < LOW) {
      el.style.opacity = "1";
      el.style.transform = "none";
    }
  }
}

export function startUiWatchdog() {
  const tick = () => {
    rafTicks++;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);

  window.addEventListener("error", (e) => {
    log("error", `js: ${e.message} @ ${e.filename}:${e.lineno}`);
    const stack = e.error instanceof Error && e.error.stack ? e.error.stack : `at ${e.filename}:${e.lineno}:${e.colno}`;
    reportUi(e.message, stack);
  });
  window.addEventListener("unhandledrejection", (e) => {
    log("error", `promise: ${String(e.reason)}`);
    const [msg, stack] = reasonText(e.reason);
    reportUi(msg, stack);
  });
  const origError = console.error;
  console.error = (...args: unknown[]) => {
    log("error", `console: ${args.map(String).join(" ").slice(0, 500)}`);
    origError(...args);
  };

  let stuckSince = 0;
  let lastRaf = 0;
  window.setInterval(() => {
    const raf = rafTicks - lastRaf;
    lastRaf = rafTicks;
    const page = document.querySelector("main")?.firstElementChild as HTMLElement | null;
    if (!page || document.visibilityState !== "visible" || !pageInvisible(page)) {
      stuckSince = 0;
      return;
    }
    const now = Date.now();
    if (!stuckSince) {
      stuckSince = now;
      return;
    }
    if (now - stuckSince < 2000) return;
    const before = `hidden=${document.hidden} raf/s=${raf} page.op=${opacity(page)} ${describeAnimations()} view=${location.hash || location.search}`;
    heal(page);
    log("warn", `невидимая страница - восстановлено. ${before} → page.op=${opacity(page)}`);
    reportUi("Окно зависло: страница прозрачна больше 2 секунд", before);
    stuckSince = 0;
  }, 1000);
}
