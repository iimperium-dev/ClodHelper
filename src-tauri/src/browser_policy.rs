//! «Всегда через мост» для Chromium-браузеров: политика ProxySettings +
//! WebRtcIPHandling. Главнее параметров запуска и расширений — браузер идёт
//! через мост, даже открытый из Dock или меню «Пуск».
//!   - macOS: управляемые настройки (resources/browser-proxy-policy.sh),
//!     одна системная просьба пароля, как у Claude;
//!   - Windows: HKLM\SOFTWARE\Policies\<браузер>
//!     (resources/browser-proxy-policy.ps1), одно окно UAC.

use serde::{Deserialize, Serialize};

use crate::regval::RegValue;

#[cfg(not(target_os = "windows"))]
const SCRIPT: &str = include_str!("../resources/browser-proxy-policy.sh");
#[cfg(target_os = "windows")]
const SCRIPT: &str = include_str!("../resources/browser-proxy-policy.ps1");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserPolicyStatus {
    pub configured: bool,
    pub managed: bool,
    pub conflict: Option<String>,
    #[serde(default)]
    pub auto_restore: bool,
    #[serde(default)]
    pub server: String,
    pub notice: Option<String>,
}

#[derive(Deserialize)]
struct ScriptResult {
    ok: bool,
    status: Option<BrowserPolicyStatus>,
    error: Option<String>,
}

/// Браузер по пути к нему: домен управляемых настроек macOS (по имени
/// бандла) — он же ключ браузера на Windows (по имени exe). None — браузер
/// без политик. Разбор пути без учёта ОС: тесты гоняются и на macOS.
pub fn policy_domain(app_path: &str) -> Option<&'static str> {
    let leaf = app_path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next()?;
    let lower = leaf.to_ascii_lowercase();
    let stem = lower
        .strip_suffix(".app")
        .or_else(|| lower.strip_suffix(".exe"))
        .unwrap_or(&lower);
    Some(match stem {
        "google chrome" => "com.google.Chrome",
        // Windows: chrome.exe — и у Google Chrome, и у сборок Chromium.
        "chrome" if app_path.to_ascii_lowercase().contains("\\chromium\\") => "org.chromium.Chromium",
        "chrome" => "com.google.Chrome",
        "brave browser" | "brave" => "com.brave.Browser",
        "microsoft edge" | "msedge" => "com.microsoft.Edge",
        "chromium" => "org.chromium.Chromium",
        _ => return None,
    })
}

/// PAC моста для режима «Сайты только в выбранных браузерах».
pub const PAC_SERVER: &str = "pac:http://127.0.0.1:8887/proxy.pac";

/// Вход моста для политики — только локальный 127.0.0.1:порт или PAC моста.
fn valid_server(srv: &str) -> Result<(), String> {
    if srv == PAC_SERVER {
        return Ok(());
    }
    let port = srv.strip_prefix("127.0.0.1:").unwrap_or_default();
    if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) || port.parse::<u16>().map_or(true, |p| p == 0) {
        return Err("Ожидался вход моста 127.0.0.1:порт".into());
    }
    Ok(())
}

fn parse(payload: &str) -> Result<BrowserPolicyStatus, String> {
    let r: ScriptResult = serde_json::from_str(payload.trim())
        .map_err(|e| format!("Некорректный ответ политики браузера: {e}"))?;
    if !r.ok {
        return Err(r.error.unwrap_or_else(|| "Политика браузера: ошибка".into()));
    }
    r.status.ok_or_else(|| "Политика браузера: нет состояния".into())
}

