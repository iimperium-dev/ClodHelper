//! Автозапуск GUI при входе в систему (в трей, `--tray`).
//!
//! Windows — tauri-plugin-autostart (ключ Run в реестре), как раньше.
//!
//! macOS 13+ — SMAppService: агент `ru.iimperium.clodhelper.autostart.plist`
//! лежит ВНУТРИ бандла (Contents/Library/LaunchAgents), поэтому «Объекты
//! входа» показывают его как Clod Helper с иконкой, а не как LaunchAgent
//! «Clod Helper.plist» с пробелом в метке, который писал плагин.
//!
//! Агент запускает не GUI, а `clod-helper --login-item`: тот просит
//! LaunchServices открыть приложение (`open -g -j … --args --tray`) и сразу
//! выходит. Иначе GUI жил бы внутри launchd-задачи агента, и
//! SMAppService.unregister() (выключение тумблера) убивал бы открытое окно,
//! а register() (включение) поднимал бы вторую копию приложения.
//!
//! macOS 11–12 (SMAppService ещё нет) — тот же `--login-item` через свой
//! LaunchAgent в ~/Library/LaunchAgents; он подхватится при следующем входе.

use serde::Serialize;
#[cfg(target_os = "windows")]
use tauri::AppHandle;

/// Состояние автозапуска для UI.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct AutostartState {
    pub enabled: bool,
    /// macOS: агент зарегистрирован, но пользователь должен разрешить его в
    /// «Системные настройки → Основные → Объекты входа».
    pub needs_approval: bool,
}

/// Флаг агента входа (см. шапку модуля).
pub const LOGIN_ITEM_FLAG: &str = "--login-item";

/// `clod-helper --login-item`: открыть приложение в трее через LaunchServices
/// и выйти. None — это не наш режим.
#[cfg(target_os = "macos")]
pub fn login_item_cli(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some(LOGIN_ITEM_FLAG) {
        return None;
    }
    let bundle = std::env::current_exe().ok().and_then(|exe| app_bundle_of(&exe));
    let Some(bundle) = bundle else {
        return Some(1);
    };
    // -g: не выводить на передний план; -j: запустить скрытым.
    let ok = std::process::Command::new("/usr/bin/open")
        .args(["-g", "-j"])
        .arg(&bundle)
        .args(["--args", "--tray"])
        .status()
        .is_ok_and(|s| s.success());
    Some(if ok { 0 } else { 1 })
}

/// `…/Clod Helper.app/Contents/MacOS/clod-helper` → `…/Clod Helper.app`.
#[cfg(target_os = "macos")]
fn app_bundle_of(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(std::path::Path::to_path_buf)
}

#[cfg(target_os = "windows")]
pub fn get(app: &AppHandle) -> Result<AutostartState, String> {
    use tauri_plugin_autostart::ManagerExt;
    let enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    Ok(AutostartState { enabled, needs_approval: false })
}

#[cfg(target_os = "windows")]
pub fn set(app: &AppHandle, enabled: bool) -> Result<AutostartState, String> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    get(app)
}

#[cfg(target_os = "windows")]
pub fn migrate_legacy() {}

