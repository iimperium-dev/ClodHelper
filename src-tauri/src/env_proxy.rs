//! Управление ГЛОБАЛЬНЫМИ прокси-переменными Windows (User scope).
//!
//! Зачем: раньше проксирование включалось глобальными User env-переменными
//! HTTP_PROXY/HTTPS_PROXY — их видел ЛЮБОЙ процесс на машине. Теперь модель
//! другая: прокси впрыскивается только в приложения, запущенные через профили
//! (env в дочернем процессе). Глобальные переменные становятся лишними и даже
//! вредными — их надо убрать из системы, чтобы «шляпа» не висела глобально.
//!
//! Читаем/пишем User-scope переменные окружения через реестр
//! HKCU\Environment (то же, что делает setx / System Properties).

use serde::Serialize;

const KEYS: &[&str] = &["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"];

/// Текущее состояние глобальных прокси-переменных.
#[derive(Debug, Clone, Serialize)]
pub struct GlobalProxyEnv {
    /// Заданы ли какие-либо HTTP(S)_PROXY в User-scope.
    pub present: bool,
    /// Значения найденных переменных (name → value) для показа в UI.
    pub values: Vec<(String, String)>,
}

/// Прочитать User-scope прокси-переменные из реестра.
pub fn read() -> GlobalProxyEnv {
    let mut values = Vec::new();
    for key in KEYS {
        if let Some(v) = read_user_env(key) {
            if !v.trim().is_empty() {
                values.push(((*key).to_string(), v));
            }
        }
    }
    GlobalProxyEnv {
        present: !values.is_empty(),
        values,
    }
}

/// Удалить все глобальные HTTP(S)_PROXY из User-scope.
///
/// Использует `[Environment]::SetEnvironmentVariable(name, $null, 'User')` —
/// корректно удаляет из HKCU\Environment и рассылает WM_SETTINGCHANGE, чтобы
/// новые процессы не унаследовали. Прав администратора НЕ требует (User scope).
/// Уже запущенные процессы (включая нас) свои унаследованные копии сохраняют
/// до перезапуска — это ожидаемо.
pub fn clear() -> Result<(), String> {
    let mut script = String::new();
    for key in KEYS {
        script.push_str(&format!(
            "[Environment]::SetEnvironmentVariable('{key}', $null, 'User'); ",
        ));
    }
    script.push_str("Write-Output 'ENV_CLEARED'");

    let out = run_ps_capture(&script)
        .ok_or_else(|| "powershell.exe не выполнил очистку".to_string())?;
    if out.contains("ENV_CLEARED") {
        Ok(())
    } else {
        Err(format!("Не удалось очистить переменные: {out}"))
    }
}

fn read_user_env(name: &str) -> Option<String> {
    run_ps_capture(&format!(
        "[Environment]::GetEnvironmentVariable('{name}', 'User')"
    ))
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

fn run_ps_capture(script: &str) -> Option<String> {
    use std::process::Command;
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}
