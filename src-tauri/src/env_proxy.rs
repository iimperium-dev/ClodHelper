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
//!
//! macOS: аналога User-scope env нет. Используем три слоя:
//! - `launchctl setenv` — переменные получают приложения, запущенные из
//!   Dock/Finder ПОСЛЕ включения (это и есть «состояние тумблера»);
//! - LaunchAgent, повторяющий setenv при входе в систему (иначе ребут
//!   стирает значения);
//! - управляемый блок в `~/.zshenv` — новые вкладки уже открытого Терминала
//!   (его окружение снято до включения) и ssh-сессии.
//!
//! На macOS имена регистрозависимы, а curl читает только `http_proxy` в
//! нижнем регистре — поэтому пишем оба варианта.

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

const READ_KEYS: [&str; 2] = ["HTTP_PROXY", "HTTPS_PROXY"];

/// Прочитать User-scope прокси-переменные.
pub fn read() -> GlobalProxyEnv {
    let values = backend::read_values(&READ_KEYS);
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
/// гарантируем). В Windows имена переменных окружения регистронезависимы:
/// `HTTP_PROXY` и `http_proxy` — одна и та же запись. Поэтому после записи
/// UPPERCASE нельзя отдельно «удалять lowercase»: это удалит только что
/// записанное значение и тумблер мгновенно вернётся в состояние «выключен».
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
        if let Some(v) = backend::read_user_env(key) {
            if !v.contains(&bridge_needle) {
                foreign.insert(key.to_string(), v);
            }
        }
    }
    if !foreign.is_empty() {
        let _ = crate::settings::set_saved_proxy_env(Some(foreign));
    }
    backend::enable(proxy_url, &merged_no_proxy())
}

