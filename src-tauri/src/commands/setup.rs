//! Установка, детект окружения, Claude Desktop/политика, Telegram, поиск и раскладка приложений.

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

// ВАЖНО: синхронные #[tauri::command] выполняются в ГЛАВНОМ потоке окна.
// Любая команда, спавнящая PowerShell/сканирующая процессы, обязана быть
// async + spawn_blocking — иначе UI виснет («не отвечает»), особенно под
// 5-секундным поллингом Dashboard. Проверено вживую: сумма sync-команд
// замораживала окно наглухо.

#[tauri::command]
pub async fn detect_node() -> Result<NodeInfo, String> {
    tokio::task::spawn_blocking(system::detect_node)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

#[tauri::command]
pub async fn detect_cursor() -> Result<CursorInfo, String> {
    tokio::task::spawn_blocking(system::detect_cursor)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

#[tauri::command]
pub async fn launch_claude_desktop() -> Result<(), String> {
    tokio::task::spawn_blocking(crate::claude_desktop::launch)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Telegram через SOCKS-вход моста (macOS): статус по живым соединениям.
#[tauri::command]
pub async fn telegram_status() -> Result<crate::telegram::TelegramStatus, String> {
    tokio::task::spawn_blocking(crate::telegram::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Открыть в Telegram окно «Добавить прокси» (tg://socks) с входом моста.
#[tauri::command]
pub async fn telegram_setup() -> Result<crate::telegram::TelegramStatus, String> {
    tokio::task::spawn_blocking(crate::telegram::setup)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Claude policy is separate from system proxy/environment settings.
#[tauri::command]
pub async fn claude_proxy_status() -> Result<crate::claude_proxy::ClaudeProxyStatus, String> {
    crate::claude_proxy::status().await
}

#[tauri::command]
pub async fn claude_proxy_enable(app: AppHandle) -> Result<crate::claude_proxy::ClaudeProxyStatus, String> {
    let policy = tokio::task::spawn_blocking(crate::claude_policy::status)
        .await.map_err(|e| e.to_string())??;
    if let Some(conflict) = policy.conflict {
        return Err(conflict);
    }
    // Upgrade only the bridge, preserving its existing shared routing config.
    // Never point Claude at the old Smart listener as a fallback.
    if crate::claude_proxy::check_bridge().await.is_err() {
        bridge_update(app).await?;
        crate::claude_proxy::wait_for_bridge().await?;
    }
    crate::claude_proxy::check_bridge().await?;
    let policy = tokio::task::spawn_blocking(|| {
        crate::claude_desktop::install_launcher()?;
        crate::claude_policy::enable()
    }).await.map_err(|e| e.to_string())??;
    // Claude Code вне приложения (Терминал, VS Code) настройку Claude Desktop
    // не видит — ему нужны системные переменные прокси. Включаем вместе с
    // Claude; чужой прокси не трогаем. Через прокси идёт только AI (умный режим).
    let _ = tokio::task::spawn_blocking(|| {
        if !crate::env_proxy::read().present {
            crate::env_proxy::set(&format!("http://127.0.0.1:{}", crate::shim::LISTEN_PORT))?;
        }
        Ok::<(), String>(())
    })
    .await;
    let mut status = crate::claude_proxy::status().await?;
    status.policy = policy;
    Ok(status)
}

#[tauri::command]
pub async fn claude_proxy_disable() -> Result<crate::claude_proxy::ClaudeProxyStatus, String> {
    let policy = tokio::task::spawn_blocking(crate::claude_policy::disable)
        .await.map_err(|e| e.to_string())??;
    let mut status = crate::claude_proxy::status().await?;
    status.policy = policy; // retain any ownership/restore notice
    Ok(status)
}

/// macOS: новое окно Терминала с `claude`. Терминал запускается через
/// LaunchServices и получает глобальные прокси-переменные (launchctl setenv).
#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn launch_claude_code() -> Result<(), String> {
    let out = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "tell application \"Terminal\" to do script \"claude\"",
            "-e",
            "tell application \"Terminal\" to activate",
        ])
        .output()
        .map_err(|e| format!("Не удалось открыть Терминал: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Не удалось открыть Терминал: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub fn launch_claude_code() -> Result<(), String> {
    // Open a new Windows Terminal (preferred) or PowerShell window with
    // `claude` ready to run. Detached so it survives our app's lifetime.
    // wt запускаем НАПРЯМУЮ: через `cmd /c start` спавн «успешен» даже когда
    // wt не установлен (успех у cmd, не у wt) — fallback никогда не срабатывал
    // и кнопка молча ничего не открывала.
    let try_wt = Command::new("wt")
        .args(["powershell", "-NoExit", "-Command", "claude"])
        .spawn();
    if try_wt.is_ok() {
        return Ok(());
    }
    Command::new("cmd")
        .args([
            "/c",
            "start",
            "",
            "powershell",
            "-NoExit",
            "-Command",
            "claude",
        ])
        .spawn()
        .map_err(|e| format!("Не удалось открыть терминал: {e}"))?;
    Ok(())
}

/// macOS: прямой запуск бандла с флагом (ярлык-обёртка несёт тот же флаг).
#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn launch_cursor() -> Result<(), String> {
    let app = system::find_cursor_exe()
        .ok_or_else(|| "Cursor не найден на этом Mac".to_string())?;
    apps::open_app(
        &app.to_string_lossy(),
        &format!("http://127.0.0.1:{}", shim::LISTEN_PORT),
        "Cursor",
    )
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub fn launch_cursor() -> Result<(), String> {
    // Предпочитаем ярлык «Cursor (proxy)» — он несёт --proxy-server. Если его
    // нет (пользователь удалил) — запускаем Cursor.exe напрямую с флагом.
    // Desktop через known-folder (OneDrive-aware).
    let shortcut = system::cursor_shortcut_path()
        .ok_or_else(|| "Не нашёл рабочий стол".to_string())?;

    if shortcut.exists() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "", shortcut.to_str().unwrap_or("")]);
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn().map_err(|e| format!("Не удалось запустить ярлык: {e}"))?;
        return Ok(());
    }

    // Прокси у Cursor живёт в settings.json (http.proxy), не во флаге запуска,
    // поэтому просто запускаем Cursor.exe — он подхватит прокси из настроек.
    let exe = system::find_cursor_exe()
        .ok_or_else(|| "Cursor не найден на этом ПК".to_string())?;
    let mut cmd = Command::new(&exe);
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map_err(|e| format!("Не удалось запустить Cursor: {e}"))?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct CursorSetupResult {
    pub ok: bool,
    /// Найден ли Cursor.exe (false → пользователю надо сначала поставить Cursor).
    pub found: bool,
    pub message: String,
}

/// macOS: ярлык «Cursor (proxy).app» на рабочем столе.
#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub async fn setup_cursor() -> Result<CursorSetupResult, String> {
    tokio::task::spawn_blocking(|| {
        let Some(app) = system::find_cursor_exe() else {
            return CursorSetupResult {
                ok: false,
                found: false,
                message: "Cursor не найден на этом Mac. Установите его с cursor.com и повторите.".into(),
            };
        };
        let created = system::cursor_shortcut_path()
            .ok_or_else(|| "Не нашёл рабочий стол".to_string())
            .and_then(|dest| {
                let body = format!(
                    "exec /usr/bin/open -a {} --args {}",
                    crate::macos::sh_quote(&app.to_string_lossy()),
                    crate::macos::sh_quote(&format!(
                        "--proxy-server=http://127.0.0.1:{}",
                        shim::LISTEN_PORT
                    )),
                );
                crate::macos::create_app_wrapper(&dest, "Cursor (proxy)", &body, Some(&app))
            });
        match created {
            Ok(()) => CursorSetupResult {
                ok: true,
                found: true,
                message: "Ярлык «Cursor (proxy)» создан на рабочем столе.".into(),
            },
            Err(e) => CursorSetupResult {
                ok: false,
                found: true,
                message: format!("Не удалось создать ярлык: {e}"),
            },
        }
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn setup_cursor(app: AppHandle) -> Result<CursorSetupResult, String> {
    // Создаёт ярлык «Cursor (proxy)». Сам мост уже поднят шимом — здесь только
    // ярлык, поэтому запускаем скрипт блокирующе и читаем результат.
    let script = resolve_resource(&app, "cursor-setup.ps1")?;
    let output = run_ps_capture(&script, &[]).await?;
    let found = !output.contains("Cursor не найден");
    let ok = found && output.contains("ярлык создан");
    let message = if !found {
        "Cursor не найден на этом ПК. Установите его с cursor.com и повторите.".to_string()
    } else if ok {
        "Ярлык «Cursor (proxy)» создан на рабочем столе.".to_string()
    } else {
        "Не удалось создать ярлык. Подробности в логе установки.".to_string()
    };
    Ok(CursorSetupResult { ok, found, message })
}

#[tauri::command]
pub async fn detect_python() -> Result<PythonInfo, String> {
    tokio::task::spawn_blocking(system::detect_python)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

#[tauri::command]
pub fn parse_proxy(url: String) -> Result<ProxyConfig, String> {
    proxy::parse(&url)
}

#[tauri::command]
pub async fn check_proxy(url: String) -> ProxyCheckResult {
    proxy::check(&url).await
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    Code,
    Desktop,
    Both,
}

#[tauri::command]
pub async fn run_install(
    app: AppHandle,
    mode: Option<InstallMode>,
    cursor: bool,
    proxy_url: String,
) -> Result<(), String> {
    // Легаси-установка через PS-скрипты; новый мастер — wizard_install.
    if cfg!(not(target_os = "windows")) {
        return Err("Легаси-установка доступна только на Windows — используйте мастер".into());
    }
    // Validate proxy first — fail fast with a friendly message.
    proxy::parse(&proxy_url)?;

    // Open a debug log file so we can read what actually happened even if the
    // UI shows nothing. Path is shown to the user too.
    let log_path = std::env::temp_dir().join("clod-helper-install.log");
    let log_file = Arc::new(Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok(),
    ));
    log_line(&log_file, "");
    log_line(&log_file, "════════════════════════════════════════════");
    log_line(
        &log_file,
        &format!("run_install started at {:?}", std::time::SystemTime::now()),
    );
    log_line(&log_file, &format!("proxy_url = {proxy_url}"));

    emit_verbose(
        &app,
        format!("⓵ Полный лог: {}", log_path.display()),
    );

    let resources_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("Не удалось найти папку ресурсов: {e}"))?;
    log_line(&log_file, &format!("resource_dir = {}", resources_dir.display()));
    emit_verbose(&app, format!("⓶ resources_dir = {}", resources_dir.display()));

    // ─── DIAGNOSTIC: prove powershell.exe actually works from this process ──
    // (verbose-only — hidden from default UI; still written to file log)
    emit_verbose(&app, "");
    emit_verbose(&app, "── ДИАГНОСТИКА ──");
    log_line(&log_file, "--- diagnostic ---");
    run_diagnostic(&app, &log_file).await;
    emit_verbose(&app, "── /ДИАГНОСТИКА ──");
    emit_verbose(&app, "");

    let code_ps1 = resolve_resource(&app, "claude-setup.ps1")?;
    let desktop_ps1 = resolve_resource(&app, "claude-desktop-setup.ps1")?;
    let cursor_ps1 = resolve_resource(&app, "cursor-setup.ps1")?;

    log_line(&log_file, &format!("code_ps1     = {}", code_ps1.display()));
    log_line(&log_file, &format!("desktop_ps1  = {}", desktop_ps1.display()));
    log_line(&log_file, &format!("cursor_ps1   = {}", cursor_ps1.display()));
    emit_verbose(&app, format!("⓷ code_ps1    = {}", code_ps1.display()));
    emit_verbose(&app, format!("⓸ desktop_ps1 = {}", desktop_ps1.display()));

    let need_code = matches!(mode, Some(InstallMode::Code) | Some(InstallMode::Both));
    let need_desktop = matches!(mode, Some(InstallMode::Desktop) | Some(InstallMode::Both));

    // ВАЖНО: Bridge должен подняться ПЕРВЫМ. Claude Code в шаге 3 прописывает
    // HTTPS_PROXY=http://127.0.0.1:8889 — если bridge ещё не работает, CLI
    // не сможет связаться с Anthropic. Поэтому desktop-setup (он же setup
    // bridge'а) идёт всегда — даже если выбран только Cursor.
    // Если Claude Desktop не выбран — не создаём его ярлык (иначе на
    // cursor-only установке появился бы лишний ярлык Claude Desktop).
    emit(&app, "");
    emit(&app, "▸ Запускаем локальный прокси-bridge…");
    log_line(&log_file, "--- claude-desktop-setup.ps1 (bridge) ---");
    let mut bridge_args: Vec<&str> = vec!["-ProxyUrl", &proxy_url, "-Yes"];
    if !need_desktop {
        bridge_args.push("-SkipDesktopShortcut");
    }
    run_ps_script(&app, &log_file, &desktop_ps1, &bridge_args).await?;
    emit(&app, "✓ Bridge запущен на 127.0.0.1:8889");

    // Автозапуск: setup-скрипт кладёт легаси .vbs в Startup — заменяем его
    // задачей Планировщика с RestartOnFailure (упавший мост поднимается сам).
    // Не фатально: при ошибке легаси .vbs остаётся рабочим запасным путём.
    emit(&app, "▸ Настраиваем автозапуск моста (Планировщик задач)…");
    let url_for_task = proxy_url.clone();
    match tokio::task::spawn_blocking(move || crate::tasks::register(&url_for_task)).await {
        Ok(Ok(())) => emit(&app, "✓ Задача ClodBridge зарегистрирована (автоперезапуск при падении)"),
        Ok(Err(e)) => {
            log_line(&log_file, &format!("WARN: task register failed: {e}"));
            emit(&app, format!("⚠ Планировщик не настроен ({e}) — остаётся автозапуск через Startup"));
        }
        Err(e) => log_line(&log_file, &format!("WARN: task register join: {e}")),
    }

    if need_code {
        emit(&app, "");
        emit(&app, "▸ Устанавливаем Claude Code…");
        log_line(&log_file, "--- claude-setup.ps1 ---");
        run_ps_script(
            &app,
            &log_file,
            &code_ps1,
            &["-ProxyUrl", &proxy_url, "-Yes"],
        )
        .await?;
        emit(&app, "✓ Claude Code установлен");
    }
    if need_desktop {
        // bridge уже запустили выше — здесь ничего дополнительного делать
        // не надо. Сообщение для пользователя:
        emit(&app, "");
        emit(&app, "✓ Claude Desktop настроен");
    }
    if cursor {
        emit(&app, "");
        emit(&app, "▸ Настраиваем Cursor IDE…");
        log_line(&log_file, "--- cursor-setup.ps1 ---");
        run_ps_script(&app, &log_file, &cursor_ps1, &[]).await?;
        emit(&app, "✓ Cursor IDE настроен");
    }

    // Сохраняем proxy_url в settings.json чтобы при следующем запуске
    // приложение открыло Dashboard (а не повторно Wizard).
    if let Err(e) = settings::set_proxy_url(proxy_url.clone()) {
        log_line(&log_file, &format!("WARN: settings save failed: {e}"));
    }

    emit(&app, "");
    emit(&app, "✓ Всё готово!");
    log_line(&log_file, "run_install completed OK");
    Ok(())
}

/// Найти путь к bundled-ресурсу `name` (из папки `resources/`).
/// В dev-режиме папка `resources/` лежит не внутри resource_dir (target/debug/),
/// а в src-tauri/resources/ — поэтому делаем fallback на CARGO_MANIFEST_DIR.
pub(crate) fn resolve_resource(app: &AppHandle, name: &str) -> Result<std::path::PathBuf, String> {
    let resources_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("Не удалось найти папку ресурсов: {e}"))?;
    let primary = resources_dir.join("resources").join(name);
    if primary.exists() {
        return Ok(primary);
    }
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest.join("resources").join(name);
    if dev_path.exists() {
        return Ok(dev_path);
    }
    Ok(primary)
}

/// Запустить PS-скрипт блокирующе и вернуть объединённый stdout+stderr (UTF-8).
/// Используется для коротких операций без стриминга в UI (например setup_cursor
/// с Dashboard). Ошибка — только если powershell не стартовал.
#[cfg(target_os = "windows")]
pub(crate) async fn run_ps_capture(
    script: &std::path::Path,
    args: &[&str],
) -> Result<String, String> {
    if !script.exists() {
        return Err(format!(
            "Не найден скрипт: {}. Переустановите Clod Helper.",
            script.display()
        ));
    }
    let raw = script.to_string_lossy();
    let cleaned = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|s| format!(r"\\{s}"))
        .or_else(|| raw.strip_prefix(r"\\?\").map(String::from))
        .unwrap_or_else(|| raw.to_string());
    let script_escaped = cleaned.replace('\'', "''");
    let mut tail = String::new();
    for a in args {
        if a.starts_with('-') {
            tail.push(' ');
            tail.push_str(a);
        } else {
            let escaped = a.replace('\'', "''");
            tail.push_str(&format!(" '{escaped}'"));
        }
    }
    let command_line = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $OutputEncoding=[System.Text.Encoding]::UTF8; \
         $ErrorActionPreference='Continue'; \
         & '{script_escaped}'{tail}; \
         exit $LASTEXITCODE"
    );
    let ps_args: Vec<String> = vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-OutputFormat".into(),
        "Text".into(),
        "-Command".into(),
        command_line,
    ];
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut cmd = Command::new("powershell.exe");
        cmd.args(&ps_args).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd
            .output()
            .map_err(|e| format!("powershell.exe не запустился: {e}"))?;
        let mut s = String::from_utf8_lossy(&out.stdout).to_string();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(s)
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

pub(crate) fn log_line(file: &Arc<Mutex<Option<std::fs::File>>>, text: &str) {
    if let Ok(mut guard) = file.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{text}");
        }
    }
}

pub(crate) fn emit(app: &AppHandle, line: impl Into<String>) {
    let _ = app.emit("install:log", line.into());
}

pub(crate) fn emit_verbose(app: &AppHandle, line: impl Into<String>) {
    let _ = app.emit("install:log", format!("__verbose__:{}", line.into()));
}

pub(crate) async fn run_diagnostic(
    app: &AppHandle,
    log_file: &Arc<Mutex<Option<std::fs::File>>>,
) {
    let app_c = app.clone();
    let log_c = log_file.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Write-Output 'D1: hello from powershell'; \
             Write-Output ('D2: PSVersion = ' + $PSVersionTable.PSVersion.ToString()); \
             Write-Output ('D3: ExecutionPolicy = ' + (Get-ExecutionPolicy -Scope Process) + ' / ' + (Get-ExecutionPolicy -Scope CurrentUser) + ' / ' + (Get-ExecutionPolicy -Scope LocalMachine)); \
             Write-Output ('D4: script exists = ' + (Test-Path 'C:\\clod-helper\\src-tauri\\target\\debug\\resources\\claude-desktop-setup.ps1')); \
             Write-Output ('D5: pwd = ' + (Get-Location).Path); \
             Write-Output 'D6: done'",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let output = cmd.output();
        match output {
            Ok(o) => {
                let out = String::from_utf8_lossy(&o.stdout);
                let err = String::from_utf8_lossy(&o.stderr);
                for line in out.lines() {
                    emit_verbose(&app_c, line.to_string());
                    log_line(&log_c, line);
                }
                for line in err.lines() {
                    let tagged = format!("[stderr] {line}");
                    emit_verbose(&app_c, tagged.clone());
                    log_line(&log_c, &tagged);
                }
                let s = format!(
                    "[diag-exit] code={:?}, stdout_bytes={}, stderr_bytes={}",
                    o.status.code(),
                    o.stdout.len(),
                    o.stderr.len()
                );
                emit_verbose(&app_c, s.clone());
                log_line(&log_c, &s);
            }
            Err(e) => {
                let s = format!("[diag-error] {e}");
                emit_verbose(&app_c, s.clone());
                log_line(&log_c, &s);
            }
        }
    })
    .await;
}

