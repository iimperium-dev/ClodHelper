//! Автозапуск моста, здоровье, маршрутизация, обновление.

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

// ─── Автозапуск моста (Планировщик задач) ───────────────────────

/// Статус задачи автозапуска моста.
#[tauri::command]
pub async fn bridge_task_status() -> crate::tasks::BridgeTaskStatus {
    tokio::task::spawn_blocking(crate::tasks::status)
        .await
        .unwrap_or(crate::tasks::BridgeTaskStatus {
            registered: false,
            state: None,
        })
}

/// Зарегистрировать задачу автозапуска (использует proxy_url из settings).
/// Удаляет легаси .vbs из Startup после успешной регистрации.
#[tauri::command]
pub async fn bridge_task_register() -> Result<crate::tasks::BridgeTaskStatus, String> {
    let url = settings::load()
        .proxy_url
        .ok_or_else(|| "Прокси ещё не настроен — сначала сохраните его".to_string())?;
    tokio::task::spawn_blocking(move || -> Result<crate::tasks::BridgeTaskStatus, String> {
        crate::tasks::register(&url)?;
        // Повторная настройка должна сразу восстановить мост, а не только
        // записать задачу автозапуска до следующего входа в Windows.
        shim::start()?;
        Ok(crate::tasks::status())
    })
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

// ─── Мост: здоровье и маршрутизация ─────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeUpstreamHealth {
    pub url: String,
    pub healthy: bool,
    pub ok: u64,
    pub fail: u64,
    /// Отказов подряд (мост 1.5.0+; у старых — 0).
    #[serde(default)]
    pub fail_streak: u64,
}

/// Одна ошибка из кольцевого буфера моста: секунды-от-старта + текст.
#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeErrorEntry {
    pub t: u64,
    pub msg: String,
}

/// Зеркало JSON от GET /healthz нового Rust-моста.
#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeHealth {
    pub status: String,
    pub version: String,
    pub uptime_sec: u64,
    pub listen: String,
    pub mode: String,
    pub active: u64,
    pub total: u64,
    pub via_upstream: u64,
    pub via_direct: u64,
    pub errors: u64,
    pub upstreams: Vec<BridgeUpstreamHealth>,
    /// Последние ошибки моста (мост 1.1.0+; у старых версий поля нет).
    #[serde(default)]
    pub last_errors: Vec<BridgeErrorEntry>,
}