#[cfg(not(target_os = "windows"))]
pub fn status(domain: &str) -> Result<BrowserPolicyStatus, String> {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-s", "--", "Status", domain])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Политика браузера: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(SCRIPT.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| format!("Политика браузера: {e}"))?;
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// Включить/снять с правами администратора (одно системное окно пароля).
/// Enable кладёт копию скрипта в root-папку — её запускает служба
/// автовосстановления; пользовательский процесс подменить её не может.
#[cfg(not(target_os = "windows"))]
pub fn set(domain: &str, server: Option<&str>) -> Result<BrowserPolicyStatus, String> {
    use base64::Engine;
    if policy_domain_known(domain).is_none() {
        return Err("Неизвестный браузер".into());
    }
    let enc = base64::engine::general_purpose::STANDARD.encode(SCRIPT);
    let shell = match server {
        Some(srv) => {
            valid_server(srv)?;
            format!(
                "/bin/mkdir -p '{dir}' && /usr/sbin/chown root:wheel '{dir}' && /bin/chmod 755 '{dir}' && \
                 /bin/echo {enc} | /usr/bin/base64 -D > '{dir}/browser-proxy-policy.sh'; \
                 /bin/echo {enc} | /usr/bin/base64 -D | /bin/sh -s -- Enable {domain} {srv}; exit 0",
                dir = "/Library/Application Support/ClodHelper",
            )
        }
        None => format!("/bin/echo {enc} | /usr/bin/base64 -D | /bin/sh -s -- Disable {domain}; exit 0"),
    };
    let apple = format!(
        "do shell script \"{shell}\" with prompt \"Clod Helper закрепляет прокси в браузере.\" with administrator privileges"
    );
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &apple])
        .output()
        .map_err(|e| format!("Не удалось запросить права администратора: {e}"))?;
    if String::from_utf8_lossy(&out.stderr).contains("-128") {
        return Err("Запрос пароля отменён".into());
    }
    let r = parse(&String::from_utf8_lossy(&out.stdout))?;
    // Не верим выводу root-процесса на слово — перечитываем фактическое состояние.
    let real = status(domain)?;
    if server.is_some() && !real.configured {
        return Err("Не удалось подтвердить политику браузера".into());
    }
    Ok(BrowserPolicyStatus { notice: r.notice, ..real })
}

fn policy_domain_known(domain: &str) -> Option<()> {
    matches!(
        domain,
        "com.google.Chrome" | "com.brave.Browser" | "com.microsoft.Edge" | "org.chromium.Chromium"
    )
    .then_some(())
}

/// Windows: ключ политик браузера в HKLM (как в browser-proxy-policy.ps1).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn policy_key(domain: &str) -> Option<&'static str> {
    Some(match domain {
        "com.google.Chrome" => r"SOFTWARE\Policies\Google\Chrome",
        "com.brave.Browser" => r"SOFTWARE\Policies\BraveSoftware\Brave",
        "com.microsoft.Edge" => r"SOFTWARE\Policies\Microsoft\Edge",
        "org.chromium.Chromium" => r"SOFTWARE\Policies\Chromium",
        _ => return None,
    })
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const OWNER_KEY: &str = r"SOFTWARE\ClodHelper\BrowserProxyPolicy";
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const WEBRTC: &str = "disable_non_proxied_udp";
/// Отдельные политики прокси прежних версий: ProxySettings их перекрывает,
/// поэтому любая из них — признак чужой настройки.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const LEGACY_PROXY: [&str; 5] = ["ProxyMode", "ProxyServerMode", "ProxyServer", "ProxyPacUrl", "ProxyBypassList"];
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const FOREIGN: &str = "У браузера уже есть чужая политика прокси — Clod Helper её не меняет.";

/// Значение ProxySettings, которое пишет Helper (байт в байт как скрипт).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn proxy_settings(server: &str) -> String {
    if let Some(url) = server.strip_prefix("pac:") {
        return format!(r#"{{"ProxyMode":"pac_script","ProxyPacUrl":"{url}"}}"#);
    }
    format!(r#"{{"ProxyMode":"fixed_servers","ProxyServer":"{server}","ProxyBypassList":"<local>"}}"#)
}

/// Исходное значение, которое Helper вправе заменить и потом вернуть:
/// ProxySettings — нет или пустая строка, WebRtcIPHandling — нет или уже наш.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn original_ok(v: &RegValue, proxy_settings: bool) -> bool {
    match v {
        RegValue::Absent => true,
        RegValue::Str(s) => s == if proxy_settings { "" } else { WEBRTC },
        RegValue::Other(_) => false,
    }
}