#[cfg(target_os = "macos")]
pub use mac::{get, migrate_legacy, set};

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2_foundation::NSString;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};
    use tauri::AppHandle;

    use super::{AutostartState, LOGIN_ITEM_FLAG};
    use crate::macos;

    pub const LABEL: &str = "ru.iimperium.clodhelper.autostart";
    /// Имя plist'а агента в Contents/Library/LaunchAgents (tauri.macos.conf.json).
    const PLIST_NAME: &str = "ru.iimperium.clodhelper.autostart.plist";
    /// Что писал tauri-plugin-autostart (метка = productName).
    const LEGACY_PLIST: &str = "Clod Helper.plist";

    fn launch_agent(name: &str) -> Option<PathBuf> {
        macos::launch_agents_dir().map(|d| d.join(name))
    }

    /// Агент SMAppService; None — macOS < 13 (класса ещё нет).
    fn service() -> Option<Retained<SMAppService>> {
        AnyClass::get(c"SMAppService")?;
        // SAFETY: класс есть (проверено выше); имя — валидный NSString.
        Some(unsafe { SMAppService::agentServiceWithPlistName(&NSString::from_str(PLIST_NAME)) })
    }

    fn status(svc: &SMAppService) -> SMAppServiceStatus {
        // SAFETY: обычный геттер свойства.
        unsafe { svc.status() }
    }

    /// Свой LaunchAgent для macOS 11–12 (и запасной путь, если SMAppService
    /// отказал): та же команда `--login-item`, грузится при следующем входе.
    fn write_fallback_agent() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let path = launch_agent(&format!("{LABEL}.plist"))
            .ok_or_else(|| "Не нашёл домашнюю папку".to_string())?;
        macos::write_agent_plist(
            &path,
            &macos::Agent {
                label: LABEL,
                program_arguments: &[exe.to_string_lossy().to_string(), LOGIN_ITEM_FLAG.into()],
                keep_alive: false,
                stderr_log: None,
            },
        )
    }

    fn remove_file(name: &str) {
        if let Some(p) = launch_agent(name).filter(|p| p.exists()) {
            let _ = std::fs::remove_file(p);
        }
    }

    pub fn get(_app: &AppHandle) -> Result<AutostartState, String> {
        let file_enabled = [format!("{LABEL}.plist"), LEGACY_PLIST.to_string()]
            .iter()
            .any(|n| launch_agent(n).is_some_and(|p| p.exists()));
        let st = service().map(|s| status(&s));
        Ok(AutostartState {
            enabled: file_enabled
                || matches!(st, Some(SMAppServiceStatus::Enabled | SMAppServiceStatus::RequiresApproval)),
            needs_approval: st == Some(SMAppServiceStatus::RequiresApproval),
        })
    }

    pub fn set(app: &AppHandle, enabled: bool) -> Result<AutostartState, String> {
        if enabled {
            enable()?;
        } else {
            disable()?;
        }
        get(app)
    }

    fn enable() -> Result<(), String> {
        let registered = match service() {
            Some(svc) => {
                // SAFETY: обычный вызов API; ошибка возвращается как NSError.
                match unsafe { svc.registerAndReturnError() } {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!("SMAppService.register: {}", e.localizedDescription());
                        false
                    }
                }
            }
            None => false,
        };
        if registered {
            remove_file(&format!("{LABEL}.plist"));
        } else {
            write_fallback_agent()?;
        }
        // Плагин больше не нужен. bootout НЕ делаем: если текущий процесс
        // запущен этим агентом при входе, launchd убил бы его вместе с задачей.
        remove_file(LEGACY_PLIST);
        Ok(())
    }

    fn disable() -> Result<(), String> {
        if let Some(svc) = service() {
            if status(&svc) != SMAppServiceStatus::NotRegistered {
                // SAFETY: обычный вызов API; ошибка возвращается как NSError.
                unsafe { svc.unregisterAndReturnError() }
                    .map_err(|e| format!("Не удалось выключить автозапуск: {}", e.localizedDescription()))?;
            }
        }
        remove_file(&format!("{LABEL}.plist"));
        remove_file(LEGACY_PLIST);
        Ok(())
    }

    /// Разово при старте: автозапуск плагина («Clod Helper.plist») →
    /// SMAppService. Если перенести не удалось, старый файл остаётся рабочим.
    pub fn migrate_legacy() {
        if !launch_agent(LEGACY_PLIST).is_some_and(|p| p.exists()) {
            return;
        }
        if let Err(e) = enable() {
            tracing::warn!("миграция автозапуска не удалась: {e}");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::app_bundle_of;
        use std::path::Path;

        #[test]
        fn bundle_is_found_from_executable() {
            assert_eq!(
                app_bundle_of(Path::new("/Applications/Clod Helper.app/Contents/MacOS/clod-helper")),
                Some(Path::new("/Applications/Clod Helper.app").to_path_buf())
            );
            assert_eq!(app_bundle_of(Path::new("/usr/local/bin/clod-helper")), None);
        }

        #[test]
        fn bundled_agent_plist_is_valid_and_matches() {
            let plist = concat!(env!("CARGO_MANIFEST_DIR"), "/macos/ru.iimperium.clodhelper.autostart.plist");
            let text = std::fs::read_to_string(plist).unwrap();
            assert!(text.contains(&format!("<string>{}</string>", super::LABEL)));
            assert!(text.contains("<string>Contents/MacOS/clod-helper</string>"));
            assert!(text.contains(&format!("<string>{}</string>", super::super::LOGIN_ITEM_FLAG)));
            assert!(!text.contains("<key>KeepAlive</key>"), "агент только запускает GUI и выходит");
            assert!(std::process::Command::new("/usr/bin/plutil")
                .args(["-lint", "-s", plist])
                .status()
                .unwrap()
                .success());
            let conf = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.macos.conf.json")).unwrap();
            assert!(conf.contains(&format!("Library/LaunchAgents/{}", super::PLIST_NAME)));
        }
    }
}
