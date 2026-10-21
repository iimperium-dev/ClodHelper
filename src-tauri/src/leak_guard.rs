//! «Защита от утечек»: брандмауэр запрещает прямой выход к сетям Claude
//! (Anthropic) и Telegram. Мост ходит к ним через прокси и не задет;
//! программа, пошедшая мимо моста, получает отказ вместо утечки IP.
//!   - macOS: pf, правила в своём якоре (resources/leak-guard.sh), служба
//!     возвращает их после перезагрузки; одно системное окно пароля.
//!   - Windows: исходящие правила брандмауэра Windows в своей группе
//!     (resources/leak-guard.ps1) — постоянные сами по себе; одно окно UAC.

use serde::{Deserialize, Serialize};

#[cfg(not(target_os = "windows"))]
pub(crate) const SCRIPT: &str = include_str!("../resources/leak-guard.sh");
#[cfg(target_os = "windows")]
pub(crate) const SCRIPT: &str = include_str!("../resources/leak-guard.ps1");

/// Сервисы, которые умеет закрывать защита.
pub const SERVICES: [&str; 2] = ["claude", "telegram"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LeakGuardStatus {
    pub enabled: bool,
    /// "claude", "telegram" или "claude,telegram".
    #[serde(default)]
    pub services: String,
    #[serde(default)]
    pub auto_restore: bool,
    /// Живая проверка: прямое соединение с Anthropic действительно отклонено.
    #[serde(default)]
    pub verified: Option<bool>,
}

#[derive(Deserialize)]
struct ScriptResult {
    ok: bool,
    status: Option<LeakGuardStatus>,
    error: Option<String>,
}

fn parse(payload: &str) -> Result<LeakGuardStatus, String> {
    let r: ScriptResult = serde_json::from_str(payload.trim())
        .map_err(|e| format!("Некорректный ответ защиты от утечек: {e}"))?;
    if !r.ok {
        return Err(r.error.unwrap_or_else(|| "Защита от утечек: ошибка".into()));
    }
    r.status.ok_or_else(|| "Защита от утечек: нет состояния".into())
}

/// Список сервисов в каноничном виде ("claude,telegram"); пустой — снять защиту.
pub fn normalize(services: &[String]) -> Result<String, String> {
    let mut out = Vec::new();
    for s in SERVICES {
        if services.iter().any(|x| x == s) {
            out.push(s);
        }
    }
    if services.iter().any(|x| !SERVICES.contains(&x.as_str())) {
        return Err("Неизвестный сервис".into());
    }
    Ok(out.join(","))
}

/// Текущий список с добавленным (on) или убранным сервисом — для «Подключить»
/// у Claude/Telegram и переключателя в окне приложения.
pub fn with_service(current: &LeakGuardStatus, service: &str, on: bool) -> Result<String, String> {
    let mut list: Vec<String> = if current.enabled {
        current.services.split(',').filter(|x| !x.is_empty()).map(String::from).collect()
    } else {
        Vec::new()
    };
    list.retain(|x| x != service);
    if on {
        list.push(service.to_string());
    }
    normalize(&list)
}

/// Фрагмент shell для root: включить защиту для `services` (проверенного
/// normalize). Вывод глушится — его вставляют в чужую root-команду (окно
/// пароля Claude), где stdout занят своим JSON.
#[cfg(not(target_os = "windows"))]
pub fn shell_enable(services: &str) -> String {
    use base64::Engine;
    let enc = base64::engine::general_purpose::STANDARD.encode(SCRIPT);
    format!(
        "/bin/mkdir -p '{dir}' && /usr/sbin/chown root:wheel '{dir}' && /bin/chmod 755 '{dir}' && \
         /bin/echo {enc} | /usr/bin/base64 -D > '{dir}/leak-guard.sh'; \
         /bin/echo {enc} | /usr/bin/base64 -D | /bin/sh -s -- Enable {services} >/dev/null 2>&1",
        dir = "/Library/Application Support/ClodHelper",
    )
}

/// Действительно ли прямой выход к Anthropic закрыт: pf отвечает отказом
/// (block return) мгновенно, брандмауэр Windows — запретом доступа (WSAEACCES),
/// без защиты — соединение устанавливается.
pub fn probe_claude_blocked() -> Option<bool> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr: SocketAddr = "160.79.104.10:443".parse().ok()?;
    probe_verdict(TcpStream::connect_timeout(&addr, Duration::from_secs(4)).map(|_| ()))
}

fn probe_verdict(connect: std::io::Result<()>) -> Option<bool> {
    use std::io::ErrorKind;
    match connect {
        Ok(()) => Some(false),
        Err(e) if matches!(e.kind(), ErrorKind::ConnectionRefused | ErrorKind::PermissionDenied) => Some(true),
        // Таймаут/нет сети — по этой проверке не понять.
        Err(_) => None,
    }
}

