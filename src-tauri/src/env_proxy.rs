//! Управление ГЛОБАЛЬНЫМИ прокси-переменными Windows (User scope).
//!
//! Это ГЛАВНЫЙ механизм доставки прокси (модель Фазы 1). Опыт эксплуатации
//! показал: впрыскивание env только в дочерние процессы лаунчера хрупко
//! (VS Code пересобирает окружение терминала, MSIX глотает аргументы,
//! приложения запускают не через хелпер). Глобальные HTTP(S)_PROXY в User
//! scope наследует ЛЮБОЙ новый процесс - VS Code, его терминал, Claude Code,
//! git, node - без флагов, ярлыков и правки конфигов. Указывают они на
//! локальный мост (127.0.0.1:8889), так что «шляпа» не глобальная: браузеры
//! env-прокси не читают, а мост сам решает, что гнать через upstream.
//!
//! Читаем/пишем User-scope переменные окружения через реестр
//! HKCU\Environment (то же, что делает setx / System Properties).
//!
//! macOS: аналога User-scope env нет. Используем три слоя:
//! - `launchctl setenv` - переменные получают приложения, запущенные из
//!   Dock/Finder ПОСЛЕ включения (это и есть «состояние тумблера»);
//! - LaunchAgent, повторяющий setenv при входе в систему (иначе ребут
//!   стирает значения);
//! - управляемый блок в `~/.zshenv` - новые вкладки уже открытого Терминала
//!   (его окружение снято до включения) и ssh-сессии.
//!
//! На macOS имена регистрозависимы, а curl читает только `http_proxy` в
//! нижнем регистре - поэтому пишем оба варианта.

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
/// `HTTP_PROXY` и `http_proxy` - одна и та же запись. Поэтому после записи
/// UPPERCASE нельзя отдельно «удалять lowercase»: это удалит только что
/// записанное значение и тумблер мгновенно вернётся в состояние «выключен».
///
/// Действует на процессы, запущенные ПОСЛЕ (WM_SETTINGCHANGE рассылаем
/// автоматически). Прав администратора не требует (User scope).
pub fn set(proxy_url: &str) -> Result<(), String> {
    // Чужие значения (корпоративный прокси, свой VPN-тул) сохраняем ПЕРЕД
    // перезаписью: «выключить» обязано вернуть их, а не стереть насовсем -
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
/// Если при включении мы перезаписали чужие значения - возвращаем их, а не
/// просто зануляем (см. set()). Прав администратора НЕ требует. Уже
/// запущенные процессы (включая нас) свои унаследованные копии сохраняют до
/// перезапуска - это ожидаемо.
pub fn clear() -> Result<(), String> {
    let saved = crate::settings::load().saved_proxy_env.unwrap_or_default();
    backend::disable(&saved)?;
    let _ = crate::settings::set_saved_proxy_env(None);
    Ok(())
}

/// «Остановить мост»: переменные указывают на мост, и без него всё, что их
/// читает (Claude Code, VS Code, node, python), теряет сеть. Человек жмёт
/// «стоп», чтобы ходить напрямую, - снимаем их и помним, что вернуть.
pub fn pause_for_stop() {
    if !read().points_to_bridge {
        return;
    }
    match clear() {
        Ok(()) => {
            let _ = crate::settings::set_env_paused(true);
        }
        Err(e) => tracing::warn!("переменные прокси не сняты при остановке моста: {e}"),
    }
}

/// Мост снова работает: вернуть переменные, снятые остановкой.
pub fn resume_after_start() {
    if !crate::settings::load().env_paused {
        return;
    }
    match set(&format!("http://127.0.0.1:{}", crate::shim::LISTEN_PORT)) {
        Ok(()) => {
            let _ = crate::settings::set_env_paused(false);
        }
        Err(e) => tracing::warn!("переменные прокси не возвращены после запуска моста: {e}"),
    }
}

#[cfg(target_os = "windows")]
mod backend {
    /// User-переменные - прямо из HKCU\Environment (их хранилище): запуск
    /// PowerShell стоил ~1-2 с, а read() дёргается при каждом обновлении
    /// окна. Реестр регистронезависим - читаем только UPPERCASE-имена.
    pub fn read_values(keys: &[&str]) -> Vec<(String, String)> {
        keys.iter()
            .filter_map(|k| read_user_env(k).map(|v| (k.to_string(), v)))
            .collect()
    }

    /// Пишет HKCU\Environment напрямую (тип значения сохраняется) и один раз
    /// рассылает WM_SETTINGCHANGE, чтобы новые процессы унаследовали.
    /// Раньше это делал PowerShell: 1,5-2 с на запуск плюс рассылка на
    /// каждую переменную, и шаг мастера с тумблером ждали десятки секунд.
    pub fn enable(proxy_url: &str, no_proxy: &str) -> Result<(), String> {
        write_all(
            &[
                ("HTTP_PROXY", Some(proxy_url)),
                ("HTTPS_PROXY", Some(proxy_url)),
                ("NO_PROXY", Some(no_proxy)),
            ],
            "Не удалось записать переменные",
        )
    }

    /// Возвращает сохранённые чужие значения, остальные наши переменные
    /// удаляет из HKCU\Environment, и рассылает WM_SETTINGCHANGE.
    pub fn disable(saved: &std::collections::HashMap<String, String>) -> Result<(), String> {
        let changes: Vec<(&str, Option<&str>)> = ["HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"]
            .into_iter()
            .map(|key| (key, saved.get(key).map(String::as_str)))
            .collect();
        write_all(&changes, "Не удалось очистить переменные")
    }

    /// Как прежний скрипт: идём по всем переменным по порядку, даже если
    /// одна не записалась, и рассылаем изменение, если хоть что-то поменяли.
    fn write_all(changes: &[(&str, Option<&str>)], fail: &str) -> Result<(), String> {
        let mut errors = Vec::new();
        let mut changed = false;
        for (key, value) in changes {
            match crate::win::set_user_env(key, *value) {
                Ok(()) => changed = true,
                Err(e) => errors.push(format!("{key}: {e}")),
            }
        }
        if changed {
            crate::win::broadcast_environment_change();
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("{fail}: {}", errors.join("; ")))
        }
    }

    pub fn read_user_env(name: &str) -> Option<String> {
        let v = crate::win::user_env(name)?;
        let v = v.trim();
        (!v.is_empty()).then(|| v.to_string())
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
    pub const APPLY_ENV_FLAG: &str = "--apply-env";

    /// `clod-helper --apply-env K=V …` (из LaunchAgent при входе в систему):
    /// выставить переменные через launchctl и выйти, не поднимая GUI.
    /// Принимаем только наши ключи - агент не превращается в «setenv чего угодно».
    pub fn apply_env_cli(args: &[String]) -> Option<i32> {
        if args.get(1).map(String::as_str) != Some(APPLY_ENV_FLAG) {
            return None;
        }
        let mut code = 0;
        for pair in &args[2..] {
            let Some((k, v)) = pair.split_once('=') else { continue };
            if !KEYS.iter().any(|key| key.eq_ignore_ascii_case(k)) {
                continue;
            }
            if !macos::launchctl(&["setenv", k, v]).0 {
                code = 1;
            }
        }
        Some(code)
    }

    pub fn read_values(keys: &[&str]) -> Vec<(String, String)> {
        keys.iter()
            .filter_map(|k| read_user_env(k).map(|v| (k.to_string(), v)))
            .collect()
    }

    /// Переписать агент старого формата (`/bin/sh -c "launchctl setenv …"`)
    /// на `clod-helper --apply-env` с теми же значениями. Разово при старте.
    pub fn migrate_legacy_agent() {
        let Some(plist) = agent_plist().filter(|p| p.exists()) else { return };
        let Ok(text) = std::fs::read_to_string(&plist) else { return };
        if !text.contains("<string>/bin/sh</string>") {
            return;
        }
        let (Some(url), Some(no_proxy)) = (read_user_env("HTTPS_PROXY"), read_user_env("NO_PROXY"))
        else {
            return;
        };
        if let Err(e) = enable(&url, &no_proxy) {
            tracing::warn!("миграция агента переменных не удалась: {e}");
        }
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

        // Ребут стирает launchctl setenv - повторяем его при входе в систему.
        // Запускаем сам clod-helper (`--apply-env`), а не `/bin/sh -c`: в
        // «Объектах входа» безымянный «sh» выглядел как подозрительный объект.
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let mut program_arguments = vec![exe.to_string_lossy().to_string(), APPLY_ENV_FLAG.into()];
        program_arguments.extend(
            pairs
                .iter()
                .flat_map(|(k, v)| [(k.to_string(), *v), (k.to_lowercase(), *v)])
                .map(|(k, v)| format!("{k}={v}")),
        );
        let plist = agent_plist().ok_or_else(|| "Не нашёл домашнюю папку".to_string())?;
        macos::write_agent_plist(
            &plist,
            &macos::Agent {
                label: ENV_AGENT,
                program_arguments: &program_arguments,
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
        if body.is_none() && !text.contains(ZSH_BEGIN) {
            return Ok(());
        }
        let block = body.map(|b| format!("# Добавлено Clod Helper: прокси через локальный мост.\n{b}"));
        let out = macos::replace_managed_block(&text, ZSH_BEGIN, ZSH_END, block.as_deref());
        if out == text {
            return Ok(());
        }
        if out.is_empty() {
            // Файл создали мы и в нём больше ничего нет.
            let _ = std::fs::remove_file(&path);
            return Ok(());
        }
        std::fs::write(&path, out).map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
    }
}

#[cfg(not(target_os = "windows"))]
pub use backend::{apply_env_cli, migrate_legacy_agent};

#[cfg(all(test, not(target_os = "windows")))]
mod apply_env_tests {
    use super::apply_env_cli;

    #[test]
    fn not_our_flag_means_normal_launch() {
        let args = vec!["clod-helper".to_string(), "--tray".to_string()];
        assert_eq!(apply_env_cli(&args), None);
        assert_eq!(apply_env_cli(&["clod-helper".to_string()]), None);
    }

    #[test]
    fn foreign_keys_are_ignored() {
        // Только чужие/битые пары - launchctl не вызывается, выход 0.
        let args: Vec<String> = ["clod-helper", "--apply-env", "PATH=/evil", "garbage", "DYLD_INSERT_LIBRARIES=x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(apply_env_cli(&args), Some(0));
    }
}
