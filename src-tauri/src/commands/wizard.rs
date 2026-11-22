//! Мастер установки одним вызовом.

#![allow(unused_imports)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::apps::{self, AppProfile, AppProfileStatus, ShortcutTarget};
use crate::env_proxy::{self, GlobalProxyEnv};
use crate::firewall::{self, KillswitchStatus};
use crate::proxy::{self, ProxyCheckResult, ProxyConfig};
use crate::settings::{self, Settings};
use crate::shim::{self, ShimStatus, ShimTestResult};
use crate::system::{self, CursorInfo, NodeInfo, PythonInfo};

use super::*;

// ─── Новый мастер (3 шага): установка одним вызовом ──────────────

/// Результат установки нового мастера.
#[derive(Debug, Serialize)]
pub struct WizardInstallResult {
    /// Реальная проверка соединения через мост (не «зелёный pill по флагу»).
    pub test: shim::ShimTestResult,
    /// Включилось ли системное проксирование (env). false - не фатально,
    /// режим включается в «Приложениях» → Claude Desktop → ⚙.
    pub env_on: bool,
    /// Claude Desktop: "ok" - подключён к мосту, "err" - не вышло (отказ в
    /// окне системы, чужая политика), "absent" - не установлен.
    pub claude: &'static str,
    /// Claude Desktop был открыт во время подключения: настройка прокси
    /// подхватится только после перезапуска.
    pub claude_running: bool,
}

/// Какие приложения уже стоят: мастер предлагает поставить недостающие и
/// опрашивает это раз в несколько секунд, пока идёт установка.
#[derive(Debug, Serialize)]
pub struct WizardApps {
    pub claude: bool,
    pub codex: bool,
}

/// Свежая проверка без кэша: человек только что поставил приложение.
#[tauri::command]
pub async fn wizard_apps() -> WizardApps {
    tokio::task::spawn_blocking(|| WizardApps {
        claude: crate::claude_desktop::detect_now().is_some(),
        codex: apps::codex_installed(),
    })
    .await
    .unwrap_or(WizardApps { claude: false, codex: false })
}

/// Где поставить приложение. Windows - страница в Microsoft Store (ID
/// сверены с каталогом Store: пакеты AnthropicPBC.Claude и OpenAI.Codex,
/// их же ищет детект), macOS - страница загрузки на сайте разработчика.
fn store_url(id: &str) -> Option<&'static str> {
    #[cfg(target_os = "windows")]
    let url = match id {
        "claude" => "ms-windows-store://pdp/?ProductId=9P6K58THS811",
        "codex" => "ms-windows-store://pdp/?ProductId=9PLM9XGG6VKS",
        _ => return None,
    };
    #[cfg(not(target_os = "windows"))]
    let url = match id {
        "claude" => "https://claude.com/download",
        "codex" => "https://openai.com/codex/",
        _ => return None,
    };
    Some(url)
}

/// Открыть страницу установки приложения. Адреса - только свои, из кода:
/// схема ms-windows-store открывается отсюда, а не из интерфейса.
#[tauri::command]
pub fn wizard_open_store(app: String) -> Result<(), String> {
    let url = store_url(&app).ok_or("Неизвестное приложение")?;
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| format!("Не удалось открыть магазин: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_urls_are_known_and_fixed() {
        for id in ["claude", "codex"] {
            let url = store_url(id).unwrap();
            if cfg!(target_os = "windows") {
                assert!(url.starts_with("ms-windows-store://pdp/?ProductId=9"));
            } else {
                assert!(url.starts_with("https://"));
            }
        }
        assert!(store_url("https://evil.example").is_none());
    }
}

/// Подключить Claude Desktop к мосту. Уже подключён - ничего не спрашиваем.
/// Windows спросит разрешение администратора, macOS - пароль, один раз.
pub(crate) async fn connect_claude(app: &AppHandle) -> bool {
    let configured = tokio::task::spawn_blocking(crate::claude_policy::status)
        .await
        .ok()
        .and_then(|r| r.ok())
        .is_some_and(|s| s.configured);
    configured || claude_proxy_enable(app.clone()).await.is_ok()
}

