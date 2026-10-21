//! Профили приложений: список, запуск, ярлыки, статус по соединениям.

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

// ─── App profiles (проксируемые приложения) ─────────────────────

/// Список профилей + их runtime-статус (найдено ли приложение, есть ли ярлык).
/// Внутри Get-AppxPackage (секунды!) — только spawn_blocking.
#[tauri::command]
pub async fn apps_list() -> Result<Vec<AppProfileStatus>, String> {
    tokio::task::spawn_blocking(|| {
        let s = settings::load_with_presets();
        apps::statuses(&s.app_profiles)
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Иконки программ (data-URL PNG) для подсказок и строк своих приложений.
#[tauri::command]
pub async fn apps_icons(paths: Vec<String>) -> Result<std::collections::HashMap<String, String>, String> {
    tokio::task::spawn_blocking(move || apps::icons(&paths))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Поиск установленных программ по названию (Spotlight / меню «Пуск»).
#[tauri::command]
pub async fn apps_search(query: String) -> Result<Vec<apps::FoundApp>, String> {
    tokio::task::spawn_blocking(move || {
        // Убранные из списка (встроенные прячутся) — тоже находятся.
        let hidden: Vec<(String, String)> = settings::load_with_presets()
            .app_profiles
            .into_iter()
            .filter(|p| p.hidden)
            .filter_map(|p| Some((apps::profile_file(&p)?, p)))
            .map(|(path, p)| (p.name, path))
            .collect();
        apps::search_with_hidden(&query, &hidden)
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Настройки приложения: свой прокси (None — «Авто») и «весь трафик».
/// Для Claude / Codex / Telegram прокси задаётся назначением сервиса (UI
/// вызывает proxy_assignments_set) — здесь только профиль.
#[tauri::command]
pub async fn apps_settings_set(
    id: String,
    proxy: Option<usize>,
    full_proxy: Option<bool>,
) -> Result<Vec<AppProfileStatus>, String> {
    let mut s = settings::load_with_presets();
    let p = s
        .app_profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| "Профиль не найден".to_string())?;
    p.proxy = proxy;
    p.full_proxy = full_proxy;
    let saved = settings::set_profiles(s.app_profiles)?;
    Ok(apps::statuses(&saved.app_profiles))
}

/// Раскладка «Приложений» (вид, папки, порядок) — JSON от UI.
#[tauri::command]
pub async fn apps_layout_get() -> Result<Option<serde_json::Value>, String> {
    Ok(settings::load().apps_layout)
}

#[tauri::command]
pub async fn apps_layout_set(layout: serde_json::Value) -> Result<(), String> {
    let mut s = settings::load();
    s.apps_layout = Some(layout);
    settings::save(&s)
}

/// Открыть в браузере страницу расширения, перехватившего прокси.
#[tauri::command]
pub async fn apps_open_extension(id: String, ext_id: String) -> Result<(), String> {
    let profile = settings::load_with_presets()
        .app_profiles
        .into_iter()
        .find(|p| p.id == id && p.exe_path.is_some())
        .ok_or_else(|| "Профиль не найден".to_string())?;
    let path = profile.exe_path.clone().unwrap_or_default();
    #[cfg(not(target_os = "windows"))]
    return tokio::task::spawn_blocking(move || crate::browser_ext::open_extension_page(&path, &ext_id))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
    // Windows: адрес chrome://extensions передаётся exe браузера — так же,
    // как «Открыть», через мост (запущенный браузер просто откроет вкладку).
    #[cfg(target_os = "windows")]
    return tokio::task::spawn_blocking(move || {
        let url = crate::browser_ext::extension_page_url(&path, &ext_id)?;
        apps::open_in_browser(&profile, &url)
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
}

/// Через мост ли идёт каждое приложение — по живым соединениям.
#[tauri::command]
pub async fn apps_traffic() -> Result<Vec<crate::traffic::AppTraffic>, String> {
    tokio::task::spawn_blocking(|| {
        let s = settings::load_with_presets();
        crate::traffic::snapshot(&s.app_profiles)
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Сохранить новое состояние профилей (enabled-галочки, добавленные свои).
/// Если killswitch активен — правила файрвола НЕ пересобираются автоматически
/// здесь (это делает отдельный killswitch_enable), чтобы не плодить UAC-промпты
/// на каждый чих. UI подскажет «переприменить killswitch» если нужно.
/// settings.json VS Code больше не трогаем — терминал покрывает глобальный env.
#[tauri::command]
pub async fn apps_set(profiles: Vec<AppProfile>) -> Result<Vec<AppProfileStatus>, String> {
    let saved = settings::set_profiles(profiles)?;
    Ok(apps::statuses(&saved.app_profiles))
}

/// Добавить кастомный профиль (пользователь указал exe).
#[tauri::command]
pub async fn apps_add_custom(
    name: String,
    exe_path: String,
) -> Result<Vec<AppProfileStatus>, String> {
    if !std::path::Path::new(&exe_path).exists() {
        return Err("Файл не найден по указанному пути".into());
    }
    let mut s = settings::load_with_presets();
    // Программа уже есть, но убрана из списка (в т.ч. встроенная) — вернуть.
    // Путь сверяем с тем, где она стоит сейчас: поиск отдаёт путь из «Пуска»
    // или реестра, у встроенной он мог быть сохранён иначе или не сохранён.
    if let Some(p) = s.app_profiles.iter_mut().find(|p| {
        p.hidden
            && (p.exe_path.as_deref().is_some_and(|e| apps::same_path(e, &exe_path))
                || apps::profile_file(p).is_some_and(|f| apps::same_path(&f, &exe_path)))
    }) {
        p.hidden = false;
        let saved = settings::set_profiles(s.app_profiles)?;
        return Ok(apps::statuses(&saved.app_profiles));
    }
    // id = custom-<N>
    let n = s
        .app_profiles
        .iter()
        .filter(|p| p.id.starts_with("custom-"))
        .count()
        + 1;
    let leaf = std::path::Path::new(&exe_path)
        .file_name()
        .map(|x| x.to_string_lossy().to_string())
        .unwrap_or_else(|| "app.exe".into());
    s.app_profiles.push(AppProfile {
        id: format!("custom-{n}"),
        name,
        kind: apps::LaunchKind::Custom,
        exe_path: Some(exe_path),
        app_id: None,
        process_names: vec![leaf],
        enabled: true,
        builtin: false,
        hidden: false,
        proxy: None,
        full_proxy: None,
    });
    let saved = settings::set_profiles(s.app_profiles)?;
    Ok(apps::statuses(&saved.app_profiles))
}

/// Убрать приложение из списка (встроенные прячутся, свои удаляются); у
/// браузера заодно снимается политика «Всегда через мост».
#[tauri::command]
pub async fn apps_remove(id: String) -> Result<Vec<AppProfileStatus>, String> {
    // Браузер с политикой «Всегда через мост»: убрать из списка = снять и её,
    // иначе браузер продолжает ходить через мост при любом запуске (политика
    // главнее флагов). Отмена пароля — не убираем, чтобы не было рассинхрона.
    if let Ok(domain) = browser_domain_of(&id) {
        let configured = tokio::task::spawn_blocking(move || crate::browser_policy::status(domain))
            .await
            .map_err(|e| format!("Внутренняя ошибка: {e}"))?
            .is_ok_and(|st| st.configured);
        if configured {
            tokio::task::spawn_blocking(move || crate::browser_policy::set(domain, None))
                .await
                .map_err(|e| format!("Внутренняя ошибка: {e}"))?
                .map_err(|e| format!("Не удалось снять «Всегда через мост»: {e}"))?;
        }
    }
    let mut s = settings::load_with_presets();
    // Встроенный не удаляем (пресеты пересеялись бы при запуске) — прячем.
    if let Some(p) = s.app_profiles.iter_mut().find(|p| p.id == id && p.builtin) {
        p.hidden = true;
    } else {
        s.app_profiles.retain(|p| p.id != id);
    }
    let saved = settings::set_profiles(s.app_profiles)?;
    Ok(apps::statuses(&saved.app_profiles))
}

/// Запустить приложение через мост.
#[tauri::command]
pub async fn apps_launch(id: String) -> Result<(), String> {
    let s = settings::load_with_presets();
    let profile = s
        .app_profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "Профиль не найден".to_string())?
        .clone();
    tokio::task::spawn_blocking(move || apps::launch(&profile))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Закрыть приложение, открытое мимо моста, и запустить через мост.
#[tauri::command]
pub async fn apps_relaunch(id: String) -> Result<(), String> {
    let profile = settings::load_with_presets()
        .app_profiles
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "Профиль не найден".to_string())?;
    tokio::task::spawn_blocking(move || apps::relaunch(&profile))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Создать ярлык «<name> (proxy)» на рабочем столе или в панели задач.
#[tauri::command]
pub async fn apps_create_shortcut(
    id: String,
    target: ShortcutTarget,
) -> Result<String, String> {
    let s = settings::load_with_presets();
    let profile = s
        .app_profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "Профиль не найден".to_string())?
        .clone();
    tokio::task::spawn_blocking(move || apps::create_shortcut(&profile, target))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}


// ─── Приватность и политика браузера ────────────────────────────

/// Проверка приватности: IP выхода и настоящий, DNS, кто идёт мимо моста.
#[tauri::command]
pub async fn privacy_check() -> Result<crate::privacy::PrivacyReport, String> {
    let leaking = tokio::task::spawn_blocking(|| {
        let s = settings::load_with_presets();
        let traffic = crate::traffic::snapshot(&s.app_profiles);
        s.app_profiles
            .iter()
            .filter(|p| !p.hidden)
            .filter(|p| {
                traffic.iter().any(|t| {
                    t.id == p.id && (t.proxy_extension.is_some() || t.direct > 0)
                })
            })
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
    Ok(crate::privacy::check_within(leaking, std::time::Duration::from_secs(25)).await)
}

fn browser_domain_of(id: &str) -> Result<&'static str, String> {
    let path = settings::load_with_presets()
        .app_profiles
        .into_iter()
        .find(|p| p.id == id)
        .and_then(|p| p.exe_path)
        .ok_or_else(|| "Профиль не найден".to_string())?;
    crate::browser_policy::policy_domain(&path)
        .ok_or_else(|| "Этот браузер не поддерживает закрепление политикой".to_string())
}

/// Состояние «всегда через мост» у браузера; None — браузер без политик.
#[tauri::command]
pub async fn browser_policy_status(id: String) -> Result<Option<crate::browser_policy::BrowserPolicyStatus>, String> {
    let Ok(domain) = browser_domain_of(&id) else { return Ok(None) };
    tokio::task::spawn_blocking(move || crate::browser_policy::status(domain).map(Some))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Включить/снять «всегда через мост» (пароль Mac / окно UAC). Вход — как у «Открыть».
#[tauri::command]
pub async fn browser_policy_set(id: String, enable: bool) -> Result<crate::browser_policy::BrowserPolicyStatus, String> {
    let domain = browser_domain_of(&id)?;
    let server = if enable {
        let profile = settings::load_with_presets()
            .app_profiles
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| "Профиль не найден".to_string())?;
        Some(apps::launch_proxy_url(&profile).trim_start_matches("http://").to_string())
    } else {
        None
    };
    tokio::task::spawn_blocking(move || crate::browser_policy::set(domain, server.as_deref()))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}
