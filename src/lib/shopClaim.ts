/*
  Купленный на сайте прокси: ссылка clodhelper://add?key=... доходит до
  программы (Rust, src-tauri/src/shop.rs), окно спрашивает человека и только
  по кнопке «Добавить» прокси попадает в пул. Здесь - чистая логика окна.
*/
import { isParseReject, parseProxyInput } from "./proxyParse";

/** Событие окна: пул поменяли не со страницы «Прокси» - ей перечитать его. */
export const POOL_CHANGED_EVENT = "clod:pool-changed";

export interface ClaimProxy {
  url: string;
  socks_url?: string;
  country?: string | null;
  label?: string | null;
  expires_at?: string | null;
}

export interface Claim {
  proxies: ClaimProxy[];
  email_masked?: string | null;
}

/** host:port без схемы и логина-пароля: так прокси показываем и сравниваем. */
export function hostPortOf(url: string): string {
  const rest = url.includes("://") ? url.slice(url.indexOf("://") + 3) : url;
  return (rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest).replace(/\/+$/, "").toLowerCase();
}

/** URL в том же виде, что даёт ручное добавление на странице «Прокси». */
export function normalizeUrl(url: string): string {
  const p = parseProxyInput(url);
  return p && !isParseReject(p) ? p.url : url.trim();
}

/** Код страны из ответа магазина: «FI» - да, «Finland» - нет. */
export function countryCode(country?: string | null): string | null {
  const c = country?.trim() ?? "";
  return /^[A-Za-z]{2}$/.test(c) ? c.toUpperCase() : null;
}

/** Название страны по-русски («FI» - «Финляндия»), иначе как прислал магазин. */
export function countryName(country?: string | null): string | null {
  const c = country?.trim();
  if (!c) return null;
  const code = countryCode(c);
  if (!code) return c;
  try {
    return new Intl.DisplayNames(["ru"], { type: "region" }).of(code) ?? code;
  } catch {
    return code;
  }
}

/** «до 22 декабря 2026» или null, если срока нет или он нечитаемый. */
export function expiryText(iso?: string | null): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  const s = d.toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: "numeric" });
  return `до ${s.replace(/\s*г\.?$/, "")}`;
}

/** Событие окна: в поле прокси вставили ключ магазина - App запросит покупку. */
export const SHOP_KEY_EVENT = "clod:shop-key";

/** Попросить App открыть окно покупки по ключу (как по ссылке). */
export function openShopKey(key: string) {
  window.dispatchEvent(new CustomEvent<string>(SHOP_KEY_EVENT, { detail: key }));
}

/** Ключ: base64url 32 байт без «=» - 43 знака, последний несёт 4 бита. */
const KEY_RE = /^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$/;

/**
 * Ключ магазина из строки в поле прокси: сам ключ или ссылка
 * clodhelper://add?key=... / кабинет .../#key=... Иначе null.
 */
export function shopKeyFrom(raw: string): string | null {
  const t = raw.trim();
  if (KEY_RE.test(t)) return t;
  const m = /^(?:clodhelper:\/\/add\/?\?key=|https:\/\/clodhelper\.com\/buy\/account\/#key=)([A-Za-z0-9_-]{43})$/i.exec(t);
  return m && KEY_RE.test(m[1]) ? m[1] : null;
}

/** Срок прокси из магазина (без ключа), как отдаёт shop_meta_get. */
export interface ShopInfo {
  expires_at?: string | null;
  expired: boolean;
}

/** «до 15 декабря» (год - только если не текущий), null - срока нет. */
export function expiryShort(iso?: string | null, now = new Date()): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  const opts: Intl.DateTimeFormatOptions =
    d.getFullYear() === now.getFullYear() ? { day: "numeric", month: "long" } : { day: "numeric", month: "long", year: "numeric" };
  return `до ${d.toLocaleDateString("ru-RU", opts).replace(/\s*г\.?$/, "")}`;
}

/** Срок прошёл: магазин так сказал или дата уже позади. */
export function isExpired(info?: ShopInfo | null, now = Date.now()): boolean {
  if (!info) return false;
  if (info.expired) return true;
  const t = info.expires_at ? new Date(info.expires_at).getTime() : NaN;
  return !Number.isNaN(t) && t <= now;
}