/// Установка для нового мастера: мост из ресурсов → настройки + задача
/// автозапуска + старт → системное проксирование → пресеты приложений →
/// честная проверка соединения. Прогресс - события "wizard:step"
/// ("<фаза>:start|ok|err"). Легаси PS-скрипты (python-мост, npm, режимы
/// установки) в этом пути не участвуют - всё делают существующие Rust-пути.
#[tauri::command]
pub async fn wizard_install(
    app: AppHandle,
    proxy_url: String,
) -> Result<WizardInstallResult, String> {
    let step = |phase: &str, state: &str| {
        let _ = app.emit("wizard:step", format!("{phase}:{state}"));
    };
    // Пока идёт установка мастера, тихая установка обновления ждёт.
    let _busy = crate::auto_update::busy_guard();
    proxy::parse(&proxy_url)?;

    // 1) Мост: копия из ресурсов в %LOCALAPPDATA%\ClaudeDesktopProxy.
    step("bridge", "start");
    let src = resolve_resource(&app, shim::BRIDGE_EXE)?;
    let dst = shim::bridge_exe_path()
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
    let copied = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let _ = shim::stop(); // переустановка поверх живого моста: exe залочен
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }
        let mut last_err = String::new();
        for _ in 0..3 {
            match shim::install_bridge_binary(&src, &dst) {
                Ok(_) => {
                    last_err.clear();
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
            }
        }
        if last_err.is_empty() {
            Ok(())
        } else {
            Err(format!("Не удалось установить мост: {last_err}"))
        }
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
    if let Err(e) = copied {
        step("bridge", "err");
        return Err(e);
    }
    step("bridge", "ok");

    // 2) Настройки + задача автозапуска + запуск моста.
    step("task", "start");
    let url = proxy_url.clone();
    let started = tokio::task::spawn_blocking(move || -> Result<(), String> {
        settings::set_proxy_url(url.clone())?;
        crate::tasks::register(&url)?;
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
    if let Err(e) = started {
        step("task", "err");
        return Err(e);
    }
    step("task", "ok");

    // 3) Системное проксирование - главный выключатель новой модели.
    //    Неудача не фатальна: режим есть в настройках Claude Desktop.
    step("env", "start");
    let env_on = tokio::task::spawn_blocking(|| {
        env_proxy::set(&format!("http://127.0.0.1:{}", shim::LISTEN_PORT)).is_ok()
    })
    .await
    .unwrap_or(false);
    step("env", if env_on { "ok" } else { "err" });

    // 4) Пресеты профилей приложений - чтобы раздел «Приложения» был готов.
    step("presets", "start");
    let _ = tokio::task::spawn_blocking(settings::load_with_presets).await;
    step("presets", "ok");

    // 5) Claude Desktop: штатный прокси Claude - одна системная просьба
    //    (Windows - окно разрешения, macOS - пароль) прямо в мастере, а не
    //    поиск кнопки потом. Отмена или конфликт не фатальны: в «Приложениях»
    //    будет «Подключить». Нет Claude Desktop - следующий шаг мастера
    //    предложит его поставить.
    let has_claude = tokio::task::spawn_blocking(|| crate::claude_desktop::detect_now().is_some())
        .await
        .unwrap_or(false);
    let (claude, claude_running) = if has_claude {
        step("claude", "start");
        let ok = connect_claude(&app).await;
        step("claude", if ok { "ok" } else { "err" });
        let running = tokio::task::spawn_blocking(crate::claude_desktop::running).await.unwrap_or(false);
        (if ok { "ok" } else { "err" }, running)
    } else {
        step("claude", "skip");
        ("absent", false)
    };

    // Telegram мастер не трогает: подключается по выбору пользователя кнопкой
    // в «Приложениях». Раньше мастер сам включал Telegram защиту от утечек
    // (закрывая прямой выход) - и тот оставался без связи, если человек не
    // нажал «Подключить» в окне Telegram или прокси ему был не нужен.

    // 6) Честная проверка: реальный запрос через мост, не зелёный pill.
    step("test", "start");
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let test = shim::test().await;
    step("test", if test.ok { "ok" } else { "err" });

    Ok(WizardInstallResult { test, env_on, claude, claude_running })
}

/// Подключение Claude Desktop из шага «Приложения».
#[derive(Debug, Serialize)]
pub struct WizardClaudeConnect {
    /// false - отказ в окне системы или ошибка (можно позже в «Приложениях»).
    pub ok: bool,
    /// Открыт (магазин часто сам запускает приложение после установки):
    /// прокси подхватится после перезапуска.
    pub running: bool,
}

/// Мастер: Claude Desktop только что поставили - подключить к мосту.
/// Err - Claude Desktop так и не нашёлся.
#[tauri::command]
pub async fn wizard_connect_claude(app: AppHandle) -> Result<WizardClaudeConnect, String> {
    let found = tokio::task::spawn_blocking(|| crate::claude_desktop::detect_now().is_some())
        .await
        .unwrap_or(false);
    if !found {
        return Err("Claude Desktop не найден".into());
    }
    let ok = connect_claude(&app).await;
    let running = tokio::task::spawn_blocking(crate::claude_desktop::running).await.unwrap_or(false);
    Ok(WizardClaudeConnect { ok, running })
}
