//! Очистка текстов перед отправкой на сервер (отчёты о сбоях, отчёт
//! человека, диагностика) и подпись сбоя `sig`. Без личных данных: адреса,
//! логины и пароли прокси, почта и имя пользователя в путях заменяются
//! заглушками.

use regex::{Captures, Regex};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

struct Rules {
    proxy_line: Regex,
    url_auth: Regex,
    /// Прокси без схемы: login:pass@host:port, login:pass:host:port, host:port@login:pass.
    bare_at: Regex,
    bare_colon: Regex,
    bare_host_first: Regex,
    /// key=..., token=..., password=..., pass=, pwd=, secret=
    kv_secret: Regex,
    /// "password":"x", "pass": "x" в JSON.
    json_secret: Regex,
    bearer: Regex,
    /// Ключ магазина: 43 знака base64url подряд (доступ к кабинету покупателя).
    shop_key: Regex,
    /// password: x, token: x, api-key: x, x-api-key: x.
    kv_colon: Regex,
    /// Cookie: ..., Set-Cookie: ...
    cookie: Regex,
    /// Authorization / Proxy-Authorization: Basic ...
    auth_header: Regex,
    /// Домен с настоящим доменом верхнего уровня.
    domain: Regex,
    /// rDNS: 185-12-34-56.provider.ru, ip-10-0-0-1.ec2.internal.
    rdns: Regex,
    num: Regex,
    hexid: Regex,
    quoted: Regex,
    asset_hash: Regex,
    win_user: Regex,
    win_user_end: Regex,
    unix_user: Regex,
    unix_user_end: Regex,
    email: Regex,
    ipv4: Regex,
    ipv6: Regex,
    url_tail: Regex,
    col: Regex,
}

/// Настоящие домены верхнего уровня. Расширения файлов (rs, js, ts, json,
/// exe, dll, ps1, md, log, txt, toml, py, so...) сюда не входят: иначе
/// main.rs и index-AB12.js стали бы «доменами».
///
/// Три группы по тому, как легко зона путается с кодом и файлами:
/// - STRONG: чистим всегда;
/// - EXT: совпадают с расширениями файлов (`x.sh`, `Clod Helper.app`):
///   чистим при трёх и более метках или в контексте URL и хоста;
/// - PROP: совпадают с обращением к полю (`data.id`, `window.top`,
///   `console.info`, `user.name`): чистим только в контексте URL и хоста.
const TLD_STRONG: &str = concat!(
    "com|net|org|edu|gov|biz|xyz|online|site|club|vip|cloud|tech|store|shop|link|live|space|page|fun|work|world|today|news|life|",
    "ru|su|ua|kz|uz|az|lv|ee|cz|sk|hu|ro|bg|hr|si|gr|de|fr|es|pt|nl|ch|se|fi|dk|uk|ie|us|mx|br|cn|jp|kr|tw|hk|sg|tr|il|ir|ae|sa|za|au|nz|eu|",
    "internal|local|lan|localdomain|corp|intranet|рф"
);
const TLD_EXT: &str = "sh|app|pl|cc";
const TLD_PROP: &str = concat!(
    "info|name|pro|dev|io|ai|me|tv|fm|ly|gg|top|ge|am|lt|by|it|be|no|at|ca|ar|cl|co|in|id|th|vn|eg|is|to|home"
);

/// Свои и сервисные домены, которые оставляем как есть.
fn domain_allowed(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_lowercase();
    ["clodhelper.com", "github.com", "githubusercontent.com"]
        .iter()
        .any(|d| h == *d || h.ends_with(&format!(".{d}")))
        || crate::diagnostics::is_service_domain(&h)
}

fn in_list(list: &str, tld: &str) -> bool {
    list.split('|').any(|t| t == tld)
}

/// Оставить найденное как есть: свой или сервисный домен, вызов функции
/// (`ident.tld(`), кадр стека, обращение к полю без контекста URL и хоста.
fn keep_domain(s: &str, start: usize, end: usize) -> bool {
    let host = &s[start..end];
    if domain_allowed(host) {
        return true;
    }
    let (before, after) = (&s[..start], &s[end..]);
    if after.starts_with('(') {
        return true;
    }
    // Кадр стека: «at name.tld (...)».
    let line = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
    static FRAME: OnceLock<Regex> = OnceLock::new();
    if FRAME.get_or_init(|| Regex::new(r"^\s*at\s+(?:async\s+|new\s+)?$").expect("regex")).is_match(line) {
        return true;
    }
    let tld = host.rsplit('.').next().unwrap_or("").to_lowercase();
    let labels = host.split('.').count();
    let url_ctx = before.ends_with("://")
        || before.ends_with('@')
        || after.starts_with('/')
        || (after.starts_with(':') && after[1..].starts_with(|c: char| c.is_ascii_digit()));
    if in_list(TLD_PROP, &tld) {
        return !url_ctx;
    }
    if in_list(TLD_EXT, &tld) {
        return !(labels >= 3 || url_ctx);
    }
    false
}

