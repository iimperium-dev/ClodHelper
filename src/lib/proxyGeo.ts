import type { ProxyCheckResult } from "./types";

/*
  Адрес прокси для показа и кэш гео-проверок. Лежат отдельно от страницы
  «Прокси»: ими пользуются «Обзор», окна и трей, и им незачем тянуть
  за собой всю страницу.
*/

/** Кэш гео-проверок в localStorage: флаг и страна видны всегда, не только
    после ручного пинга (переживает перезапуски приложения).
    v2: ключи - host:port (логин-пароль в localStorage не пишем). */
const GEO_CACHE_KEY = "clod-proxy-geo-v2";

/** host:port из URL пула (без схемы и логина-пароля - эстетика и ключ кэша). */
export function hostPort(url: string): string {
  const rest = url.includes("://") ? url.split("://")[1] : url;
  return rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest;
}

/** Хост прокси без порта - так его показываем и копируем. */
export function hostOnly(url: string): string {
  const hp = hostPort(url);
  const colon = hp.lastIndexOf(":");
  return colon > 0 ? hp.slice(0, colon) : hp;
}

export function loadGeoCache(): Record<string, ProxyCheckResult> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(GEO_CACHE_KEY) ?? "{}");
    return v && typeof v === "object" && !Array.isArray(v)
      ? (v as Record<string, ProxyCheckResult>)
      : {};
  } catch {
    return {};
  }
}

export function saveGeoCache(cache: Record<string, ProxyCheckResult>) {
  try {
    localStorage.setItem(GEO_CACHE_KEY, JSON.stringify(cache));
  } catch {
    /* ignore */
  }
}
