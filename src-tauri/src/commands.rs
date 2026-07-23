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
pub fn launch_claude_desktop() -> Result<(), String> {
    // Prefer the shortcut we created on desktop — it carries the
    // --proxy-server flag. If the user deleted it, fall back to launching
    // Claude.exe directly out of the MSIX install location.
    let userprofile = std::env::var("USERPROFILE")
        .map_err(|e| format!("USERPROFILE не задан: {e}"))?;
    let shortcut = std::path::PathBuf::from(&userprofile)
        .join("Desktop")
        .join("Claude Desktop (proxy).lnk");

    if shortcut.exists() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "", shortcut.to_str().unwrap_or("")]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn().map_err(|e| format!("Не удалось запустить ярлык: {e}"))?;
        return Ok(());
    }

    // Fallback: launch via shell:AppsFolder (MSIX activation).
    let mut cmd = Command::new("cmd");
    cmd.args([
        "/c",
        "start",
        "",
        "shell:AppsFolder\\Claude_pzs8sxrjxfjjc!Claude",
    ]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map_err(|e| format!("Не удалось запустить Claude Desktop: {e}"))?;
    Ok(())
}

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

#[tauri::command]
pub fn launch_cursor() -> Result<(), String> {
    // Предпочитаем ярлык «Cursor (proxy)» — он несёт --proxy-server. Если его
    // нет (пользователь удалил) — запускаем Cursor.exe напрямую с флагом.
    let userprofile = std::env::var("USERPROFILE")
        .map_err(|e| format!("USERPROFILE не задан: {e}"))?;
    let shortcut = std::path::PathBuf::from(&userprofile)
        .join("Desktop")
        .join("Cursor (proxy).lnk");

    if shortcut.exists() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "", shortcut.to_str().unwrap_or("")]);
        #[cfg(target_os = "windows")]
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
    #[cfg(target_os = "windows")]
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
fn resolve_resource(app: &AppHandle, name: &str) -> Result<std::path::PathBuf, String> {
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
async fn run_ps_capture(
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

fn log_line(file: &Arc<Mutex<Option<std::fs::File>>>, text: &str) {
    if let Ok(mut guard) = file.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{text}");
        }
    }
}

fn emit(app: &AppHandle, line: impl Into<String>) {
    let _ = app.emit("install:log", line.into());
}

fn emit_verbose(app: &AppHandle, line: impl Into<String>) {
    let _ = app.emit("install:log", format!("__verbose__:{}", line.into()));
}

