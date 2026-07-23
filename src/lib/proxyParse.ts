/*
  Умный парсер прокси-строк: принимает любые популярные форматы прокси-шопов
  и раскладывает на хост/порт/логин/пароль. Когда формат неоднозначен
  (например host:port:user:pass vs user:pass:host:port) — возвращает
  confidence:"low" + альтернативу, и UI эскалирует подтверждение.

  Поддерживаемые формы:
    http://user:pass@host:port      (стандарт)
    user:pass@host:port
    host:port@user:pass
    host:port:user:pass             (частый формат шопов)
    user:pass:host:port
    host:port
    "host:port user:pass"           (через пробел/таб)
*/

export interface ParsedProxy {
  host: string;
  port: number;
  username?: string;
  password?: string;
  /** Нормализованный URL http://[user:pass@]host:port */
  url: string;
  confidence: "high" | "low";
  /** Альтернативная трактовка (для «Поменять местами») при low. */
  alt?: ParsedProxy;
}

function isPort(s: string): boolean {
  if (!/^\d{1,5}$/.test(s)) return false;
  const n = Number(s);
  return n >= 1 && n <= 65535;
}

function isIPv4(s: string): boolean {
  const m = s.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  return !!m && m.slice(1).every((o) => Number(o) <= 255);
}

function isHostLike(s: string): boolean {
  return /^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$/i.test(s) && (s.includes(".") || s === "localhost");
}

function buildUrl(host: string, port: number, user?: string, pass?: string): string {
  const auth = user ? `${encodeURIComponent(user)}:${encodeURIComponent(pass ?? "")}@` : "";
  return `http://${auth}${host}:${port}`;
}

function make(
  host: string,
  port: number,
  user: string | undefined,
  pass: string | undefined,
  confidence: "high" | "low",
): ParsedProxy {
  return {
    host,
    port,
    username: user,
    password: pass,
    url: buildUrl(host, port, user, pass),
    confidence,
  };
}

/** Ошибка разбора с человеческим объяснением (например, SOCKS-прокси). */
export interface ParseReject {
  error: string;
}

export function isParseReject(r: ParsedProxy | ParseReject | null): r is ParseReject {
  return !!r && "error" in r;
}

export function parseProxyInput(raw: string): ParsedProxy | ParseReject | null {
  let s = raw.trim();
  if (!s) return null;

  // SOCKS молча превращать в http нельзя: мост говорит только HTTP CONNECT,
  // и такой прокси «не отвечал» бы без объяснения причины.
  if (/^socks\d?h?:\/\//i.test(s)) {
    return { error: "Это SOCKS-прокси — мост работает только с HTTP-прокси. Попросите у продавца HTTP-адрес." };
  }

  // Схему убираем (протокол всегда http для моста).
  s = s.replace(/^https?:\/\//i, "");
  s = s.replace(/\/+$/, "");

  // "host:port user:pass" (пробел/таб/;) → склеиваем в 4-частную форму.
  const spaceParts = s.split(/[\s;,]+/).filter(Boolean);
  if (spaceParts.length === 2 && spaceParts[0].includes(":") && spaceParts[1].includes(":")) {
    s = `${spaceParts[0]}:${spaceParts[1]}`;
  } else if (spaceParts.length > 1) {
    s = spaceParts.join(":");
  }

  // Формы с @: одна из сторон — host:port.
  if (s.includes("@")) {
    const at = s.lastIndexOf("@");
    const left = s.slice(0, at);
    const right = s.slice(at + 1);
    const lp = left.split(":");
    const rp = right.split(":");
    const leftIsHost = lp.length === 2 && isPort(lp[1]) && (isIPv4(lp[0]) || isHostLike(lp[0]));
    const rightIsHost = rp.length === 2 && isPort(rp[1]) && (isIPv4(rp[0]) || isHostLike(rp[0]));

    if (leftIsHost && rightIsHost) {
      // Обе стороны похожи на host:port («доменный» логин + цифровой пароль).
      // IP-адрес — сильный сигнал хоста; без него — эскалация в UI.
      const asStd = make(rp[0], Number(rp[1]), lp[0], lp.slice(1).join(":"), "low"); // user:pass@host:port
      const asRev = make(lp[0], Number(lp[1]), rp[0], rp.slice(1).join(":"), "low"); // host:port@user:pass
      if (isIPv4(rp[0]) && !isIPv4(lp[0])) return { ...asStd, confidence: "high" };
      if (isIPv4(lp[0]) && !isIPv4(rp[0])) return { ...asRev, confidence: "high" };
      return { ...asStd, alt: asRev };
    }
    if (rightIsHost) {
      // user:pass@host:port — стандарт.
      const [user, ...passParts] = lp;
      return make(rp[0], Number(rp[1]), user, passParts.join(":"), "high");
    }
    if (leftIsHost) {
      // host:port@user:pass
      const [user, ...passParts] = rp;
      return make(lp[0], Number(lp[1]), user, passParts.join(":"), "high");
    }
    return null;
  }

  const parts = s.split(":");

  if (parts.length === 2) {
    // host:port без auth.
    if (isPort(parts[1]) && (isIPv4(parts[0]) || isHostLike(parts[0]))) {
      return make(parts[0], Number(parts[1]), undefined, undefined, "high");
    }
    return null;
  }

  if (parts.length === 4) {
    const v1ok = isPort(parts[1]) && (isIPv4(parts[0]) || isHostLike(parts[0])); // host:port:user:pass
    const v2ok = isPort(parts[3]) && (isIPv4(parts[2]) || isHostLike(parts[2])); // user:pass:host:port

    if (v1ok && !v2ok) {
      return make(parts[0], Number(parts[1]), parts[2], parts[3], "high");
    }
    if (v2ok && !v1ok) {
      return make(parts[2], Number(parts[3]), parts[0], parts[1], "high");
    }
    if (v1ok && v2ok) {
      // Обе трактовки валидны. IP-адрес — сильный сигнал: логины-IP не бывают.
      const v1 = make(parts[0], Number(parts[1]), parts[2], parts[3], "low");
      const v2 = make(parts[2], Number(parts[3]), parts[0], parts[1], "low");
      if (isIPv4(parts[0]) && !isIPv4(parts[2])) {
        return { ...v1, confidence: "high" };
      }
      if (isIPv4(parts[2]) && !isIPv4(parts[0])) {
        return { ...v2, confidence: "high" };
      }
      // Неоднозначно: предлагаем host:port:user:pass (частее у шопов) + alt.
      return { ...v1, alt: v2 };
    }
    return null;
  }

  if (parts.length === 3) {
    // host:port:user (без пароля) — редкий, но встречается.
    if (isPort(parts[1]) && (isIPv4(parts[0]) || isHostLike(parts[0]))) {
      return make(parts[0], Number(parts[1]), parts[2], "", "low");
    }
    return null;
  }

  return null;
}