/// Живая проверка — только когда защита закрывает Claude.
fn probed(mut st: LeakGuardStatus) -> LeakGuardStatus {
    if st.enabled && st.services.split(',').any(|s| s == "claude") {
        st.verified = probe_claude_blocked();
    }
    st
}

/// Не верим выводу привилегированного процесса: состояние перечитано, а для
/// Claude ещё и проверено вживую.
fn confirm(services: &str, real: LeakGuardStatus) -> Result<LeakGuardStatus, String> {
    if services.is_empty() {
        return if real.enabled {
            Err("Не удалось снять защиту от утечек".into())
        } else {
            Ok(real)
        };
    }
    if !real.enabled || real.services != services {
        return Err("Не удалось подтвердить правила брандмауэра".into());
    }
    if real.verified == Some(false) {
        return Err(if cfg!(target_os = "windows") {
            "Правила добавлены, но прямое соединение с Claude не блокируется — проверьте, что брандмауэр Windows включён".into()
        } else {
            "Не удалось подтвердить правила брандмауэра".into()
        });
    }
    Ok(real)
}

#[cfg(not(target_os = "windows"))]
pub fn status() -> Result<LeakGuardStatus, String> {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-s", "--", "Status"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Защита от утечек: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(SCRIPT.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| format!("Защита от утечек: {e}"))?;
    Ok(probed(parse(&String::from_utf8_lossy(&out.stdout))?))
}

/// Включить для `services` ("claude,telegram") или снять (пустая строка).
/// Enable кладёт копию скрипта в root-папку — её запускает служба после
/// перезагрузки; пользовательский процесс подменить её не может.
#[cfg(not(target_os = "windows"))]
pub fn set(services: &str) -> Result<LeakGuardStatus, String> {
    use base64::Engine;
    let enc = base64::engine::general_purpose::STANDARD.encode(SCRIPT);
    let shell = if services.is_empty() {
        format!("/bin/echo {enc} | /usr/bin/base64 -D | /bin/sh -s -- Disable; exit 0")
    } else {
        // services уже проверен normalize(): только claude/telegram и запятая.
        format!("{}; /bin/sh '/Library/Application Support/ClodHelper/leak-guard.sh' Status; exit 0", shell_enable(services))
    };
    let apple = format!(
        "do shell script \"{shell}\" with prompt \"Clod Helper включает защиту от утечек в брандмауэре.\" with administrator privileges"
    );
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &apple])
        .output()
        .map_err(|e| format!("Не удалось запросить права администратора: {e}"))?;
    if String::from_utf8_lossy(&out.stderr).contains("-128") {
        return Err("Запрос пароля отменён".into());
    }
    parse(&String::from_utf8_lossy(&out.stdout))?;
    confirm(services, status()?)
}

/// Имя (InstanceID) правила брандмауэра — как Get-LeakGuardRuleName в скрипте.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn rule_name(service: &str) -> String {
    format!("ClodHelper-LeakGuard-{service}")
}

/// Правило из хранилища брандмауэра («v2.33|Action=Block|Active=TRUE|Dir=Out|…»)
/// действует: включено, исходящее, «Блокировать».
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn rule_blocks(rule: &str) -> bool {
    let fields: Vec<&str> = rule.split('|').collect();
    ["Action=Block", "Active=TRUE", "Dir=Out"].iter().all(|f| fields.contains(f))
}

/// Windows: правила брандмауэра читаются без прав администратора — прямо из
/// его хранилища в реестре (Get-NetFirewallRule через PowerShell — секунды).
#[cfg(target_os = "windows")]
pub fn status() -> Result<LeakGuardStatus, String> {
    let mut on = Vec::new();
    for service in SERVICES {
        match crate::win::firewall_rule(&rule_name(service)) {
            Ok(Some(rule)) if rule_blocks(&rule) => on.push(service),
            Ok(_) => {}
            // Хранилище закрыто для чтения — спросим сам брандмауэр.
            Err(_) => {
                let payload = crate::elevate::read_status(SCRIPT, "-Action Status")
                    .map_err(|e| format!("Защита от утечек: {e}"))?;
                return Ok(probed(parse(&payload)?));
            }
        }
    }
    let services = on.join(",");
    Ok(probed(LeakGuardStatus {
        enabled: !services.is_empty(),
        auto_restore: !services.is_empty(),
        services,
        verified: None,
    }))
}