/// Заменить домены вне белого списка на `<host>`.
fn scrub_domains(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for m in rules().domain.find_iter(s) {
        if keep_domain(s, m.start(), m.end()) {
            continue;
        }
        out.push_str(&s[last..m.start()]);
        out.push_str("<host>");
        last = m.end();
    }
    out.push_str(&s[last..]);
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Заменить известное значение. Логин, имя и хост - только по границам слова
/// (с учётом кириллицы), пароль - как подстроку. Уже вставленные `<...>` не трогаем.
fn replace_secret(s: &str, v: &str, tag: &str) -> String {
    let Ok(re) = Regex::new(&format!("(?i){}", regex::escape(v))) else {
        return s.replace(v, tag);
    };
    let word = tag != "<pass>";
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for m in re.find_iter(s) {
        let (before, after) = (&s[..m.start()], &s[m.end()..]);
        if before.ends_with('<') && after.starts_with('>') {
            continue;
        }
        if word
            && (before.chars().next_back().is_some_and(is_word_char) || after.chars().next().is_some_and(is_word_char))
        {
            continue;
        }
        out.push_str(&s[last..m.start()]);
        out.push_str(tag);
        last = m.end();
    }
    out.push_str(&s[last..]);
    out
}

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| {
        let re = |p: &str| Regex::new(p).expect("regex");
        Rules {
            // host:port:login:pass (хост с точкой: домен или IPv4).
            proxy_line: re(r"\b[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+:\d{2,5}:[^\s:]+:[^\s:]+"),
            // Пароль может содержать «/» и «@»: берём до ПОСЛЕДНЕГО «@» в слове.
            url_auth: re(r"([A-Za-z][A-Za-z0-9+.-]*)://[^\s/@:]+(?::\S*)?@"),
            bare_at: re(concat!(
                r#"(^|[\s"'(<\[,;=])[^\s@:"'/=]+:(?:[^\s/]|/[^\s/])*@"#,
                r"(?:[A-Za-z0-9-]+\.)*[A-Za-z0-9-]+:\d{2,5}\b",
            )),
            bare_colon: re(concat!(
                r#"(^|[\s"'(<\[,;=])[^\s@:"'/=]+:[^\s:@"'/]+:"#,
                r"(?:[A-Za-z0-9-]+\.)+[A-Za-z0-9-]+:\d{2,5}\b",
            )),
            bare_host_first: re(concat!(
                r#"(^|[\s"'(<\[,;=])(?:[A-Za-z0-9-]+\.)+[A-Za-z0-9-]+:\d{2,5}@"#,
                r"[^\s@:]+:\S+",
            )),
            kv_secret: re(concat!(
                r#"(?i)\b((?:[A-Za-z0-9_-]*(?:key|token|password|passwd|secret)|(?:[A-Za-z0-9_-]*[_-])?(?:pass|pwd)))"#,
                r#"=[^\s&"',;]+"#,
            )),
            shop_key: re(r"(^|[^A-Za-z0-9_-])[A-Za-z0-9_-]{43}($|[^A-Za-z0-9_=-])"),
            json_secret: re(concat!(
                r#"(?i)"((?:[A-Za-z0-9_-]*(?:key|token|password|passwd|secret)|(?:[A-Za-z0-9_-]*[_-])?(?:pass|pwd)))""#,
                r#"\s*:\s*(?:"(?:[^"\\]|\\.)*"|-?[0-9][0-9.]*|true|false|null)"#,
            )),
            bearer: re(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"),
            auth_header: re(
                r#"(?i)\b((?:proxy-)?authorization["']?\s*[:=]\s*["']?)(basic|digest|negotiate|ntlm)\s+[^\s"',;]+"#,
            ),
            domain: re(&format!(
                r"(?i)\b[\p{{L}}\p{{N}}](?:[\p{{L}}\p{{N}}-]*[\p{{L}}\p{{N}}])?(?:\.[\p{{L}}\p{{N}}](?:[\p{{L}}\p{{N}}-]*[\p{{L}}\p{{N}}])?)*\.(?:{TLD_STRONG}|{TLD_EXT}|{TLD_PROP})\b"
            )),
            kv_colon: re(concat!(
                r#"(?i)\b((?:[A-Za-z0-9_-]*(?:token|secret|password|passwd)|(?:x-)?api[-_]?key|key|pass|pwd))"#,
                r#"\s*:\s*([^\s"',;]+)"#,
            )),
            cookie: re(r"(?i)\b((?:set-)?cookie\s*:\s*)[^\r\n]+"),
            rdns: re(r"(?i)\b(?:[a-z0-9]+-)*\d{1,3}-\d{1,3}-\d{1,3}-\d{1,3}(?:\.[a-z0-9-]+)+\b"),
            num: re(r"\d+"),
            hexid: re(r"(?i)\b(?:0x)?[0-9a-f]{8,}\b"),
            quoted: re(r#"'[^']*'|"[^"]*"|`[^`]*`"#),
            asset_hash: re(r"[-.][A-Za-z0-9_]{6,12}(\.(?:js|mjs|css|map))\b"),
            // Имя пользователя целиком до следующего «\» или «/», пробелы внутри
            // можно. Без разделителя после имени - старое правило (до пробела).
            win_user: re(r#"(?i)([A-Z]:[\\/]+Users[\\/]+)[^\\/\s"'<>|]+"#),
            win_user_end: re(r#"(?i)([A-Z]:[\\/]+Users[\\/]+)[^\\/"'<>|:*?\r\n]{1,64}([\\/])"#),
            unix_user: re(r#"(?i)(/(?:Users|home)/)[^/\s"'<>|\\]+"#),
            unix_user_end: re(r#"(?i)(/(?:Users|home)/)[^/"'<>|\\:*?\r\n]{1,64}(/)"#),
            email: re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}"),
            ipv4: re(r"\b(?:\d{1,3}\.){3}\d{1,3}\b"),
            ipv6: re(concat!(
                r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b",
                r"|\b(?:[0-9a-f]{1,4}:){1,7}:(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,6})?",
                r"|(?:^|[\s\[(])::(?:[0-9a-f]{1,4}:){0,6}[0-9a-f]{1,4}\b",
            )),
            url_tail: re(r"[?#]\S*"),
            col: re(r"(:\d+):\d+\b"),
        }
    })
}

/// Имя пользователя в путях Windows и Unix заменить на `<user>`.
fn scrub_user_paths(text: &str) -> String {
    let r = rules();
    let mut s = r.win_user_end.replace_all(text, "${1}<user>${2}").into_owned();
    s = r.win_user.replace_all(&s, "${1}<user>").into_owned();
    s = r.unix_user_end.replace_all(&s, "${1}<user>${2}").into_owned();
    r.unix_user.replace_all(&s, "${1}<user>").into_owned()
}

/// Слова, которые нельзя считать личными, даже если они лежат в переменных
/// окружения (иначе обычный текст превратится в заглушки).
const COMMON_WORDS: &[&str] = &[
    "user", "users", "admin", "administrator", "root", "guest", "default", "public", "home", "localhost", "onedrive",
    "owner", "computer", "desktop", "laptop", "windows", "system", "local", "test", "пользователь", "администратор", "mac", "macbook", "pc", "dev",
];

/// Значения из окружения, которые выдают человека: имя пользователя,
/// компьютера, домена, папка профиля и OneDrive. `get` читает переменную.
pub fn secrets_from_env(get: impl Fn(&str) -> Option<String>) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = Vec::new();
    let mut add = |v: &str, tag: &'static str| {
        let v = v.trim();
        let low = v.to_lowercase();
        if v.chars().count() >= 3 && !COMMON_WORDS.contains(&low.as_str()) && !out.iter().any(|(x, _)| x.to_lowercase() == low) {
            out.push((v.to_string(), tag));
        }
    };
    let last_part = |p: &str| p.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or("").to_string();
    for k in ["USERNAME", "USER"] {
        if let Some(v) = get(k) {
            add(&v, "<user>");
        }
    }
    for k in ["COMPUTERNAME", "USERDOMAIN", "HOSTNAME"] {
        if let Some(v) = get(k) {
            add(&v, "<host>");
            // MacBook-Ivana.local: короткое имя тоже выдаёт человека.
            if let Some(short) = v.trim().split('.').next().filter(|x| *x != v.trim()) {
                add(short, "<host>");
            }
        }
    }
    for k in ["USERPROFILE", "HOME"] {
        if let Some(v) = get(k) {
            add(&last_part(&v), "<user>");
        }
    }
    for k in ["OneDrive", "OneDriveCommercial"] {
        if let Some(v) = get(k) {
            add(&last_part(&v), "<dir>");
        }
    }
    out.sort_by_key(|(v, _)| std::cmp::Reverse(v.len()));
    out
}

/// Логины, пароли и хосты из прокси (пул и резервы) для точной замены.
pub fn secrets_from_urls(urls: &[String]) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = Vec::new();
    let mut add = |v: String, tag: &'static str| {
        if v.chars().count() >= 3 && !out.iter().any(|(x, _)| *x == v) {
            out.push((v, tag));
        }
    };
    let dec = |s: &str| {
        url::form_urlencoded::parse(format!("x={s}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    };
    for u in urls {
        let Ok(p) = url::Url::parse(u) else { continue };
        if !p.username().is_empty() {
            add(dec(p.username()), "<login>");
            add(p.username().to_string(), "<login>");
        }
        if let Some(pw) = p.password() {
            add(dec(pw), "<pass>");
            add(pw.to_string(), "<pass>");
        }
        if let Some(h) = p.host_str() {
            add(h.to_string(), "<host>");
        }
    }
    // Длинные первыми: короткое не должно разрезать длинное.
    out.sort_by_key(|(v, _)| std::cmp::Reverse(v.len()));
    out
}

/// Имя компьютера из системы (gethostname). На Windows есть COMPUTERNAME.
#[cfg(unix)]
fn system_hostname() -> Option<String> {
    extern "C" {
        fn gethostname(name: *mut std::os::raw::c_char, len: usize) -> std::os::raw::c_int;
    }
    let mut buf = [0u8; 256];
    // SAFETY: буфер на стеке, длина передана с запасом под нуль.
    let rc = unsafe { gethostname(buf.as_mut_ptr() as *mut std::os::raw::c_char, buf.len() - 1) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let s = String::from_utf8_lossy(&buf[..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(not(unix))]
fn system_hostname() -> Option<String> {
    None
}

/// Известные секреты: пул прокси с резервами и данные из окружения.
fn known_secrets() -> Vec<(String, &'static str)> {
    let mut v = secrets_from_urls(&crate::settings::bridge_upstreams(&crate::settings::load()));
    // На macOS из программы (не из терминала) HOSTNAME не задан: спрашиваем систему.
    let env = |k: &str| std::env::var(k).ok().or_else(|| if k == "HOSTNAME" { system_hostname() } else { None });
    for e in secrets_from_env(env) {
        if !v.iter().any(|(x, _)| x.eq_ignore_ascii_case(&e.0)) {
            v.push(e);
        }
    }
    v.sort_by_key(|(x, _)| std::cmp::Reverse(x.len()));
    v
}

/// Очистить текст перед отправкой.
pub fn scrub(text: &str) -> String {
    scrub_with(text, &known_secrets())
}

/// Спрятать ключи магазина (43 знака base64url) - и для журналов на диске.
pub fn mask_shop_keys(text: &str) -> String {
    // Границы входят в совпадение, поэтому два ключа через один разделитель
    // находятся за два прохода.
    let mut s = text.to_string();
    for _ in 0..2 {
        s = rules().shop_key.replace_all(&s, "${1}<key>${2}").into_owned();
    }
    s
}

pub fn scrub_with(text: &str, secrets: &[(String, &'static str)]) -> String {
    let r = rules();
    let text = &mask_shop_keys(text);
    let mut s = r.proxy_line.replace_all(text, "<proxy>").into_owned();
    s = r.url_auth.replace_all(&s, "${1}://<auth>@").into_owned();
    s = scrub_user_paths(&s);
    for re in [&r.bare_at, &r.bare_colon, &r.bare_host_first] {
        s = re.replace_all(&s, "${1}<proxy>").into_owned();
    }
    s = r.json_secret.replace_all(&s, "\"${1}\":\"<secret>\"").into_owned();
    // «Authorization: Basic ...» раньше «Authorization: x»: схема остаётся видна.
    s = r.kv_secret.replace_all(&s, "${1}=<secret>").into_owned();
    s = r.auth_header.replace_all(&s, "${1}${2} <token>").into_owned();
    s = r.bearer.replace_all(&s, "Bearer <token>").into_owned();
    s = r.cookie.replace_all(&s, "${1}<secret>").into_owned();
    s = r
        .kv_colon
        .replace_all(&s, |c: &Captures| {
            // Уже заменённое (<secret>, <token>) не трогаем.
            if c[2].starts_with('<') {
                c[0].to_string()
            } else {
                format!("{}: <secret>", &c[1])
            }
        })
        .into_owned();
    for (v, tag) in secrets {
        // Без учёта регистра: Windows путает «Ivan» и «IVAN».
        // Короткое имя (до 4 знаков) вне путей слишком часто совпадает с обычным
        // словом (Mac, Mach-O): пути с ним уже очищены правилами путей выше.
        if matches!(*tag, "<user>" | "<host>" | "<dir>") && v.chars().count() <= 4 {
            continue;
        }
        s = replace_secret(&s, v, tag);
    }
    s = r.email.replace_all(&s, "<email>").into_owned();
    s = r
        .ipv4
        .replace_all(&s, |c: &Captures| {
            let m = &c[0];
            if m.split('.').all(|o| o.parse::<u16>().is_ok_and(|n| n <= 255)) {
                "<ip>".to_string()
            } else {
                m.to_string()
            }
        })
        .into_owned();
    s = r
        .ipv6
        .replace_all(&s, |c: &Captures| {
            // Пробел или скобка перед «::» остаются на месте.
            let m = &c[0];
            let lead: String = m.chars().take_while(|ch| ch.is_whitespace() || *ch == '[' || *ch == '(').collect();
            format!("{lead}<ip>")
        })
        .into_owned();
    // rDNS целиком, потом остальные домены вне белого списка.
    s = r.rdns.replace_all(&s, "<host>").into_owned();
    scrub_domains(&s)
}

/// Обрезать до max БАЙТ UTF-8 по границе символа, без добавок в конец:
/// лимиты сервера считаются в байтах.
pub fn clip_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Очистить и уложить в лимит. Сначала грубо режем исходник (чтобы не гонять
/// регулярки по мегабайтам), потом чистим, и только потом точно обрезаем:
/// замены могут удлинить текст.
pub fn scrub_clip(text: &str, max: usize) -> String {
    clip_bytes(&scrub(&pre_clip(text, max.saturating_mul(4))), max)
}

/// Грубая обрезка исходника. Если режем, отступаем к последнему пробелу или
/// переводу строки: иначе адрес прокси с паролем мог бы остаться обрубком,
/// который правила уже не узнают.
fn pre_clip(text: &str, max: usize) -> String {
    let cut = clip_bytes(text, max);
    if cut.len() == text.len() {
        return cut;
    }
    let tail_from = cut.len().saturating_sub(512);
    match cut.char_indices().rev().take_while(|(i, _)| *i >= tail_from).find(|(_, c)| c.is_whitespace()) {
        Some((i, _)) => cut[..i].to_string(),
        None => cut,
    }
}

/// Короткий вид для подписи: rs, ui, br, up (сервер: 4-128 символов из
/// букв, цифр и _.:-).
fn sig_prefix(kind: &str) -> &'static str {
    match kind {
        "rust" => "rs",
        "ui" => "ui",
        "bridge" => "br",
        "update" => "up",
        _ => "xx",
    }
}

/// Подпись сбоя: вид-хэш, где хэш - первые 16 hex sha256 от kind и
/// нормализованного места (например rs-4f1a...).
/// Нормализация: без query/hash в URL, без номеров колонок, без имени
/// пользователя в путях, обратные слэши как прямые.
pub fn sig(kind: &str, place: &str) -> String {
    let r = rules();
    let mut p = place.trim().replace('\\', "/");
    p = r.url_tail.replace_all(&p, "").into_owned();
    p = r.col.replace_all(&p, "${1}").into_owned();
    p = scrub_user_paths(&p);
    sig_from(kind, &[&p])
}

/// Сообщение без всего, что меняется от случая к случаю: числа, хеши в именах
/// файлов, длинные hex-идентификаторы, всё в кавычках. Первая строка, до 160 знаков.
pub fn normalize_msg(msg: &str) -> String {
    let r = rules();
    let first = msg.trim().lines().next().unwrap_or("");
    let mut s = scrub_user_paths(&first.replace('\\', "/"));
    s = r.quoted.replace_all(&s, "''").into_owned();
    s = r.asset_hash.replace_all(&s, "${1}").into_owned();
    s = r.hexid.replace_all(&s, "#").into_owned();
    s = r.num.replace_all(&s, "#").into_owned();
    s.chars().take(160).collect()
}

fn sig_from(kind: &str, parts: &[&str]) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    for p in parts {
        h.update(b"\n");
        h.update(p.as_bytes());
    }
    let hex: String = h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{}-{hex}", sig_prefix(kind))
}

/// Подпись паники: файл без номера строки и сообщение без чисел.
pub fn sig_rust(file: &str, msg: &str) -> String {
    sig_from("rust", &[&rust_loc(file), &normalize_msg(msg)])
}

/// Файл паники без строки и без имени пользователя, до 128 знаков.
pub fn rust_loc(file: &str) -> String {
    let mut p = scrub_user_paths(&file.trim().replace('\\', "/"));
    p = rules().col.replace_all(&p, "${1}").into_owned();
    // «файл:строка»: строку отбрасываем.
    if let Some(i) = p.rfind(':').filter(|&i| i > 1 && p[i + 1..].chars().all(|c| c.is_ascii_digit())) {
        p.truncate(i);
    }
    p.chars().take(128).collect()
}

/// Имя функции первого кадра стека JS ("at f (url:1:2)" или "f@url:1:2").
/// Пусто, если кадр без имени. Файл и строка не нужны: бандл минифицирован.
pub fn ui_function(stack: &str) -> String {
    static AT: OnceLock<Regex> = OnceLock::new();
    let at = AT.get_or_init(|| Regex::new(r"^at\s+(?:async\s+)?(?:new\s+)?([^\s(]+)\s+\(").expect("regex"));
    for line in stack.lines().map(str::trim) {
        let name = if let Some(c) = at.captures(line) {
            c[1].to_string()
        } else if line.starts_with("at ") {
            String::new() // «at http://...:1:2» - кадр без имени
        } else if let Some((n, _)) = line.split_once('@').filter(|(_, r)| r.contains(':')) {
            n.to_string()
        } else {
            continue;
        };
        let name = name.trim_matches(|c: char| c == '<' || c == '>' || c == '"');
        let ok = !name.is_empty() && name != "anonymous" && !name.contains("://") && !name.contains('/');
        return if ok { name.chars().take(128).collect() } else { String::new() };
    }
    String::new()
}

/// Подпись сбоя интерфейса: функция первого кадра и сообщение без чисел.
pub fn sig_ui(msg: &str, stack: &str) -> String {
    sig_from("ui", &[&ui_function(stack), &normalize_msg(msg)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(s: &str) -> String {
        scrub_with(s, &[])
    }

    #[test]
    fn scrubs_ips() {
        assert_eq!(clean("connect 185.12.34.56:8080 failed"), "connect <ip>:8080 failed");
        assert_eq!(clean("v 10.0.26100.1 и 999.1.1.1"), "v 10.0.26100.1 и 999.1.1.1", "не IP остаётся");
        assert!(!clean("addr 2001:db8:85a3:0:0:8a2e:370:7334 ok").contains("2001"));
        assert!(!clean("addr 2001:db8::1 ok").contains("db8"));
        assert_eq!(clean("loopback ::1 x"), "loopback <ip> x");
        assert_eq!(clean("время 12:30:45 дальше"), "время 12:30:45 дальше");
    }

    #[test]
    fn scrubs_urls_with_login() {
        assert_eq!(
            clean("proxy http://user:p%40ss@host.example.com:8080/x"),
            "proxy http://<auth>@<host>:8080/x"
        );
        assert_eq!(clean("socks5://u:p@1.2.3.4:1080"), "socks5://<auth>@<ip>:1080");
    }

    #[test]
    fn scrubs_proxy_lines() {
        assert_eq!(clean("1.2.3.4:8080:login:secret end"), "<proxy> end");
        assert_eq!(clean("proxy.example.com:3128:me:pw"), "<proxy>");
    }

    #[test]
    fn scrubs_email_and_user_paths() {
        assert_eq!(clean("пишет a.b+c@mail.example.org"), "пишет <email>");
        assert_eq!(clean(r"C:\Users\Иван\AppData\x"), r"C:\Users\<user>\AppData\x");
        assert_eq!(clean("c:/users/john/x.rs"), "c:/users/<user>/x.rs");
        assert_eq!(clean(r"C:\\Users\\john\\x"), r"C:\\Users\\<user>\\x");
        assert_eq!(clean("/Users/anna/Library/x"), "/Users/<user>/Library/x");
        assert_eq!(clean("/home/bob/.config"), "/home/<user>/.config");
    }

    #[test]
    fn user_name_with_space_is_replaced_whole() {
        assert_eq!(clean(r"C:\Users\Иван Петров\AppData\x"), r"C:\Users\<user>\AppData\x");
        assert_eq!(clean("c:/users/John Smith Jr/x.rs"), "c:/users/<user>/x.rs");
        assert_eq!(clean("/Users/Anna Maria/Library/x"), "/Users/<user>/Library/x");
        assert_eq!(clean("/home/mary ann/.config"), "/home/<user>/.config");
        // Без разделителя после имени - до пробела, текст дальше не калечим.
        assert_eq!(clean(r"нет доступа к C:\Users\anna повтор"), r"нет доступа к C:\Users\<user> повтор");
        assert_eq!(clean(r"open C:\Users\anna failed"), r"open C:\Users\<user> failed");
        assert_eq!(
            sig("rust", r"C:\Users\Иван Петров\.cargo\x.rs:5"),
            sig("rust", "C:/Users/boris/.cargo/x.rs:5")
        );
    }

    #[test]
    fn env_values_are_secrets() {
        let env = |k: &str| match k {
            "USERNAME" => Some("ivanp".to_string()),
            "USER" => Some("admin".to_string()),
            "COMPUTERNAME" => Some("DESKTOP-7K2QX".to_string()),
            "USERDOMAIN" => Some("ab".to_string()),
            "HOSTNAME" => Some("macbook-ivan".to_string()),
            "USERPROFILE" => Some(r"C:\Users\Иван Петров".to_string()),
            "HOME" => Some("/Users/olga/".to_string()),
            "OneDrive" => Some(r"C:\Users\Иван Петров\OneDrive - Рога и Копыта".to_string()),
            "OneDriveCommercial" => Some(r"C:\Users\x\OneDrive".to_string()),
            _ => None,
        };
        let s = secrets_from_env(env);
        let has = |v: &str, t: &str| s.iter().any(|(x, tag)| x == v && *tag == t);
        assert!(has("ivanp", "<user>") && has("DESKTOP-7K2QX", "<host>") && has("macbook-ivan", "<host>"));
        assert!(has("Иван Петров", "<user>") && has("olga", "<user>") && has("OneDrive - Рога и Копыта", "<dir>"));
        assert!(s.iter().all(|(v, _)| v != "admin" && v != "ab" && v != "OneDrive"), "общие слова и короткие не берём");
        let out = scrub_with("host desktop-7k2qx, user IVANP, dir OneDrive - Рога и Копыта, admin ok", &s);
        assert_eq!(out, "host <host>, user <user>, dir <dir>, admin ok");
    }

    #[test]
    fn scrubs_proxies_without_scheme() {
        assert_eq!(clean("proxy login:pass@1.2.3.4:8000 end"), "proxy <proxy> end");
        assert_eq!(clean("login:pass@host.com:8000"), "<proxy>");
        assert_eq!(clean("login:p/a@ss@host.com:8000 x"), "<proxy> x");
        assert_eq!(clean("login:pass:1.2.3.4:8000"), "<proxy>");
        assert_eq!(clean("login:pass:host.com:8000,"), "<proxy>,");
        assert_eq!(clean("1.2.3.4:8000@login:pass"), "<proxy>");
        assert_eq!(clean("set a=login:pass@1.2.3.4:8000;"), "set a=<proxy>;");
        assert_eq!(clean(r#"{"p":"login:pass@1.2.3.4:8000"}"#), r#"{"p":"<proxy>"}"#);
        // Обычный текст не трогаем.
        assert_eq!(clean("Contact: me@mail.ru ok"), "Contact: <email> ok");
        assert_eq!(clean("время 12:30:45 дальше"), "время 12:30:45 дальше");
    }

    #[test]
    fn url_password_may_contain_slash_and_at() {
        assert_eq!(clean("http://user:p/a@ss@host.com:8080/x"), "http://<auth>@<host>:8080/x");
        assert_eq!(clean("socks5://u:@@/x@1.2.3.4:1080 end"), "socks5://<auth>@<ip>:1080 end");
    }

    #[test]
    fn scrubs_shop_keys() {
        let k = "q83vEjRWeJCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJA";
        for t in [
            format!("clodhelper://add?key={k}"),
            format!("GET /v1/claim/{k} 404"),
            format!("https://clodhelper.com/buy/account/#key={k}"),
            format!("{k} {k}"),
            k.to_string(),
        ] {
            let out = scrub_with(&t, &[]);
            assert!(!out.contains(k), "{out}");
            assert!(!mask_shop_keys(&t).contains(k), "{t}");
        }
        // Короче и длиннее 43 знаков - не ключ.
        assert_eq!(mask_shop_keys("abc"), "abc");
        let long = "a".repeat(44);
        assert_eq!(mask_shop_keys(&long), long);
    }

    #[test]
    fn scrubs_known_proxy_secrets() {
        let secrets = secrets_from_urls(&["http://mylogin:S3cretPw@gw.proxy.test:9000".to_string()]);
        let out = scrub_with("login mylogin, pass S3cretPw, host gw.proxy.test", &secrets);
        assert_eq!(out, "login <login>, pass <pass>, host <host>");
        // Короткие значения не трогаем, чтобы не калечить текст.
        assert!(secrets_from_urls(&["http://a:b@1.1.1.1:80".to_string()]).iter().all(|(v, _)| v != "a" && v != "b"));
    }

    #[test]
    fn sig_is_stable() {
        let a = sig("ui", "http://tauri.localhost/assets/index.js:10:55?v=1#x");
        assert_eq!(a.len(), 19);
        assert!(a.starts_with("ui-"));
        assert_eq!(a, sig("ui", "http://tauri.localhost/assets/index.js:10:99"), "колонка и query не влияют");
        assert_ne!(a, sig("ui", "http://tauri.localhost/assets/index.js:11:55"));
        assert_ne!(a, sig("rust", "http://tauri.localhost/assets/index.js:10:55"));
        assert_eq!(
            sig("rust", r"C:\Users\anna\.cargo\x.rs:5"),
            sig("rust", "C:/Users/boris/.cargo/x.rs:5"),
            "имя пользователя не влияет"
        );
        assert_eq!(sig("rust", "/home/a/x.rs:5"), sig("rust", "/home/b/x.rs:5"));
    }

    #[test]
    fn sig_fits_server_format() {
        let ok = |s: &str| {
            (4..=128).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c))
        };
        for (kind, pre) in [("rust", "rs-"), ("ui", "ui-"), ("bridge", "br-"), ("update", "up-")] {
            let s = sig(kind, r"C:\Users\Иван Петров\x.rs:5");
            assert!(ok(&s) && s.starts_with(pre), "{s}");
        }
        assert!(ok(&sig("что-то", "место")));
    }

    #[test]
    fn scrubs_key_token_password_and_bearer() {
        assert_eq!(clean("GET /x?token=abc123&y=1"), "GET /x?token=<secret>&y=1");
        assert_eq!(clean("password=hunter2 ok"), "password=<secret> ok");
        assert_eq!(clean("api_key=sk-12345, next"), "api_key=<secret>, next");
        assert_eq!(clean("Authorization: Bearer abc.def-ghi_1/2="), "Authorization: Bearer <token>");
        assert_eq!(clean("keyboard layout"), "keyboard layout");
    }

    #[test]
    fn clip_fits_bytes_after_scrub() {
        let long = "Ошибка соединения с 1.2.3.4 ".repeat(500);
        let out = scrub_clip(&long, 1024);
        assert!(out.len() <= 1024, "{}", out.len());
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
        // Замена IP на <ip> делает текст длиннее; лимит всё равно держится.
        let ips = "1.2.3.4 ".repeat(2000);
        assert!(scrub_clip(&ips, 1000).len() <= 1000);
        assert_eq!(clip_bytes("яя", 3), "я", "по границе символа");
    }

    #[test]
    fn domains_outside_whitelist_become_host() {
        assert_eq!(clean("connect to my-proxy.example.net failed"), "connect to <host> failed");
        assert_eq!(clean("GET https://shop.mycorp.ru/path?x=1"), "GET https://<host>/path?x=1");
        assert_eq!(clean("proxy.example.net:8080 refused"), "<host>:8080 refused");
        assert_eq!(clean("dns server.corp.local ok"), "dns <host> ok");
        // Белый список: сервисы, свой сайт и GitHub остаются.
        assert_eq!(clean("api.anthropic.com claude.ai"), "api.anthropic.com claude.ai");
        assert_eq!(clean("stats.clodhelper.com github.com raw.githubusercontent.com"), "stats.clodhelper.com github.com raw.githubusercontent.com");
        // Имена файлов и версии не домены.
        for f in [
            "main.rs", "src/hosts.rs", "index-AB12.js", "app.json", "setup.ps1", "x.sh", "Clod Helper.app", "clod-helper.exe",
            "kernel32.dll", "README.md", "ui.log", "notes.txt", "Cargo.toml", "main.ts", "v0.9.1", "10.0.26100.1", "tool.py",
        ] {
            assert_eq!(clean(&format!("open {f} now")), format!("open {f} now"), "{f}");
        }
    }

    #[test]
    fn rdns_names_are_replaced_whole() {
        assert_eq!(clean("from 185-12-34-56.provider.ru ok"), "from <host> ok");
        assert_eq!(clean("host ip-10-0-0-1.ec2.internal."), "host <host>.");
        assert_eq!(clean("ec2-3-4-5-6.compute-1.amazonaws.com x"), "<host> x");
    }

    #[test]
    fn secrets_in_json_and_headers() {
        assert_eq!(clean(r#"{"password":"x","a":1}"#), r#"{"password":"<secret>","a":1}"#);
        assert_eq!(clean(r#"{"pass": "x y"}"#), r#"{"pass":"<secret>"}"#);
        assert_eq!(clean(r#"{"my_pwd":"a\"b"}"#), r#"{"my_pwd":"<secret>"}"#);
        assert_eq!(clean("pass=abc end"), "pass=<secret> end");
        assert_eq!(clean("pwd=abc&x=1"), "pwd=<secret>&x=1");
        assert_eq!(clean("secret=abc"), "secret=<secret>");
        assert_eq!(clean("client_secret=abc"), "client_secret=<secret>");
        assert_eq!(clean("Proxy-Authorization: Basic dXNlcjpwYXNz"), "Proxy-Authorization: Basic <token>");
        assert_eq!(clean("Authorization: Basic dXNlcjpwYXNz=="), "Authorization: Basic <token>");
        assert_eq!(clean(r#""Authorization":"Basic dXNlcjpwYXNz""#), r#""Authorization":"Basic <token>""#);
        // Обычные слова не трогаем.
        assert_eq!(clean("bypass=1 compass=2 Basic settings"), "bypass=1 compass=2 Basic settings");
        assert_eq!(clean(r#"{"bypass":"x"}"#), r#"{"bypass":"x"}"#);
    }

    #[test]
    fn host_name_secrets_include_short_name_and_system_hostname() {
        let s = secrets_from_env(|k| (k == "HOSTNAME").then(|| "Ivans-MacBook.local".to_string()));
        assert_eq!(scrub_with("on Ivans-MacBook.local and ivans-macbook", &s), "on <host> and <host>");
        // gethostname на этой машине отвечает или его нет (Windows): паники нет.
        let _ = system_hostname();
    }

    #[cfg(unix)]
    #[test]
    fn system_hostname_is_not_empty_on_unix() {
        assert!(system_hostname().is_some_and(|h| !h.is_empty()));
    }

    #[test]
    fn clip_scrubs_before_cutting() {
        // Адрес с паролем на границе грубой обрезки не остаётся обрубком.
        let mut t = "a ".repeat(195);
        t.push_str("http://user:pa55word@proxy.example.net:8080 tail text");
        let out = scrub_clip(&t, 100);
        assert!(!out.contains("pa55") && !out.contains("user:") && !out.contains("proxy.example"), "{out}");
        assert!(out.len() <= 100);
        // Замены удлиняют текст, лимит держится.
        assert!(scrub_clip(&"x.example.net ".repeat(100), 120).len() <= 120);
    }

    #[test]
    fn rust_sig_ignores_line_and_numbers() {
        let a = sig_rust("src/hosts.rs", "index 5 out of range for slice of length 10");
        assert_eq!(a, sig_rust("src/hosts.rs", "index 77 out of range for slice of length 3"));
        assert_ne!(a, sig_rust("src/proxy.rs", "index 5 out of range for slice of length 10"));
        assert_ne!(a, sig_rust("src/hosts.rs", "other failure"));
        assert_eq!(sig_rust(r"C:\Users\anna\x.rs:5", "boom"), sig_rust("C:/Users/boris/x.rs:99", "boom"));
        assert_eq!(rust_loc("src/hosts.rs:12:5"), "src/hosts.rs");
        assert_eq!(rust_loc(r"C:\Users\anna\a.rs"), "C:/Users/<user>/a.rs");
        assert!(a.starts_with("rs-") && a.len() == 19);
    }

    #[test]
    fn ui_sig_by_function_and_normalized_message() {
        let st1 = "TypeError: x is undefined\n    at Xo (http://tauri.localhost/assets/index-AB12cd34.js:1:5521)\n    at y (http://tauri.localhost/assets/index-AB12cd34.js:1:100)";
        let st2 = "TypeError: x is undefined\n    at Xo (http://tauri.localhost/assets/index-ZZ99yy88.js:7:12)";
        assert_eq!(ui_function(st1), "Xo");
        let a = sig_ui("TypeError: Cannot read 'abc' of 123", st1);
        assert_eq!(a, sig_ui("TypeError: Cannot read 'zzz' of 4", st2), "числа, кавычки и хеш файла не влияют");
        assert_ne!(a, sig_ui("TypeError: Cannot read 'abc' of 123", "Error\n    at Other (a.js:1:1)"));
        assert_ne!(a, sig_ui("RangeError: bad", st1));
        assert!(a.starts_with("ui-") && a.len() == 19);
        // Колонка и строка не попадают в подпись.
        assert_eq!(sig_ui("m", "Error\n    at f (a.js:1:1)"), sig_ui("m", "Error\n    at f (b.js:9:9)"));
        // Кадр без имени и формат Safari.
        assert_eq!(ui_function("Error\n    at http://x/a.js:1:2"), "");
        assert_eq!(ui_function("f@http://x/a.js:1:2\ng@http://x/a.js:3:4"), "f");
        assert_eq!(ui_function(""), "");
        assert_eq!(normalize_msg("Failed to fetch 'https://a.b/c' at 0xdeadbeef12 code 500"), "Failed to fetch '' at # code #");
        assert_eq!(normalize_msg("load index-AB12cd34.js failed"), "load index.js failed");
    }

    #[test]
    fn colon_forms_cookies_and_numeric_json() {
        assert_eq!(clean("password: hunter2 ok"), "password: <secret> ok");
        assert_eq!(clean("pass: abc"), "pass: <secret>");
        assert_eq!(clean("pwd : abc"), "pwd: <secret>");
        assert_eq!(clean("secret: abc"), "secret: <secret>");
        assert_eq!(clean("token: abc"), "token: <secret>");
        assert_eq!(clean("key: abc"), "key: <secret>");
        assert_eq!(clean("api-key: abc"), "api-key: <secret>");
        assert_eq!(clean("X-Api-Key: abc123"), "X-Api-Key: <secret>");
        assert_eq!(clean("Cookie: sid=1; theme=dark\nnext"), "Cookie: <secret>\nnext");
        assert_eq!(clean("Set-Cookie: sid=1; Path=/"), "Set-Cookie: <secret>");
        assert_eq!(clean(r#"{"password":123,"x":1}"#), r#"{"password":"<secret>","x":1}"#);
        assert_eq!(clean(r#"{"pass": 12.5}"#), r#"{"pass":"<secret>"}"#);
        // Уже очищенное и обычные слова не трогаем.
        assert_eq!(clean("password=abc, token: Bearer xyz"), "password=<secret>, token: <secret> <token>");
        assert_eq!(clean("monkey: banana, keyboard: ru"), "monkey: banana, keyboard: ru");
    }

    #[test]
    fn more_zones_and_file_conflicts() {
        assert_eq!(clean("open сайт.рф now"), "open <host> now");
        assert_eq!(clean("a.example.link b.live c.space"), "<host> <host> <host>");
        // Конфликтные с расширениями файлов: нужно три метки или контекст URL.
        for keep in ["setup.sh", "Clod Helper.app", "run.pl", "a.cc", "install.sh now"] {
            assert_eq!(clean(&format!("see {keep}")), format!("see {keep}"), "{keep}");
        }
        assert_eq!(clean("host a.b.sh x"), "host <host> x");
        assert_eq!(clean("get https://tool.sh/install"), "get https://<host>/install");
        assert_eq!(clean("on app.example.app:8443"), "on <host>:8443");
    }

    #[test]
    fn code_like_names_are_not_domains() {
        for keep in [
            "console.info(x)", "user.name", "data.id", "arr.at(0)", "window.top", "this.ca", "obj.to", "a.lt",
            "at Object.top (file.js:1:2)", "  at console.info (http://tauri.localhost/a.js:1:2)",
        ] {
            assert_eq!(clean(keep), keep, "{keep}");
        }
        // Но с контекстом URL и порта это хост.
        assert_eq!(clean("https://example.io/x"), "https://<host>/x");
        assert_eq!(clean("server.example.info:8080"), "<host>:8080");
        // Функция с настоящим доменом вне стека - вызов, не хост.
        assert_eq!(clean("call my.example.com(1)"), "call my.example.com(1)");
    }

    #[test]
    fn known_secrets_match_whole_words_except_passwords() {
        let s = |v: &str, t: &'static str| vec![(v.to_string(), t)];
        assert_eq!(scrub_with("ivanov ivanovich divanov Ivanov.", &s("ivanov", "<user>")), "<user> ivanovich divanov <user>.");
        assert_eq!(scrub_with("Иванов Ивановы Ивановский", &s("Иванов", "<user>")), "<user> Ивановы Ивановский");
        assert_eq!(scrub_with("server1 myserver server", &s("server", "<host>")), "server1 myserver <host>");
        // Пароль - подстрока.
        assert_eq!(scrub_with("xS3cretx S3cret", &s("S3cret", "<pass>")), "x<pass>x <pass>");
        // Уже вставленные заглушки не ломаем.
        assert_eq!(scrub_with("<usernm> usernm", &s("usernm", "<user>")), "<usernm> <user>");
        assert_eq!(scrub_with("a <pass> b", &s("pass", "<pass>")), "a <pass> b");
    }

    #[test]
    fn short_user_name_is_replaced_only_inside_paths() {
        let env = |k: &str| (k == "USERNAME").then(|| "Mac".to_string());
        let sec = secrets_from_env(env);
        let text = "macOS 15, Mach-O, machine, Mac mini, /Users/Mac/Library";
        assert_eq!(scrub_with(text, &sec), "macOS 15, Mach-O, machine, Mac mini, /Users/<user>/Library");
        // Длинное имя: отдельное слово чистится, часть другого слова нет.
        let sec = vec![("Ivanov".to_string(), "<user>")];
        assert_eq!(scrub_with("Ivanov, ivanovich, IVANOV", &sec), "<user>, ivanovich, <user>");
    }

    #[test]
    fn russian_common_words_are_not_secrets() {
        let env = |k: &str| match k {
            "USERNAME" => Some("Пользователь".to_string()),
            "USER" => Some("Администратор".to_string()),
            _ => None,
        };
        assert!(secrets_from_env(env).is_empty());
    }
}