pub(crate) async fn run_ps_script(
    app: &AppHandle,
    log_file: &Arc<Mutex<Option<std::fs::File>>>,
    script: &std::path::Path,
    args: &[&str],
) -> Result<(), String> {
    if !script.exists() {
        let msg = format!(
            "Не найден скрипт: {}. Переустановите Clod Helper.",
            script.display()
        );
        log_line(log_file, &msg);
        return Err(msg);
    }

    // Tauri's resource_dir() returns Windows UNC-extended paths like
    // `\\?\C:\...`. PowerShell refuses those — strip the prefix so the script
    // is invoked with a plain drive path.
    let raw = script.to_string_lossy();
    let cleaned = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|s| format!(r"\\{s}"))
        .or_else(|| raw.strip_prefix(r"\\?\").map(String::from))
        .unwrap_or_else(|| raw.to_string());

    // Use -Command with a UTF-8 prelude so Cyrillic Write-Host output isn't
    // mangled to OEM (cp866) when there's no real console host attached.
    // We escape the script path and only quote VALUE args (flag names like
    // -ProxyUrl must remain bare or PowerShell binds them positionally).
    let script_escaped = cleaned.replace('\'', "''");
    let mut tail = String::new();
    for a in args {
        if a.starts_with('-') {
            tail.push(' ');
            tail.push_str(a);
        } else {
            let escaped = a.replace('\'', "''");
            tail.push_str(&format!(" '{escaped}'"));
        }
    }
    let command_line = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $OutputEncoding=[System.Text.Encoding]::UTF8; \
         $ErrorActionPreference='Continue'; \
         & '{script_escaped}'{tail}; \
         exit $LASTEXITCODE"
    );

    let mut ps_args: Vec<String> = vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-OutputFormat".into(),
        "Text".into(),
        "-Command".into(),
        command_line.clone(),
    ];
    // Avoid unused warning if future refactor drops args.
    let _ = &mut ps_args;

    let exec_repr = format!("powershell.exe -Command {command_line}");
    log_line(log_file, &format!("[exec] {exec_repr}"));
    emit_verbose(app, format!("[exec] {exec_repr}"));

    // Live streaming: spawn child, read stdout/stderr line-by-line on separate
    // threads, emit to UI as soon as each line arrives. Critical correctness:
    // join the reader threads BEFORE child.wait() — otherwise wait() can close
    // the pipes before drainage completes (this caused the earlier "1 line"
    // bug). Calling wait() AFTER joining is safe because pipe EOF means the
    // child has already exited.
    let app_clone = app.clone();
    let log_clone = log_file.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut cmd = Command::new("powershell.exe");
        cmd.args(&ps_args).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            let msg = format!("powershell.exe не запустился: {e}");
            log_line(&log_clone, &msg);
            msg
        })?;

        let stdout_count = Arc::new(Mutex::new(0u64));
        let stderr_count = Arc::new(Mutex::new(0u64));

        let stdout_handle = child.stdout.take().map(|stdout| {
            let app_inner = app_clone.clone();
            let log_inner = log_clone.clone();
            let count = stdout_count.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut n) = count.lock() {
                        *n += 1;
                    }
                    // Watch for the "Claude Desktop not found" marker so the
                    // UI can pause for the user to install it.
                    if line.contains("Claude Desktop не найден") {
                        let _ = app_inner.emit("install:claude_desktop_missing", ());
                    }
                    log_line(&log_inner, &line);
                    let _ = app_inner.emit("install:log", line);
                }
            })
        });

        let stderr_handle = child.stderr.take().map(|stderr| {
            let app_inner = app_clone.clone();
            let log_inner = log_clone.clone();
            let count = stderr_count.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut n) = count.lock() {
                        *n += 1;
                    }
                    let tagged = format!("[stderr] {line}");
                    log_line(&log_inner, &tagged);
                    let _ = app_inner.emit("install:log", format!("__verbose__:{tagged}"));
                }
            })
        });

        // IMPORTANT: drain pipes first, then wait. Pipe EOF == child exited.
        if let Some(h) = stdout_handle {
            let _ = h.join();
        }
        if let Some(h) = stderr_handle {
            let _ = h.join();
        }

        let status = child.wait().map_err(|e| {
            let msg = format!("Не получилось дождаться скрипта: {e}");
            log_line(&log_clone, &msg);
            msg
        })?;

        let n_out = stdout_count.lock().map(|g| *g).unwrap_or(0);
        let n_err = stderr_count.lock().map(|g| *g).unwrap_or(0);
        let code = status.code().unwrap_or(-1);
        let summary = format!(
            "[exit] code={code}, stdout_lines={n_out}, stderr_lines={n_err}"
        );
        log_line(&log_clone, &summary);
        emit_verbose(&app_clone, summary);

        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "Скрипт завершился с кодом {code} (см. полный лог в файле)"
            ))
        }
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;

    result
}

/// Журнал интерфейса: ошибки JS и случаи самовосстановления окна пишутся в
/// файл (macOS: ~/Library/Logs/ClodHelper/ui.log), чтобы разбирать редкие
/// сбои без скриншотов. Размер ограничен: при 512 КБ файл начинается заново.
#[tauri::command]
pub fn ui_log(level: String, message: String) {
    #[cfg(not(target_os = "windows"))]
    let dir = dirs::home_dir().map(|h| h.join("Library/Logs/ClodHelper"));
    #[cfg(target_os = "windows")]
    let dir = dirs::data_local_dir().map(|d| d.join("ClodHelper"));
    let Some(dir) = dir else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("ui.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line: String = message.chars().take(2000).collect();
    let level: String = level.chars().take(8).collect();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(f, "{ts} [{level}] {}", line.replace('\n', " | "));
    }
}
