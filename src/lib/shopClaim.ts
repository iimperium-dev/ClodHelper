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