/// Существующий NO_PROXY пользователя + обязательные локальные адреса.
fn merged_no_proxy() -> String {
    const REQUIRED: &[&str] = &["localhost", "127.0.0.1", "::1"];
    let existing = backend::read_user_env("NO_PROXY").unwrap_or_default();
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
/// Если при включении мы перезаписали чужие значения — возвращаем их, а не
/// просто зануляем (см. set()). Прав администратора НЕ требует. Уже
/// запущенные процессы (включая нас) свои унаследованные копии сохраняют до
/// перезапуска — это ожидаемо.
pub fn clear() -> Result<(), String> {
    let saved = crate::settings::load().saved_proxy_env.unwrap_or_default();
    backend::disable(&saved)?;
    let _ = crate::settings::set_saved_proxy_env(None);
    Ok(())
}

#[cfg(target_os = "windows")]
mod backend {
    /// ОДИН вызов PowerShell на все ключи: каждый спавн PS стоит ~0.5-1с, а
    /// read() дёргается UI-поллингом каждые 5с. Читаем только UPPERCASE-имена
    /// — реестр Windows регистронезависим, отдельный запрос lowercase вернул
    /// бы ту же запись дублем.
    pub fn read_values(keys: &[&str]) -> Vec<(String, String)> {
        let script = keys
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
                if !v.is_empty() && keys.iter().any(|rk| rk.eq_ignore_ascii_case(k)) {
                    values.push((k.to_string(), v.to_string()));
                }
            }
        }
        values
    }

    /// Записывает HKCU\Environment через [Environment]::SetEnvironmentVariable
    /// — он же рассылает WM_SETTINGCHANGE, чтобы новые процессы унаследовали.
    pub fn enable(proxy_url: &str, no_proxy: &str) -> Result<(), String> {
        let url_e = proxy_url.replace('\'', "''");
        let np_e = no_proxy.replace('\'', "''");
        let script = format!(
            "[Environment]::SetEnvironmentVariable('HTTP_PROXY', '{url_e}', 'User'); \
             [Environment]::SetEnvironmentVariable('HTTPS_PROXY', '{url_e}', 'User'); \
             [Environment]::SetEnvironmentVariable('NO_PROXY', '{np_e}', 'User'); \
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

    /// `SetEnvironmentVariable(name, $null, 'User')` корректно удаляет из
    /// HKCU\Environment и рассылает WM_SETTINGCHANGE.
    pub fn disable(saved: &std::collections::HashMap<String, String>) -> Result<(), String> {
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
        script.push_str("Write-Output 'ENV_CLEARED'");

        let out = run_ps_capture(&script)
            .ok_or_else(|| "powershell.exe не выполнил очистку".to_string())?;
        if out.contains("ENV_CLEARED") {
            Ok(())
        } else {
            Err(format!("Не удалось очистить переменные: {out}"))
        }
    }

    pub fn read_user_env(name: &str) -> Option<String> {
        run_ps_capture(&format!(
            "[Environment]::GetEnvironmentVariable('{name}', 'User')"
        ))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    }

    fn run_ps_capture(script: &str) -> Option<String> {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
        cmd.creation_flags(CREATE_NO_WINDOW);
        let out = cmd.output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

#[cfg(not(target_os = "windows"))]
mod backend {
    use std::path::PathBuf;

    use crate::macos;

    /// LaunchAgent, повторяющий `launchctl setenv` при каждом входе в систему.
    const ENV_AGENT: &str = "ru.iimperium.clodhelper.env";
    const ZSH_BEGIN: &str = "# >>> clod-helper proxy >>>";
    const ZSH_END: &str = "# <<< clod-helper proxy <<<";
    const KEYS: [&str; 3] = ["HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"];

    pub fn read_values(keys: &[&str]) -> Vec<(String, String)> {
        keys.iter()
            .filter_map(|k| read_user_env(k).map(|v| (k.to_string(), v)))
            .collect()
    }

    pub fn read_user_env(name: &str) -> Option<String> {
        let (ok, out) = macos::launchctl(&["getenv", name]);
        if !ok {
            return None;
        }
        Some(out.trim().to_string()).filter(|s| !s.is_empty())
    }

    pub fn enable(proxy_url: &str, no_proxy: &str) -> Result<(), String> {
        let pairs = [
            ("HTTP_PROXY", proxy_url),
            ("HTTPS_PROXY", proxy_url),
            ("NO_PROXY", no_proxy),
        ];
        for (key, value) in pairs {
            for name in [key.to_string(), key.to_lowercase()] {
                let (ok, out) = macos::launchctl(&["setenv", &name, value]);
                if !ok {
                    return Err(format!("launchctl setenv {name}: {}", out.trim()));
                }
            }
        }

        // Ребут стирает launchctl setenv — повторяем его при входе в систему.
        let script: String = pairs
            .iter()
            .flat_map(|(k, v)| [(k.to_string(), *v), (k.to_lowercase(), *v)])
            .map(|(k, v)| format!("/bin/launchctl setenv {k} {}; ", macos::sh_quote(v)))
            .collect();
        let plist = agent_plist().ok_or_else(|| "Не нашёл домашнюю папку".to_string())?;
        macos::write_agent_plist(
            &plist,
            &macos::Agent {
                label: ENV_AGENT,
                program_arguments: &["/bin/sh".into(), "-c".into(), script],
                keep_alive: false,
                stderr_log: None,
            },
        )?;

        let exports: String = pairs
            .iter()
            .map(|(k, v)| {
                let q = macos::sh_quote(v);
                format!("export {k}={q} {}={q}\n", k.to_lowercase())
            })
            .collect();
        write_zsh_block(Some(&exports))
    }

    pub fn disable(saved: &std::collections::HashMap<String, String>) -> Result<(), String> {
        write_zsh_block(None)?;
        if let Some(plist) = agent_plist().filter(|p| p.exists()) {
            macos::bootout(ENV_AGENT);
            std::fs::remove_file(&plist)
                .map_err(|e| format!("Не удалось удалить {}: {e}", plist.display()))?;
        }
        for key in KEYS {
            for name in [key.to_string(), key.to_lowercase()] {
                let (ok, out) = match saved.get(key) {
                    Some(v) => macos::launchctl(&["setenv", &name, v]),
                    None => macos::launchctl(&["unsetenv", &name]),
                };
                if !ok {
                    return Err(format!("launchctl {name}: {}", out.trim()));
                }
            }
        }
        Ok(())
    }

    fn agent_plist() -> Option<PathBuf> {
        macos::launch_agents_dir().map(|d| d.join(format!("{ENV_AGENT}.plist")))
    }

    /// Заменить (или убрать при None) наш блок в ~/.zshenv, не трогая
    /// остальное содержимое файла.
    fn write_zsh_block(body: Option<&str>) -> Result<(), String> {
        let path = dirs::home_dir()
            .ok_or_else(|| "Не нашёл домашнюю папку".to_string())?
            .join(".zshenv");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut out = String::new();
        let mut skipping = false;
        for line in text.lines() {
            if line.trim() == ZSH_BEGIN {
                skipping = true;
                continue;
            }
            if skipping {
                if line.trim() == ZSH_END {
                    skipping = false;
                }
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        if let Some(body) = body {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&format!(
                "{ZSH_BEGIN}\n# Добавлено Clod Helper: прокси через локальный мост.\n{body}{ZSH_END}\n"
            ));
        }
        if out == text {
            return Ok(());
        }
        if out.trim().is_empty() && body.is_none() {
            // Файл создали мы и в нём больше ничего нет.
            let _ = std::fs::remove_file(&path);
            return Ok(());
        }
        std::fs::write(&path, out).map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
    }
}
