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
    "owner", "computer", "desktop", "laptop", "windows", "system", "local", "test",
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

/// Известные секреты: пул прокси с резервами и данные из окружения.
fn known_secrets() -> Vec<(String, &'static str)> {
    let mut v = secrets_from_urls(&crate::settings::bridge_upstreams(&crate::settings::load()));
    for e in secrets_from_env(|k| std::env::var(k).ok()) {
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

pub fn scrub_with(text: &str, secrets: &[(String, &'static str)]) -> String {
    let r = rules();
    let mut s = r.proxy_line.replace_all(text, "<proxy>").into_owned();
    s = r.url_auth.replace_all(&s, "${1}://<auth>@").into_owned();
    s = scrub_user_paths(&s);
    for re in [&r.bare_at, &r.bare_colon, &r.bare_host_first] {
        s = re.replace_all(&s, "${1}<proxy>").into_owned();
    }
    for (v, tag) in secrets {
        // Без учёта регистра: Windows путает «Ivan» и «IVAN».
        match Regex::new(&format!("(?i){}", regex::escape(v))) {
            Ok(re) => s = re.replace_all(&s, regex::NoExpand(tag)).into_owned(),
            Err(_) => s = s.replace(v.as_str(), tag),
        }
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
    r.ipv6
        .replace_all(&s, |c: &Captures| {
            // Пробел или скобка перед «::» остаются на месте.
            let m = &c[0];
            let lead: String = m.chars().take_while(|ch| ch.is_whitespace() || *ch == '[' || *ch == '(').collect();
            format!("{lead}<ip>")
        })
        .into_owned()
}

/// Обрезать по границе символа.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

/// Подпись сбоя: kind + нормализованное место, первые 16 hex sha256.
/// Нормализация: без query/hash в URL, без номеров колонок, без имени
/// пользователя в путях, обратные слэши как прямые.
pub fn sig(kind: &str, place: &str) -> String {
    let r = rules();
    let mut p = place.trim().replace('\\', "/");
    p = r.url_tail.replace_all(&p, "").into_owned();
    p = r.col.replace_all(&p, "${1}").into_owned();
    p = scrub_user_paths(&p);
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update(b"\n");
    h.update(p.as_bytes());
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Место сбоя из стека JS: первая строка, где есть «:строка».
pub fn place_from_stack(msg: &str, stack: &str) -> String {
    let at = Regex::new(r":\d+(?::\d+)?\)?\s*$").expect("regex");
    stack
        .lines()
        .map(str::trim)
        .find(|l| at.is_match(l))
        .map(str::to_string)
        .unwrap_or_else(|| msg.lines().next().unwrap_or("").chars().take(120).collect())
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
            "proxy http://<auth>@host.example.com:8080/x"
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
        assert_eq!(clean("http://user:p/a@ss@host.com:8080/x"), "http://<auth>@host.com:8080/x");
        assert_eq!(clean("socks5://u:@@/x@1.2.3.4:1080 end"), "socks5://<auth>@<ip>:1080 end");
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
        assert_eq!(a.len(), 16);
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
    fn place_from_js_stack() {
        let stack = "Error: boom\n    at f (http://tauri.localhost/assets/a.js:1:2)\n    at g (b.js:3:4)";
        assert_eq!(place_from_stack("boom", stack), "at f (http://tauri.localhost/assets/a.js:1:2)");
        assert_eq!(place_from_stack("boom\nmore", ""), "boom");
    }
}
