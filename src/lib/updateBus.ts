import type { UpdateInfo } from "./types";
import whatsNew from "../whats-new.json";

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

/** Пункты «Что нового» версии из src/whats-new.json (его же CI кладёт в релиз). */
export function whatsNewItems(version: string): string[] {
  const entry = (whatsNew as Record<string, { items?: string[] }>)[version];
  return entry?.items ?? [];
}

/** 8 123 456 → «7,7 МБ». */
export function formatMb(bytes: number): string {
  return `${(bytes / 1024 / 1024).toFixed(1).replace(".", ",")} МБ`;
}
