//! «Защита от утечек» (macOS): брандмауэр pf запрещает прямой выход к сетям
//! Claude (Anthropic) и Telegram (resources/leak-guard.sh). Мост ходит к ним
//! через прокси и не задет; программа, пошедшая мимо моста, получает отказ
//! вместо утечки IP. Правила — в своём якоре, служба возвращает их после
//! перезагрузки. Включение/снятие — одно системное окно пароля.

use serde::{Deserialize, Serialize};

#[cfg(not(target_os = "windows"))]
const SCRIPT: &str = include_str!("../resources/leak-guard.sh");

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
/// (block return) мгновенно, без защиты — соединение устанавливается.
pub fn probe_claude_blocked() -> Option<bool> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr: SocketAddr = "160.79.104.10:443".parse().ok()?;
    match TcpStream::connect_timeout(&addr, Duration::from_secs(4)) {
        Ok(_) => Some(false),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => Some(true),
        // Таймаут/нет сети — по этой проверке не понять.
        Err(_) => None,
    }
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
    let mut st = parse(&String::from_utf8_lossy(&out.stdout))?;
    if st.enabled && st.services.contains("claude") {
        st.verified = probe_claude_blocked();
    }
    Ok(st)
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
    // Не верим выводу root-процесса — перечитываем и проверяем вживую.
    let real = status()?;
    if !services.is_empty() && (!real.enabled || real.verified == Some(false)) {
        return Err("Не удалось подтвердить правила брандмауэра".into());
    }
    if services.is_empty() && real.enabled {
        return Err("Не удалось снять защиту от утечек".into());
    }
    Ok(real)
}

#[cfg(target_os = "windows")]
pub fn status() -> Result<LeakGuardStatus, String> {
    Ok(LeakGuardStatus::default())
}

#[cfg(target_os = "windows")]
pub fn set(_services: &str) -> Result<LeakGuardStatus, String> {
    Err("Защита от утечек пока есть только в версии для macOS".into())
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

    /// Живой статус: cargo test live_leak_guard -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn live_leak_guard() {
        println!("leak guard → {:?}", status());
    }
}