/// Состояние по значениям реестра — те же правила, что Get-BrowserPolicyStatus
/// в browser-proxy-policy.ps1 (запись по-прежнему делает скрипт с правами).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn status_from(proxy: &RegValue, webrtc: &RegValue, legacy: bool, owner: &RegValue) -> Result<BrowserPolicyStatus, String> {
    let damaged = || "Запись владения политикой браузера повреждена; политика не изменена.".to_string();
    let state = match owner {
        RegValue::Absent => None,
        RegValue::Other(_) => return Err(damaged()),
        RegValue::Str(json) => {
            let v: serde_json::Value = serde_json::from_str(json).map_err(|_| damaged())?;
            let server = v["server"].as_str().unwrap_or_default().to_string();
            let orig_proxy = RegValue::from_backup(&v["original"]["ProxySettings"]).ok_or_else(damaged)?;
            let orig_webrtc = RegValue::from_backup(&v["original"]["WebRtcIPHandling"]).ok_or_else(damaged)?;
            // Из повреждённой записи никогда не восстанавливаем произвольное.
            if v["owner"] != "ClodHelper"
                || v["version"].as_i64() != Some(1)
                || valid_server(&server).is_err()
                || !original_ok(&orig_proxy, true)
                || !original_ok(&orig_webrtc, false)
            {
                return Err(damaged());
            }
            Some((server, orig_proxy, orig_webrtc))
        }
    };
    // PAC («Сайты» в этом браузере) WebRTC не трогает — наше там значение
    // исходное; «весь трафик» — запрет UDP мимо прокси (как в скрипте).
    let ours = state.as_ref().is_some_and(|(server, _, ow)| {
        proxy.is_str(&proxy_settings(server)) && if server == PAC_SERVER { webrtc == ow } else { webrtc.is_str(WEBRTC) }
    });
    let conflict = if legacy {
        Some(FOREIGN.to_string())
    } else {
        match &state {
            None if !original_ok(proxy, true) || !original_ok(webrtc, false) => Some(FOREIGN.to_string()),
            Some((_, op, ow)) if !ours && !(proxy == op && webrtc == ow) => {
                Some("Политика прокси браузера изменена вне Clod Helper. Она сохранена без изменений.".to_string())
            }
            _ => None,
        }
    };
    Ok(BrowserPolicyStatus {
        configured: ours && !legacy,
        managed: state.is_some(),
        conflict,
        auto_restore: false,
        server: state.map(|(server, _, _)| server).unwrap_or_default(),
        notice: None,
    })
}

/// Windows: политика HKLM читается без прав администратора — прямо из
/// реестра (PowerShell стоил бы секунды на каждое открытие окна).
#[cfg(target_os = "windows")]
pub fn status(domain: &str) -> Result<BrowserPolicyStatus, String> {
    use crate::win::{hklm, value};
    let key = policy_key(domain).ok_or_else(|| "Неизвестный браузер".to_string())?;
    let root = hklm();
    let legacy = LEGACY_PROXY.iter().any(|name| value(&root, key, name).exists());
    status_from(
        &value(&root, key, "ProxySettings"),
        &value(&root, key, "WebRtcIPHandling"),
        legacy,
        &value(&root, OWNER_KEY, domain),
    )
}

