import type { UpdateInfo } from "./types";
import whatsNew from "../whats-new.json";
import { isMac } from "./platform";

/*
  Открыть окно обновления из любого места (баннер в Обзоре и Настройках,
  уведомление в углу экрана — через событие Tauri "update:open").
*/
const OPEN = "clod:update-open";

export function openUpdateDialog(info: UpdateInfo): void {
  window.dispatchEvent(new CustomEvent<UpdateInfo>(OPEN, { detail: info }));
}

export function onOpenUpdateDialog(fn: (info: UpdateInfo) => void): () => void {
  const h = (e: Event) => fn((e as CustomEvent<UpdateInfo>).detail);
  window.addEventListener(OPEN, h);
  return () => window.removeEventListener(OPEN, h);
}

/** Пункт «Что нового»: строка — для всех ОС, объект — только для своих. */
type WhatsNewItem = string | { text: string; platforms?: ("mac" | "windows")[] };

/** Пункты «Что нового» версии для этой ОС из src/whats-new.json (его же CI
    кладёт в релиз и в манифест обновления — по ОС). */
export function whatsNewItems(version: string): string[] {
  const entry = (whatsNew as Record<string, { items?: WhatsNewItem[] }>)[version];
  const os = isMac ? "mac" : "windows";
  return (entry?.items ?? [])
    .filter((i) => typeof i === "string" || (i.platforms ?? []).includes(os))
    .map((i) => (typeof i === "string" ? i : i.text));
}

/** 8 123 456 → «7,7 МБ». */
export function formatMb(bytes: number): string {
  return `${(bytes / 1024 / 1024).toFixed(1).replace(".", ",")} МБ`;
}