/// Установленный мост — старый python-exe (без /healthz и --rules)?
/// Проба `--version` кэшируется по mtime exe. По этому флагу UI показывает
/// «Обновить мост» — надёжнее, чем гадать по тексту ошибки healthz (старый
/// мост может и таймаутить запрос, и вернуть мусор — зависит от upstream).
#[tauri::command]
pub async fn bridge_exe_legacy() -> Result<bool, String> {
    tokio::task::spawn_blocking(crate::tasks::bridge_exe_is_legacy)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Версии моста: установленного и встроенного в приложение. Если встроенный
/// новее — UI предлагает «Обновить мост» (иначе после апдейта Clod Helper
/// пользователь навсегда остался бы на старом мосте: exe копируется только
/// мастером установки).
#[derive(Debug, Serialize)]
pub struct BridgeVersions {
    pub installed: Option<String>,
    pub bundled: Option<String>,
    pub outdated: bool,
}

#[tauri::command]
pub async fn bridge_versions(app: AppHandle) -> Result<BridgeVersions, String> {
    let bundled_path = resolve_resource(&app, shim::BRIDGE_EXE).ok();
    tokio::task::spawn_blocking(move || {
        let installed = shim::bridge_exe_path()
            .filter(|p| p.exists())
            .and_then(|p| crate::tasks::exe_version(&p));
        let bundled = bundled_path
            .filter(|p| p.exists())
            .and_then(|p| crate::tasks::exe_version(&p));
        let outdated = match (
            installed.as_deref().and_then(crate::tasks::parse_version),
            bundled.as_deref().and_then(crate::tasks::parse_version),
        ) {
            (Some(i), Some(b)) => b > i,
            _ => false,
        };
        BridgeVersions { installed, bundled, outdated }
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}


// ─── Обновления ─────────────────────────────────────────────────

/// Есть ли новая версия Clod Helper (последний релиз GitHub).
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<crate::updater::UpdateInfo>, String> {
    // Ручная проверка раскатку не смотрит (но уважает остановку выпуска).
    let found = crate::updater::check(crate::updater::Mode::Manual).await?;
    crate::update_notify::remember(&found);
    // Колокольчик и трей узнают об обновлении и от ручной проверки.
    let _ = app.emit("notices:changed", crate::notices::unread());
    Ok(found)
}

/// Последнее найденное обновление (без запроса к GitHub) — для окна-уведомления.
#[tauri::command]
pub fn update_latest() -> Option<crate::updater::UpdateInfo> {
    crate::update_notify::latest()
}

/// Скачать, проверить SHA-256, поставить и перезапуститься. Ход —
/// событиями `update:progress` (окно обновления рисует по ним прогресс).
#[tauri::command]
pub async fn update_install(app: AppHandle, info: crate::updater::UpdateInfo) -> Result<(), String> {
    install_update(app, info, "manual").await
}

/// Установка обновления; `how` ("manual" | "auto") уходит в статистику.
/// Тихий путь автообновления передаст "auto".
pub(crate) async fn install_update(app: AppHandle, info: crate::updater::UpdateInfo, how: &str) -> Result<(), String> {
    // Одна установка за раз (ручная или тихая): вторая получает ошибку и не
    // трогает ни статистику, ни отметки первой.
    if crate::updater::is_installing() {
        return Err(crate::updater::ALREADY.into());
    }
    let emitter = app.clone();
    let progress = move |p: crate::updater::UpdateProgress| {
        let _ = emitter.emit("update:progress", p);
    };
    // Установщик может стартовать и тут же закрыть программу: как запущено -
    // записываем заранее, при ошибке снимаем.
    crate::telemetry::set_pending_how(Some(how));
    // Тихая установка перед запуском установщика ещё раз смотрит, что окно
    // скрыто, а человек не за компьютером.
    let watcher = app.clone();
    let silent = how == "auto";
    let ready = move || !silent || crate::auto_update::still_quiet(&watcher);
    if let Err(e) = crate::updater::install(&info, &progress, how, &ready).await {
        if e == crate::updater::ALREADY {
            return Err(e);
        }
        crate::telemetry::set_pending_how(None);
        if e == crate::updater::POSTPONED {
            return Err(e);
        }
        // Тихая установка шлёт отчёт только о сумме и запуске установщика и
        // раз на версию: сетевые ошибки и откладывание в отчёты не идут.
        if silent {
            crate::auto_update::report_auto_error(&info.version, &e);
        } else {
            crate::telemetry::report_update(&e);
        }
        return Err(e);
    }
    crate::update_notify::close_toast(&app);
    // Тихая установка идёт без окон: ни «Перезапускаю», ни пауз.
    if how != "auto" {
        progress(crate::updater::UpdateProgress { stage: "restart", received: 0, total: 0 });
        // Пауза, чтобы «Перезапускаю…» успело показаться: иначе окно исчезает
        // посреди «Устанавливаю» и это выглядит как вылет.
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    }
    // Новую версию поставит и запустит фоновый скрипт (macOS) или
    // установщик (Windows), когда мы выйдем.
    app.exit(0);
    Ok(())
}

/// Клик по уведомлению в углу экрана (обновление или рассылка).
#[tauri::command]
pub fn update_toast_open(app: AppHandle, id: Option<String>) {
    let id = id
        .or_else(|| crate::update_notify::latest().map(|u| crate::notices::update_id(&u.version)))
        .unwrap_or_default();
    crate::update_notify::open_from_toast(&app, &id);
}

/// Открыли окно или трей-попап — проверить рассылки (не чаще раза в минуту).
#[tauri::command]
pub async fn notices_refresh(app: AppHandle) {
    crate::update_notify::refresh_notices(&app, false).await;
    crate::update_notify::check_update(&app, false).await;
}

/// Колокольчик: обновление и рассылки, с отметкой «прочитано».
#[tauri::command]
pub async fn notices_list() -> Result<Vec<crate::notices::NoticeItem>, String> {
    tokio::task::spawn_blocking(crate::notices::list)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Отметить прочитанными (пустой список — все).
#[tauri::command]
pub async fn notices_mark_read(app: AppHandle, ids: Vec<String>) -> Result<Vec<crate::notices::NoticeItem>, String> {
    tokio::task::spawn_blocking(move || -> Result<Vec<crate::notices::NoticeItem>, String> {
        let ids = if ids.is_empty() {
            crate::notices::list().into_iter().map(|i| i.id).collect()
        } else {
            ids
        };
        crate::settings::mark_notices_read(&ids)?;
        let _ = app.emit("notices:changed", crate::notices::unread());
        Ok(crate::notices::list())
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub fn update_toast_close(app: AppHandle) {
    crate::update_notify::close_toast(&app);
}

/// Версия для окна «Что нового», если приложение только что обновилось.
#[tauri::command]
pub fn whats_new_take() -> Result<Option<String>, String> {
    settings::take_whats_new(env!("CARGO_PKG_VERSION"))
}

/// Автообновление: включено ли и надо ли предложить его включить.
#[tauri::command]
pub fn auto_update_get() -> settings::AutoUpdateState {
    settings::auto_update_state(&settings::load())
}

/// Включить/выключить автообновление. Ответ на предложение — тоже сюда.
#[tauri::command]
pub fn auto_update_set(enabled: bool) -> Result<settings::AutoUpdateState, String> {
    Ok(settings::auto_update_state(&settings::set_auto_update(enabled)?))
}

/// Приватность: автообновление, отчёты о сбоях, статистика; offer - предложить
/// автообновление (окно после «Что нового»).
#[tauri::command]
pub fn privacy_get() -> settings::PrivacyState {
    settings::privacy_state(&settings::load())
}

/// Переключатель приватности: key = "auto_update" | "crash_reports" | "usage_stats".
#[tauri::command]
pub fn privacy_set(key: String, on: bool) -> Result<settings::PrivacyState, String> {
    let s = match key.as_str() {
        "auto_update" => settings::set_auto_update(on)?,
        "crash_reports" => settings::set_crash_reports(on)?,
        "usage_stats" => settings::set_usage_stats(on)?,
        _ => return Err("Неизвестная настройка".into()),
    };
    Ok(settings::privacy_state(&s))
}

/// Человек увидел переключатели или строку про отчёты: с этого момента
/// отчёты и статистика могут уходить (если включены).
#[tauri::command]
pub async fn privacy_notice_seen() -> Result<(), String> {
    tokio::task::spawn_blocking(settings::set_privacy_notice_seen)
        .await
        .map_err(|e| e.to_string())?
        .map(|_| ())
}

/// Тело ручного отчёта так, как оно уйдёт на сервер (очищенное, JSON с отступами).
#[tauri::command]
pub async fn telemetry_report_preview(text: String, contact: Option<String>, nonce: Option<String>) -> String {
    crate::telemetry::report_preview(text, contact, nonce).await
}

/// Ошибка интерфейса (window.onerror и т.п.): уходит в отчёты о сбоях, если
/// они включены. Не больше 5 за запуск.
#[tauri::command]
pub async fn telemetry_ui_error(msg: String, stack: String) {
    // Читает настройки с диска - не в потоке async-рантайма.
    let _ = tokio::task::spawn_blocking(move || crate::telemetry::report_ui(&msg, &stack)).await;
}

/// Событие уведомления для статистики рассылок: shown | clicked | dismissed.
/// Уходит только при включённой статистике, один раз на событие.
#[tauri::command]
pub fn notice_event(id: String, ev: String) {
    crate::telemetry::report_notice_event(&id, &ev);
}

/// Отчёт, который человек отправил сам. Шлётся всегда.
#[tauri::command]
pub async fn telemetry_report(text: String, contact: Option<String>, nonce: Option<String>) -> Result<(), String> {
    crate::telemetry::send_report(text, contact, nonce).await
}

/// Фирменный звук уведомления.
#[tauri::command]
pub fn play_chime() {
    crate::sound::chime();
}

/// «Отчёт для поддержки»: состояние программы текстом, без паролей.
#[tauri::command]
pub async fn diagnostics_report() -> Result<String, String> {
    tokio::task::spawn_blocking(crate::diagnostics::report)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Проблема с файлом моста (удалён антивирусом и т.п.); None — всё в порядке.
#[tauri::command]
pub fn bridge_file_issue() -> Option<crate::bridge_file::Issue> {
    crate::bridge_file::issue()
}

/// «Понятно» на сообщении «мост восстановлен».
#[tauri::command]
pub fn bridge_file_dismiss() {
    crate::bridge_file::dismiss()
}

/// «Проверить снова»: человек разрешил файл / добавил исключение —
/// восстановить мост (даже если антивирус только что удалял) и запустить.
#[tauri::command]
pub async fn bridge_file_retry() -> Result<Option<crate::bridge_file::Issue>, String> {
    tokio::task::spawn_blocking(|| -> Result<(), String> {
        crate::bridge_file::ensure(true)?;
        shim::start()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(crate::bridge_file::issue())
}

/// Открыть «Безопасность Windows» → «Защита от вирусов и угроз» (журнал
/// защиты и исключения — оттуда в один-два клика).
#[tauri::command]
pub fn open_windows_security() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg("windowsdefender://threat/")
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Не удалось открыть «Безопасность Windows»: {e}"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Только для Windows".into())
    }
}

/// При запуске: встроенный мост новее установленного → обновить сразу.
/// Без этого после обновления приложения мост оставался старым до ручного
/// «Обновить мост» (а новые возможности — SOCKS, закреплённые входы —
/// недоступны).
pub(crate) async fn auto_update_bridge(app: AppHandle) {
    if !crate::tasks::is_registered() {
        return;
    }
    // Правила новой версии (например, новые домены Claude) — сразу в мост.
    let rules_changed = tokio::task::spawn_blocking(crate::tasks::write_rules_file_if_changed)
        .await
        .ok()
        .and_then(|r| r.ok())
        .is_some_and(|(_, changed)| changed);
    let Ok(v) = bridge_versions(app.clone()).await else { return };
    // Мост уже умеет сайты, но задача зарегистрирована со старыми
    // аргументами (мост поставили поверх раньше, чем появились сайты) —
    // перерегистрировать: иначе браузер в режиме «только сайты» без сети.
    if !v.outdated && tokio::task::spawn_blocking(crate::tasks::sites_args_stale).await.unwrap_or(false) {
        let fixed = tokio::task::spawn_blocking(|| -> Result<(), String> {
            let url = settings::load().proxy_url.ok_or("прокси не настроен")?;
            crate::tasks::register(&url)?;
            crate::shim::restart()
        })
        .await;
        match fixed {
            Ok(Ok(())) => tracing::info!("мост перезапущен со входом сайтов"),
            other => tracing::warn!("не удалось включить вход сайтов: {other:?}"),
        }
        return;
    }
    if !v.outdated {
        if rules_changed {
            match tokio::task::spawn_blocking(crate::shim::restart).await {
                Ok(Ok(())) => tracing::info!("правила моста обновлены"),
                other => tracing::warn!("перезапуск моста с новыми правилами не удался: {other:?}"),
            }
        }
        return;
    }
    match bridge_update(app).await {
        Ok(_) => tracing::info!("мост обновлён автоматически: {:?} → {:?}", v.installed, v.bundled),
        Err(e) => tracing::warn!("автообновление моста не удалось: {e}"),
    }
}

/// «Инструменты» → очистка данных аккаунта Claude: что найдено и сколько.
#[tauri::command]
pub async fn claude_wipe_scan() -> Result<crate::claude_wipe::Scan, String> {
    tokio::task::spawn_blocking(crate::claude_wipe::scan)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Запущены ли Claude Desktop и отдельный Claude Code — для частого опроса.
#[tauri::command]
pub async fn claude_wipe_running() -> Result<crate::claude_wipe::Running, String> {
    tokio::task::spawn_blocking(crate::claude_wipe::running)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Закрыть Claude и удалить данные аккаунта (с Claude Code и чатами — по
/// флагам). С `export_chats` локальные чаты сначала сохраняются в архив.
#[tauri::command]
pub async fn claude_wipe_run(
    include_desktop: bool,
    include_code: bool,
    include_chats: bool,
    export_chats: Option<bool>,
) -> Result<crate::claude_wipe::WipeResult, String> {
    let export = export_chats.unwrap_or(false);
    // Пока идёт очистка, тихая установка обновления ждёт.
    let busy = crate::auto_update::busy_guard();
    tokio::task::spawn_blocking(move || {
        let _busy = busy;
        crate::claude_wipe::wipe(include_desktop, include_code, include_chats, export)
    })
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Сохранить все локальные чаты Claude в ZIP в «Загрузки».
#[tauri::command]
pub async fn claude_chats_export() -> Result<crate::claude_chats_export::Archive, String> {
    let busy = crate::auto_update::busy_guard();
    tokio::task::spawn_blocking(move || {
        let _busy = busy;
        crate::claude_chats_export::export()
    })
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub fn claude_chats_export_progress() -> crate::claude_chats_export::Progress {
    crate::claude_chats_export::progress()
}

/// Показать файл в папке (Проводник / Finder) с выделением.
#[tauri::command]
pub fn reveal_in_folder(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).exists() {
        return Err("Файл не найден — возможно, его переместили".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{path}\""))
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Не удалось открыть папку: {e}"))
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .args(["-R", &path])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Не удалось открыть папку: {e}"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Err("Не поддерживается".into())
    }
}
