import type { ProxyState } from "./status";

/*
  Почему прокси не работает и что с этим сделать: к каждой причине одна
  простая фраза и одно действие. Причины приходят строками из проверки
  прокси (proxies_ping) и из моста, здесь они превращаются в совет.
*/

export type CauseAction = "replace" | "recheck" | "restart" | "addSpare";

export interface ProxyCause {
  /** Что случилось и что делать - одной-двумя короткими фразами. */
  text: string;
  action: CauseAction;
}

export const ACTION_LABEL: Record<CauseAction, string> = {
  replace: "Заменить пароль",
  recheck: "Проверить снова",
  restart: "Перезапустить мост",
  addSpare: "Добавить запасной прокси",
};

export interface CauseContext {
  /** Прокси в России, а через него ходит Claude. */
  inRussia?: boolean;
  /** В пуле есть место для ещё одного прокси. */
  canAddSpare: boolean;
}

/** Прокси отказал в логине или пароле: тут не поможет ни VPN, ни перезапуск. */
export const isAuthReason = (reason?: string) => /логин или пароль|407/i.test(reason ?? "");

/** Причина двумя-тремя словами - для правой колонки строки прокси. */
export function causeLabel(reason?: string): string {
  const r = reason ?? "";
  if (isAuthReason(r)) return "Неверный пароль";
  if (/молчит|блокирует/i.test(r)) return "Молчит";
  if (/порт закрыт|сервер выключен|не удалось подключиться/i.test(r)) return "Сервер недоступен";
  if (/не ответил за/i.test(r)) return "Нет ответа";
  if (/ошибкой 5\d\d/.test(r)) return "Ошибка прокси";
  if (/Мост не может/.test(r)) return "Нет связи с мостом";
  return "Не отвечает";
}

export function causeOf(p: ProxyState, ctx: CauseContext): ProxyCause {
  const spare = (text: string): ProxyCause =>
    ctx.canAddSpare && p.backupOf === undefined ? { text, action: "addSpare" } : { text, action: "recheck" };
  if (ctx.inRussia) {
    return spare("Прокси в России, а Claude из России не работает. Нужен прокси в другой стране.");
  }
  const r = p.reason ?? "";
  if (isAuthReason(r)) {
    return { text: "Прокси не принял логин или пароль. Возьмите новые у продавца прокси и вставьте их.", action: "replace" };
  }
  if (p.state === "unstable") {
    return spare("Часть подключений через этот прокси не проходит. Если это повторяется, добавьте запасной прокси.");
  }
  if (/молчит|блокирует/i.test(r)) {
    return spare(
      "Прокси принимает соединение, но молчит. Скорее всего, ваш провайдер блокирует этот сервер. Попросите у продавца новый IP.",
    );
  }
  if (/порт закрыт|сервер выключен|не удалось подключиться|не ответил за/i.test(r)) {
    return {
      text: "Сервер прокси не отвечает. Он выключен, сменил адрес или закончилась оплата. Уточните у продавца.",
      action: "recheck",
    };
  }
  if (/ошибкой 5\d\d/.test(r)) {
    return { text: "Прокси на связи, но сам не смог открыть сайт. Обычно это проходит за несколько минут.", action: "recheck" };
  }
  if (/Мост не может/.test(r)) {
    return { text: "Мост не может подключиться через этот прокси. Обычно помогает перезапуск моста.", action: "restart" };
  }
  return { text: r ? `${r.replace(/\.$/, "")}.` : "Прокси не отвечает.", action: "recheck" };
}

/* ── Переход в «Прокси» с готовым действием ── */

export type ProxyIntent = { kind: "replace"; url: string } | { kind: "add" };

let pending: ProxyIntent | null = null;

/** «Обзор» просит «Прокси» сразу открыть замену пароля или добавление. */
export function setProxyIntent(i: ProxyIntent) {
  pending = i;
}

/** Забрать просьбу (один раз). */
export function takeProxyIntent(): ProxyIntent | null {
  const i = pending;
  pending = null;
  return i;
}