/// Windows: «Сайты» (PAC моста) — включить одним браузерам и снять с других
/// за ОДНО окно UAC. Проверяет фактический результат по реестру.
#[cfg(target_os = "windows")]
pub fn apply_pac(enable: &[&str], disable: &[&str]) -> Result<(), String> {
    if enable.is_empty() && disable.is_empty() {
        return Ok(());
    }
    if enable.iter().chain(disable).any(|d| policy_domain_known(d).is_none()) {
        return Err("Неизвестный браузер".into());
    }
    let arguments = format!(
        "-Action Apply -EnableDomains \"{}\" -DisableDomains \"{}\" -Server {PAC_SERVER}",
        enable.join(","),
        disable.join(",")
    );
    let outcome = crate::elevate::run("browser-policy", &[("browser-proxy-policy.ps1", SCRIPT)], &arguments)?;
    if outcome.cancelled {
        return Err("Запрос прав администратора отменён".into());
    }
    let payload = outcome
        .result
        .ok_or_else(|| "Не удалось изменить политику браузера с правами администратора".to_string())?;
    // Ошибку скрипта показываем как есть, но верим только реестру.
    let script_err = parse(&payload).err();
    for d in enable {
        let st = status(d)?;
        if !(st.configured && st.server == PAC_SERVER) {
            return Err(script_err.unwrap_or_else(|| "Не удалось подтвердить политику браузера".into()));
        }
    }
    for d in disable {
        if status(d)?.managed {
            return Err(script_err.unwrap_or_else(|| "Не удалось снять политику браузера".into()));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn apply_pac(_: &[&str], _: &[&str]) -> Result<(), String> {
    Err("Пока только для Windows".into())
}

/// Windows: включить/снять одним окном UAC. Прежние значения политики
/// хранятся в записи владения (HKLM) и возвращаются при снятии.
#[cfg(target_os = "windows")]
pub fn set(domain: &str, server: Option<&str>) -> Result<BrowserPolicyStatus, String> {
    if policy_domain_known(domain).is_none() {
        return Err("Неизвестный браузер".into());
    }
    // Домен и вход моста проверены — в командную строку ничего чужого.
    let arguments = match server {
        Some(srv) => {
            valid_server(srv)?;
            format!("-Action Enable -Domain {domain} -Server {srv}")
        }
        None => format!("-Action Disable -Domain {domain}"),
    };
    let outcome = crate::elevate::run(
        "browser-policy",
        &[("browser-proxy-policy.ps1", SCRIPT)],
        &arguments,
    )?;
    if outcome.cancelled {
        return Err("Запрос прав администратора отменён".into());
    }
    let payload = outcome
        .result
        .ok_or_else(|| "Не удалось изменить политику браузера с правами администратора".to_string())?;
    let r = parse(&payload)?;
    // Не верим выводу elevated-процесса — перечитываем фактическое состояние.
    let real = status(domain)?;
    if server.is_some() && !real.configured {
        return Err("Не удалось подтвердить политику браузера".into());
    }
    if server.is_none() && real.managed {
        return Err("Не удалось снять политику браузера".into());
    }
    Ok(BrowserPolicyStatus { notice: r.notice, ..real })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_and_parse() {
        assert_eq!(policy_domain("/Applications/Google Chrome.app"), Some("com.google.Chrome"));
        assert_eq!(policy_domain("/Applications/Telegram.app"), None);
        let ok = r#"{"ok":true,"status":{"configured":true,"managed":true,"conflict":null,"auto_restore":true,"server":"127.0.0.1:8890","notice":null},"error":null}"#;
        assert!(parse(ok).unwrap().configured);
        assert!(parse(r#"{"ok":false,"status":null,"error":"x"}"#).is_err());
    }

    #[test]
    fn windows_browsers_by_exe() {
        let d = policy_domain;
        assert_eq!(d("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"), Some("com.google.Chrome"));
        assert_eq!(d("C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe"), Some("com.microsoft.Edge"));
        assert_eq!(
            d("C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
            Some("com.brave.Browser")
        );
        assert_eq!(d("C:\\Users\\u\\AppData\\Local\\Chromium\\Application\\CHROME.EXE"), Some("org.chromium.Chromium"));
        assert_eq!(d("C:\\Program Files\\Mozilla Firefox\\firefox.exe"), None);
        assert_eq!(d("D:\\Telegram Desktop\\Telegram.exe"), None);
    }

    /// Статус из реестра — те же случаи, что в tests/browser-proxy-policy.Tests.ps1.
    #[test]
    fn registry_status_matches_policy_script_rules() {
        use RegValue::{Absent, Other, Str};
        let s = |v: &str| Str(v.to_string());
        let ours = proxy_settings("127.0.0.1:8890");
        let owner = |proxy: &str, webrtc: &str| {
            s(&format!(
                r#"{{"owner":"ClodHelper","version":1,"server":"127.0.0.1:8890","original":{{"ProxySettings":{proxy},"WebRtcIPHandling":{webrtc}}}}}"#
            ))
        };
        let absent = r#"{"exists":false,"kind":null,"data":null}"#;
        let empty = r#"{"exists":true,"kind":"String","data":""}"#;
        let ours_webrtc = r#"{"exists":true,"kind":"String","data":"disable_non_proxied_udp"}"#;

        let st = status_from(&Absent, &Absent, false, &Absent).unwrap();
        assert!(!st.configured && !st.managed && st.conflict.is_none() && st.server.is_empty());

        let st = status_from(&s(&ours), &s(WEBRTC), false, &owner(absent, absent)).unwrap();
        assert!(st.configured && st.managed && st.conflict.is_none());
        assert_eq!(st.server, "127.0.0.1:8890");
        assert_eq!(ours, r#"{"ProxyMode":"fixed_servers","ProxyServer":"127.0.0.1:8890","ProxyBypassList":"<local>"}"#);

        // Чужое — конфликт: прокси, старые политики, WebRTC с другим значением.
        assert_eq!(status_from(&s(r#"{"ProxyMode":"direct"}"#), &Absent, false, &Absent).unwrap().conflict.as_deref(), Some(FOREIGN));
        assert_eq!(status_from(&Absent, &Absent, true, &Absent).unwrap().conflict.as_deref(), Some(FOREIGN));
        assert_eq!(status_from(&Absent, &s("default"), false, &Absent).unwrap().conflict.as_deref(), Some(FOREIGN));
        assert!(status_from(&Other("DWord".into()), &Absent, false, &Absent).unwrap().conflict.is_some());
        // Заменимые исходные значения — не конфликт.
        assert!(status_from(&s(""), &s(WEBRTC), false, &Absent).unwrap().conflict.is_none());
        // Старая политика рядом с нашей — не «настроено».
        let st = status_from(&s(&ours), &s(WEBRTC), true, &owner(absent, absent)).unwrap();
        assert!(!st.configured && st.conflict.is_some());

        // Изменено вне Helper — конфликт; вернулось к исходному — нет.
        let changed = status_from(&s(r#"{"ProxyMode":"direct"}"#), &s(WEBRTC), false, &owner(absent, absent)).unwrap();
        assert!(changed.conflict.unwrap().contains("изменена вне"));
        assert!(status_from(&s(""), &s(WEBRTC), false, &owner(empty, ours_webrtc)).unwrap().conflict.is_none());

        // Повреждённая запись владения никогда не принимается.
        for bad in [
            s("{"),
            s(r#"{"owner":"ClodHelper","version":1,"server":"10.0.0.1:8890","original":{"ProxySettings":{"exists":false},"WebRtcIPHandling":{"exists":false}}}"#),
            owner(r#"{"exists":true,"kind":"String","data":"{\"ProxyMode\":\"direct\"}"}"#, absent),
            owner(absent, r#"{"exists":true,"kind":"String","data":"default"}"#),
            Other("DWord".into()),
        ] {
            assert!(status_from(&Absent, &Absent, false, &bad).unwrap_err().contains("повреждена"));
        }
    }

    /// Ключи политик и запись владения совпадают со скриптом, который пишет.
    #[test]
    fn registry_keys_match_windows_script() {
        let ps = include_str!("../resources/browser-proxy-policy.ps1");
        for d in ["com.google.Chrome", "com.brave.Browser", "com.microsoft.Edge", "org.chromium.Chromium"] {
            assert!(ps.contains(&format!("'{d}' = '{}'", policy_key(d).unwrap())), "{d}");
        }
        assert!(ps.contains(&format!("'{OWNER_KEY}'")));
        assert!(ps.contains(&format!("'{WEBRTC}'")));
        assert!(LEGACY_PROXY.iter().all(|n| ps.contains(&format!("'{n}'"))));
        assert!(ps.contains("'У браузера уже есть чужая политика прокси — Clod Helper её не меняет.'"));
    }

    #[test]
    fn server_must_be_local_bridge() {
        assert!(valid_server("127.0.0.1:8890").is_ok());
        assert!(valid_server("127.0.0.1:8892").is_ok());
        for bad in ["127.0.0.1:", "127.0.0.1:0", "127.0.0.1:99999", "10.0.0.1:8890", "127.0.0.1:8890 -Action Disable"] {
            assert!(valid_server(bad).is_err(), "{bad}");
        }
        assert!(valid_server(PAC_SERVER).is_ok());
        for bad in ["pac:http://evil.example/proxy.pac", "pac:http://127.0.0.1:8887/proxy.pac -Action Disable"] {
            assert!(valid_server(bad).is_err(), "{bad}");
        }
    }

    /// PAC-режим: та же логика, что в скрипте — WebRTC остаётся исходным.
    #[test]
    fn pac_policy_status() {
        use RegValue::{Absent, Str};
        let pac = proxy_settings(PAC_SERVER);
        assert_eq!(pac, r#"{"ProxyMode":"pac_script","ProxyPacUrl":"http://127.0.0.1:8887/proxy.pac"}"#);
        let owner = Str(format!(
            r#"{{"owner":"ClodHelper","version":1,"server":"{PAC_SERVER}","original":{{"ProxySettings":{{"exists":false,"kind":null,"data":null}},"WebRtcIPHandling":{{"exists":false,"kind":null,"data":null}}}}}}"#
        ));
        let st = status_from(&Str(pac.clone()), &Absent, false, &owner).unwrap();
        assert!(st.configured && st.conflict.is_none());
        assert_eq!(st.server, PAC_SERVER);
        // Наш WebRTC-запрет при PAC — не «настроено» (так не должно остаться).
        assert!(!status_from(&Str(pac), &Str(WEBRTC.into()), false, &owner).unwrap().configured);
        // Скрипт строит ту же строку.
        let ps = include_str!("../resources/browser-proxy-policy.ps1");
        assert!(ps.contains(r#"'{{"ProxyMode":"pac_script","ProxyPacUrl":"{0}"}}'"#));
    }

    /// Живой статус: cargo test live_browser_policy -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_browser_policy() {
        println!("chrome policy → {:?}", status("com.google.Chrome"));
    }
}
