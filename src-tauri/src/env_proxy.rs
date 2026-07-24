//! Управление ГЛОБАЛЬНЫМИ прокси-переменными Windows (User scope).
//!
//! Это ГЛАВНЫЙ механизм доставки прокси (модель Фазы 1). Опыт эксплуатации
//! показал: впрыскивание env только в дочерние процессы лаунчера хрупко
//! (VS Code пересобирает окружение терминала, MSIX глотает аргументы,
//! приложения запускают не через хелпер). Глобальные HTTP(S)_PROXY в User
//! scope наследует ЛЮБОЙ новый процесс — VS Code, его терминал, Claude Code,
//! git, node — без флагов, ярлыков и правки конфигов. Указывают они на
//! локальный мост (127.0.0.1:8889), так что «шляпа» не глобальная: браузеры
//! env-прокси не читают, а мост сам решает, что гнать через upstream.
//!
//! Читаем/пишем User-scope переменные окружения через реестр
//! HKCU\Environment (то же, что делает setx / System Properties).

use serde::Serialize;

/// Текущее состояние глобальных прокси-переменных.
#[derive(Debug, Clone, Serialize)]
pub struct GlobalProxyEnv {
    /// Заданы ли какие-либо HTTP(S)_PROXY в User-scope.
    pub present: bool,
    /// Заданы ОБА HTTP_PROXY и HTTPS_PROXY, и все значения указывают на наш
    /// мост (127.0.0.1:8889). Без HTTPS_PROXY тумблер «ВКЛ» врал бы: весь
    /// https-трафик (т.е. Claude) шёл бы мимо моста.
    /// false при present=true → чужой/неполный прокси, UI показывает warning.
    pub points_to_bridge: bool,
    /// Значения найденных переменных (name → value) для показа в UI.
    pub values: Vec<(String, String)>,
}

/// Прочитать User-scope прокси-переменные из реестра.
///
/// ОДИН вызов PowerShell на все ключи: каждый спавн PS стоит ~0.5-1с, а read()
/// дёргается UI-поллингом каждые 5с. Читаем только UPPERCASE-имена — реестр
/// Windows регистронезависим, отдельный запрос lowercase вернул бы ту же
/// запись дублем.
pub fn read() -> GlobalProxyEnv {
    let read_keys = ["HTTP_PROXY", "HTTPS_PROXY"];
    let script = read_keys
        .iter()
        .map(|k| {
            format!(
                "Write-Output ('{k}=' + [Environment]::GetEnvironmentVariable('{k}', 'User'))"
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let out = run_ps_capture(&script).unwrap_or_default();

    let mut values = Vec::new();
    for line in out.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if !v.is_empty() && read_keys.iter().any(|rk| rk.eq_ignore_ascii_case(k)) {
                values.push((k.to_string(), v.to_string()));
            }
        }
    }
    let bridge_needle = format!("127.0.0.1:{}", crate::shim::LISTEN_PORT);
    let has_http = values
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("HTTP_PROXY"));
    let has_https = values
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("HTTPS_PROXY"));
    let points_to_bridge =
        has_http && has_https && values.iter().all(|(_, v)| v.contains(&bridge_needle));
    GlobalProxyEnv {
        present: !values.is_empty(),
        points_to_bridge,
        values,
    }
}