async fn run_diagnostic(
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

async fn run_ps_script(
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

// ─── Shim control (Dashboard) ────────────────────────────────────

#[tauri::command]
pub async fn shim_status() -> Result<ShimStatus, String> {
    // Скан процессов sysinfo — сотни миллисекунд; с главного потока — фриз.
    tokio::task::spawn_blocking(shim::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

#[tauri::command]
pub async fn shim_start() -> Result<(), String> {
    // Запуск через .vbs — синхронный shell, обернём в blocking чтобы не
    // блокировать tokio executor.
    tokio::task::spawn_blocking(shim::start)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub async fn shim_stop() -> Result<usize, String> {
    tokio::task::spawn_blocking(shim::stop)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub async fn shim_restart() -> Result<(), String> {
    tokio::task::spawn_blocking(shim::restart)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub async fn shim_test() -> ShimTestResult {
    shim::test().await
}

#[tauri::command]
pub fn shim_script_path() -> Option<String> {
    // Сигнал «мост установлен» для App.tsx (Dashboard vs Wizard).
    // Новый маркер — clod-bridge.exe; легаси — local-proxy.py (установки,
    // где exe ещё не появился).
    shim::bridge_exe_path()
        .filter(|p| p.exists())
        .or_else(|| shim::shim_script_path().filter(|p| p.exists()))
        .map(|p| p.to_string_lossy().to_string())
}

// ─── Settings ────────────────────────────────────────────────────

#[tauri::command]
pub fn settings_get() -> Settings {
    settings::load()
}

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
    });
    let saved = settings::set_profiles(s.app_profiles)?;
    Ok(apps::statuses(&saved.app_profiles))
}

/// Удалить кастомный профиль (builtin удалять нельзя — только выключить).
#[tauri::command]
pub async fn apps_remove(id: String) -> Result<Vec<AppProfileStatus>, String> {
    let mut s = settings::load_with_presets();
    if let Some(p) = s.app_profiles.iter().find(|p| p.id == id) {
        if p.builtin {
            return Err("Встроенный профиль нельзя удалить — только выключить".into());
        }
    }
    s.app_profiles.retain(|p| p.id != id);
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

// ─── Killswitch (firewall) ──────────────────────────────────────

/// Фактическое состояние killswitch (читает Windows Firewall, без UAC).
#[tauri::command]
pub async fn killswitch_status() -> KillswitchStatus {
    tokio::task::spawn_blocking(firewall::status)
        .await
        .unwrap_or(KillswitchStatus {
            active: false,
            rule_count: 0,
            blocked_processes: Vec::new(),
        })
}

/// Включить killswitch: собрать пути процессов включённых профилей и
/// поставить outbound-block правила (один UAC-промпт).
#[tauri::command]
pub async fn killswitch_enable() -> Result<KillswitchStatus, String> {
    let s = settings::load_with_presets();
    let paths = tokio::task::spawn_blocking({
        let profiles = s.app_profiles.clone();
        move || apps::resolve_process_paths(&profiles)
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;

    if paths.is_empty() {
        return Err(
            "Не нашёл процессов для блокировки. Включите приложение (VS Code / Codex) \
             и убедитесь что оно установлено."
                .into(),
        );
    }

    tokio::task::spawn_blocking(move || firewall::enable(&paths))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;

    let _ = settings::set_killswitch(true);
    Ok(tokio::task::spawn_blocking(firewall::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?)
}

/// Выключить killswitch: снять все наши правила (один UAC-промпт).
#[tauri::command]
pub async fn killswitch_disable() -> Result<KillswitchStatus, String> {
    tokio::task::spawn_blocking(firewall::disable)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    let _ = settings::set_killswitch(false);
    Ok(tokio::task::spawn_blocking(firewall::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?)
}

// ─── Глобальные env-переменные (главный выключатель) ────────────

/// Прочитать глобальные HTTP(S)_PROXY (User scope) — состояние тумблера.
#[tauri::command]
pub async fn global_proxy_env() -> Result<GlobalProxyEnv, String> {
    // Внутри спавн PowerShell — с главного потока это фриз окна.
    tokio::task::spawn_blocking(env_proxy::read)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Включить системное проксирование: HTTP(S)_PROXY → мост (User scope, без UAC).
/// Заодно best-effort чистим легаси-ключи из settings.json VS Code — при
/// глобальном env они стали лишними дублями.
#[tauri::command]
pub async fn set_global_proxy_env() -> Result<GlobalProxyEnv, String> {
    tokio::task::spawn_blocking(|| {
        env_proxy::set(&format!("http://127.0.0.1:{}", shim::LISTEN_PORT))?;
        let _ = crate::vscode_config::disable();
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(env_proxy::read())
}

/// Удалить глобальные HTTP(S)_PROXY из системы (User scope, без UAC).
#[tauri::command]
pub async fn clear_global_proxy_env() -> Result<GlobalProxyEnv, String> {
    tokio::task::spawn_blocking(env_proxy::clear)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(env_proxy::read())
}

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
    tokio::task::spawn_blocking(move || crate::tasks::register(&url))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(tokio::task::spawn_blocking(crate::tasks::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?)
}

// ─── Мост: здоровье и маршрутизация ─────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeUpstreamHealth {
    pub url: String,
    pub healthy: bool,
    pub ok: u64,
    pub fail: u64,
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

/// Живые счётчики моста. Ошибка = мост не отвечает ИЛИ это старый python-мост
/// без /healthz (UI предложит «Обновить мост»).
#[tauri::command]
pub async fn bridge_health() -> Result<BridgeHealth, String> {
    let client = reqwest::Client::builder()
        .no_proxy() // сами к себе — не через системный прокси
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| format!("client: {e}"))?;
    let resp = client
        .get(format!("http://127.0.0.1:{}/healthz", shim::LISTEN_PORT))
        .send()
        .await
        .map_err(|e| format!("мост не отвечает: {e}"))?;
    resp.json::<BridgeHealth>()
        .await
        .map_err(|e| format!("не удалось разобрать healthz: {e}"))
}

/// Переключить режим маршрутизации моста ("smart" | "all"): сохранить в
/// settings, переписать rules-файл (+vbs при известном upstream), перезапустить.
#[tauri::command]
pub async fn bridge_set_route_mode(mode: String) -> Result<(), String> {
    if mode != "smart" && mode != "all" {
        return Err("Режим должен быть smart или all".into());
    }
    settings::set_route_mode(mode)?;
    // Задача уже могла быть зарегистрирована без --rules (старый формат) —
    // полная перерегистрация гарантирует актуальные аргументы runner-vbs.
    if let Some(url) = settings::load().proxy_url {
        tokio::task::spawn_blocking(move || crate::tasks::register(&url))
            .await
            .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    } else {
        tokio::task::spawn_blocking(crate::tasks::write_rules_file)
            .await
            .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    }
    tokio::task::spawn_blocking(shim::restart)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(())
}

/// Обновить установленный мост из ресурсов приложения: stop → copy → start.
/// Нужен для миграции живых установок со старого python-моста на Rust-мост
/// (новый exe попадает в resources при апдейте Clod Helper).
#[tauri::command]
pub async fn bridge_update(app: AppHandle) -> Result<String, String> {
    let src = resolve_resource(&app, "clod-bridge.exe")?;
    if !src.exists() {
        return Err("В ресурсах приложения нет clod-bridge.exe".into());
    }
    let dst = shim::bridge_exe_path()
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let _ = shim::stop(); // работающий exe залочен Windows — сначала стоп
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }
        // Страховка отката: старый exe сохраняем рядом (.bak) — если новый
        // мост у пользователя поведёт себя хуже, можно вернуть вручную.
        if dst.exists() {
            let _ = std::fs::copy(&dst, dst.with_extension("exe.bak"));
        }
        // Файл может освобождаться не мгновенно после kill — пара ретраев.
        let mut last_err = String::new();
        for _ in 0..3 {
            match std::fs::copy(&src, &dst) {
                Ok(_) => {
                    // Перерегистрировать задачу: новый Rust-мост понимает
                    // --rules, vbs должен его получить (иначе smart-режим не
                    // применится). Не фатально — легаси-vbs тоже рабочий.
                    if let Some(url) = settings::load().proxy_url {
                        if let Err(e) = crate::tasks::register(&url) {
                            tracing::warn!("bridge_update: перерегистрация задачи не удалась: {e}");
                        }
                    }
                    shim::start()?;
                    return Ok(format!("Мост обновлён: {}", dst.display()));
                }
                Err(e) => {
                    last_err = e.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
        // Копирование не удалось (exe залочен антивирусом и т.п.) — старый exe
        // на месте, но мост уже остановлен shim::stop(). Обязательно поднимаем
        // обратно, иначе прокси лежит намертво до ручного «Запустить».
        if let Err(e) = shim::start() {
            tracing::warn!("bridge_update: откат-запуск старого моста не удался: {e}");
        }
        Err(format!("Не удалось скопировать мост: {last_err} (старый мост перезапущен)"))
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

// ─── Прокси-пул (до 5 upstream'ов + назначения по сервисам) ──────

/// TCP-пинг одного прокси из пула (мс). None = не подключились.
#[derive(Debug, Serialize)]
pub struct ProxyPing {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub ms: Option<u64>,
}

#[tauri::command]
pub fn proxies_get() -> Vec<String> {
    let s = settings::load();
    settings::effective_proxies(&s)
}

/// Заменить пул (1..=5; [0] — основной): валидация, сохранение, перерегистрация
/// задачи (vbs получает все --upstream) и перезапуск моста.
#[tauri::command]
pub async fn proxies_set(urls: Vec<String>) -> Result<Vec<String>, String> {
    for u in &urls {
        proxy::parse(u)?;
    }
    let saved = settings::set_proxies(urls)?;
    let primary = saved.proxy_url.clone().unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        // Не фатально: без задачи мост перезапустится с одним upstream (легаси).
        if let Err(e) = crate::tasks::register(&primary) {
            tracing::warn!("proxies_set: перерегистрация задачи: {e}");
        }
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(settings::effective_proxies(&settings::load()))
}

#[tauri::command]
pub fn proxy_assignments_get() -> std::collections::HashMap<String, usize> {
    settings::load().proxy_assignments
}

/// Имена (тэги) прокси: URL → имя.
#[tauri::command]
pub fn proxy_labels_get() -> std::collections::HashMap<String, String> {
    settings::load().proxy_labels
}

/// Задать/убрать имя прокси (пустое = убрать). Чисто косметика — мост не трогаем.
#[tauri::command]
pub fn proxy_label_set(
    url: String,
    label: String,
) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(settings::set_proxy_label(url, label)?.proxy_labels)
}

/// Назначения «сервис → прокси из пула»: сохранить, переписать rules,
/// перезапустить мост (rules читаются при старте).
#[tauri::command]
pub async fn proxy_assignments_set(
    assignments: std::collections::HashMap<String, usize>,
) -> Result<(), String> {
    settings::set_proxy_assignments(assignments)?;
    tokio::task::spawn_blocking(|| -> Result<(), String> {
        crate::tasks::write_rules_file()?;
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(())
}

/// TCP-пинг всего пула параллельно (лёгкий connect, не через интернет-запрос).
#[tauri::command]
pub async fn proxies_ping() -> Vec<ProxyPing> {
    let pool = settings::effective_proxies(&settings::load());
    let handles: Vec<_> = pool
        .into_iter()
        .map(|url| {
            tokio::spawn(async move {
                let (host, port) = match proxy::parse(&url) {
                    Ok(c) => (c.host, c.port),
                    Err(_) => return ProxyPing { url, host: String::new(), port: 0, ms: None },
                };
                let (h, p) = (host.clone(), port);
                let probe = tokio::task::spawn_blocking(move || {
                    use std::net::{TcpStream, ToSocketAddrs};
                    format!("{h}:{p}")
                        .to_socket_addrs()
                        .ok()
                        .and_then(|mut a| a.next())
                        .and_then(|addr| {
                            let t = std::time::Instant::now();
                            TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(4))
                                .ok()
                                .map(|_| t.elapsed().as_millis() as u64)
                        })
                });
                // DNS-резолв внутри блокирующей задачи своего таймаута не имеет —
                // ограничиваем всё целиком, иначе зависший резолвер держит
                // «пинг» в UI десятки секунд.
                let ms = match tokio::time::timeout(std::time::Duration::from_secs(6), probe).await {
                    Ok(Ok(ms)) => ms,
                    _ => None,
                };
                ProxyPing { url, host, port, ms }
            })
        })
        .collect();
    let mut out = Vec::new();
    for h in handles {
        if let Ok(p) = h.await {
            out.push(p);
        }
    }
    out
}

// ─── Окна (для трей-попапа) ──────────────────────────────────────

/// Показать главное окно (кнопка «Открыть приложение» в трей-попапе).
#[tauri::command]
pub fn show_main_window(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// Полный выход из GUI. Мост — отдельный процесс, он продолжает работать.
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub async fn settings_set_proxy(url: String) -> Result<Settings, String> {
    // 1) Validate proxy first — fail fast.
    proxy::parse(&url)?;

    // 2) Persist to settings.json.
    let saved = settings::set_proxy_url(url.clone())?;

    // 3) Перерегистрировать задачу автозапуска: runner-vbs получит новый
    //    upstream-URL (заодно мигрирует легаси-установку со Startup .vbs).
    let url_for_task = url.clone();
    tokio::task::spawn_blocking(move || crate::tasks::register(&url_for_task))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;

    // 4) Restart shim so new URL applies immediately.
    tokio::task::spawn_blocking(shim::restart)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;

    Ok(saved)
}