/// Windows: включить для `services` ("claude,telegram") или снять (пустая
/// строка) — одно окно UAC. Правила постоянные: служба не нужна.
#[cfg(target_os = "windows")]
pub fn set(services: &str) -> Result<LeakGuardStatus, String> {
    // services уже проверен normalize(): только claude/telegram и запятая.
    let arguments = if services.is_empty() {
        "-Action Disable".to_string()
    } else {
        format!("-Action Enable -Services {services}")
    };
    let outcome = crate::elevate::run("leak-guard", &[("leak-guard.ps1", SCRIPT)], &arguments)?;
    if outcome.cancelled {
        return Err("Запрос прав администратора отменён".into());
    }
    let payload = outcome
        .result
        .ok_or_else(|| "Не удалось изменить правила брандмауэра с правами администратора".to_string())?;
    parse(&payload)?;
    confirm(services, status()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_normalize() {
        let ok = r#"{"ok":true,"status":{"enabled":true,"services":"claude,telegram","auto_restore":true},"error":null}"#;
        let st = parse(ok).unwrap();
        assert!(st.enabled && st.auto_restore && st.verified.is_none());
        assert!(parse(r#"{"ok":false,"status":null,"error":"x"}"#).is_err());
        let v = |a: &[&str]| normalize(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(v(&["telegram", "claude"]).unwrap(), "claude,telegram");
        assert_eq!(v(&[]).unwrap(), "");
        assert!(v(&["claude; rm -rf /"]).is_err(), "в shell попадают только известные имена");
        let on = LeakGuardStatus { enabled: true, services: "telegram".into(), ..Default::default() };
        assert_eq!(with_service(&on, "claude", true).unwrap(), "claude,telegram");
        assert_eq!(with_service(&on, "telegram", false).unwrap(), "");
        assert_eq!(with_service(&LeakGuardStatus::default(), "claude", true).unwrap(), "claude");
    }

    #[test]
    fn probe_counts_refusal_and_firewall_denial_as_blocked() {
        use std::io::{Error, ErrorKind};
        assert_eq!(probe_verdict(Ok(())), Some(false));
        // pf (block return) — отказ; брандмауэр Windows — WSAEACCES.
        assert_eq!(probe_verdict(Err(Error::from(ErrorKind::ConnectionRefused))), Some(true));
        assert_eq!(probe_verdict(Err(Error::from_raw_os_error(if cfg!(windows) { 10013 } else { 13 }))), Some(true));
        assert_eq!(probe_verdict(Err(Error::from(ErrorKind::TimedOut))), None);
    }

    #[test]
    fn confirm_trusts_only_the_reread_state() {
        let st = |enabled: bool, services: &str, verified| LeakGuardStatus {
            enabled,
            services: services.into(),
            auto_restore: enabled,
            verified,
        };
        assert!(confirm("claude", st(true, "claude", Some(true))).is_ok());
        assert!(confirm("telegram", st(true, "telegram", None)).is_ok());
        assert!(confirm("claude,telegram", st(true, "claude", Some(true))).is_err(), "Telegram не встал");
        assert!(confirm("claude", st(false, "", None)).is_err());
        assert!(confirm("claude", st(true, "claude", Some(false))).is_err(), "правило есть, но не блокирует");
        assert!(confirm("", st(false, "", None)).is_ok());
        assert!(confirm("", st(true, "telegram", None)).is_err());
    }

    #[test]
    fn firewall_store_rules() {
        assert!(rule_blocks("v2.33|Action=Block|Active=TRUE|Dir=Out|RA4=160.79.104.0/255.255.248.0|Name=x|EmbedCtxt=ClodHelper LeakGuard|"));
        assert!(!rule_blocks("v2.33|Action=Block|Active=FALSE|Dir=Out|"), "выключенное правило не защищает");
        assert!(!rule_blocks("v2.33|Action=Allow|Active=TRUE|Dir=Out|"));
        assert!(!rule_blocks("v2.33|Action=Block|Active=TRUE|Dir=In|"));
        // Имя правила — то же, что создаёт скрипт Windows.
        let ps = include_str!("../resources/leak-guard.ps1");
        assert!(ps.contains("\"ClodHelper-LeakGuard-$Service\""));
        assert_eq!(rule_name("claude"), "ClodHelper-LeakGuard-claude");
    }

    /// Сети одинаковы в pf-скрипте macOS и в правилах брандмауэра Windows.
    #[test]
    fn mac_and_windows_block_the_same_networks() {
        let sh = include_str!("../resources/leak-guard.sh");
        let ps = include_str!("../resources/leak-guard.ps1");
        for service in SERVICES {
            let sh_line = sh
                .lines()
                .find(|l| l.contains(&format!("*,{service},*)")))
                .expect("сети сервиса в leak-guard.sh");
            let ps_line = ps
                .lines()
                .find(|l| l.trim_start().starts_with(&format!("{service} = @(")))
                .expect("сети сервиса в leak-guard.ps1");
            let nets = |l: &str| {
                let mut v: Vec<String> = l
                    .split(|c: char| !(c.is_ascii_hexdigit() || matches!(c, '.' | ':' | '/')))
                    .filter(|t| t.contains('/') && (t.contains('.') || t.contains(':')))
                    .map(String::from)
                    .collect();
                v.sort();
                v
            };
            assert!(!nets(sh_line).is_empty());
            assert_eq!(nets(sh_line), nets(ps_line), "сети {service}");
        }
    }

    /// Живой статус: cargo test live_leak_guard -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_leak_guard() {
        println!("leak guard → {:?}", status());
    }
}