/// Прописать глобальные прокси-переменные → мост (User scope).
///
/// HTTP_PROXY/HTTPS_PROXY = URL моста; NO_PROXY мержится с существующим
/// значением пользователя (его домены-исключения сохраняем, локальные адреса
/// гарантируем). lowercase-варианты убираем: env Windows регистронезависим,
/// пара UPPER+lower в реестре только путает.
///
/// Действует на процессы, запущенные ПОСЛЕ (WM_SETTINGCHANGE рассылается
/// автоматически). Прав администратора не требует (User scope).
pub fn set(proxy_url: &str) -> Result<(), String> {
    // Чужие значения (корпоративный прокси, свой VPN-тул) сохраняем ПЕРЕД
    // перезаписью: «выключить» обязано вернуть их, а не стереть насовсем —
    // иначе у пользователя «после вашей проги сломался интернет для CLI».
    let bridge_needle = format!("127.0.0.1:{}", crate::shim::LISTEN_PORT);
    let mut foreign = std::collections::HashMap::new();
    for key in ["HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"] {
        if let Some(v) = read_user_env(key) {
            if !v.contains(&bridge_needle) {
                foreign.insert(key.to_string(), v);
            }
        }
    }
    if !foreign.is_empty() {
        let _ = crate::settings::set_saved_proxy_env(Some(foreign));
    }

    let no_proxy = merged_no_proxy();
    let url_e = proxy_url.replace('\'', "''");
    let np_e = no_proxy.replace('\'', "''");
    let script = format!(
        "[Environment]::SetEnvironmentVariable('HTTP_PROXY', '{url_e}', 'User'); \
         [Environment]::SetEnvironmentVariable('HTTPS_PROXY', '{url_e}', 'User'); \
         [Environment]::SetEnvironmentVariable('NO_PROXY', '{np_e}', 'User'); \
         [Environment]::SetEnvironmentVariable('http_proxy', $null, 'User'); \
         [Environment]::SetEnvironmentVariable('https_proxy', $null, 'User'); \
         Write-Output 'ENV_SET'"
    );
    let out = run_ps_capture(&script)
        .ok_or_else(|| "powershell.exe не выполнил запись переменных".to_string())?;
    if out.contains("ENV_SET") {
        Ok(())
    } else {
        Err(format!("Не удалось записать переменные: {out}"))
    }
}

/// Существующий NO_PROXY пользователя + обязательные локальные адреса.
fn merged_no_proxy() -> String {
    const REQUIRED: &[&str] = &["localhost", "127.0.0.1", "::1"];
    let existing = read_user_env("NO_PROXY").unwrap_or_default();
    let mut parts: Vec<String> = existing
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for r in REQUIRED {
        if !parts.iter().any(|p| p.eq_ignore_ascii_case(r)) {
            parts.push((*r).to_string());
        }
    }
    parts.join(",")
}

/// Удалить все глобальные HTTP(S)_PROXY из User-scope.
///
/// Использует `[Environment]::SetEnvironmentVariable(name, $null, 'User')` —
/// корректно удаляет из HKCU\Environment и рассылает WM_SETTINGCHANGE, чтобы
/// новые процессы не унаследовали. Прав администратора НЕ требует (User scope).
/// Уже запущенные процессы (включая нас) свои унаследованные копии сохраняют
/// до перезапуска — это ожидаемо.
pub fn clear() -> Result<(), String> {
    // Если при включении мы перезаписали чужие значения — возвращаем их,
    // а не просто зануляем (см. set()).
    let saved = crate::settings::load().saved_proxy_env.unwrap_or_default();
    let mut script = String::new();
    for key in ["HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"] {
        match saved.get(key) {
            Some(v) => script.push_str(&format!(
                "[Environment]::SetEnvironmentVariable('{key}', '{}', 'User'); ",
                v.replace('\'', "''"),
            )),
            None => script.push_str(&format!(
                "[Environment]::SetEnvironmentVariable('{key}', $null, 'User'); ",
            )),
        }
    }
    for key in ["http_proxy", "https_proxy"] {
        script.push_str(&format!(
            "[Environment]::SetEnvironmentVariable('{key}', $null, 'User'); ",
        ));
    }
    script.push_str("Write-Output 'ENV_CLEARED'");

    let out = run_ps_capture(&script)
        .ok_or_else(|| "powershell.exe не выполнил очистку".to_string())?;
    if out.contains("ENV_CLEARED") {
        let _ = crate::settings::set_saved_proxy_env(None);
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
    // UTF-8-прелюдия: без неё PS 5.1 пишет в пайп OEM-866 и кириллица в
    // значениях/ошибках превращается в кашу (см. tasks::run_ps).
    let script = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $OutputEncoding=[System.Text.Encoding]::UTF8; {script}"
    );
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script.as_str(),
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
