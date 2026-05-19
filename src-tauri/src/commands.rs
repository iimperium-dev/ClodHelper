use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::proxy::{self, ProxyCheckResult, ProxyConfig};
use crate::settings::{self, Settings};
use crate::shim::{self, ShimStatus, ShimTestResult};
use crate::system::{self, NodeInfo, PythonInfo};

#[tauri::command]
pub fn detect_node() -> NodeInfo {
    system::detect_node()
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
    let try_wt = Command::new("cmd")
        .args(["/c", "start", "", "wt", "powershell", "-NoExit", "-Command", "claude"])
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
pub fn detect_python() -> PythonInfo {
    system::detect_python()
}

#[tauri::command]
pub fn parse_proxy(url: String) -> Result<ProxyConfig, String> {
    proxy::parse(&url)
}

#[tauri::command]
pub async fn check_proxy(url: String) -> ProxyCheckResult {
    proxy::check(&url).await
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    Code,
    Desktop,
    Both,
}

#[tauri::command]
pub async fn run_install(
    app: AppHandle,
    mode: InstallMode,
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

    let code_ps1 = resources_dir.join("resources").join("claude-setup.ps1");
    let desktop_ps1 = resources_dir
        .join("resources")
        .join("claude-desktop-setup.ps1");

    // Если запустили из dev — папка `resources/` лежит не внутри resource_dir
    // (которая указывает на target/debug/), а в src-tauri/resources/. Делаем
    // fallback на CARGO_MANIFEST_DIR.
    let code_ps1 = ensure_exists(code_ps1, "resources/claude-setup.ps1");
    let desktop_ps1 = ensure_exists(desktop_ps1, "resources/claude-desktop-setup.ps1");

    log_line(&log_file, &format!("code_ps1     = {}", code_ps1.display()));
    log_line(&log_file, &format!("desktop_ps1  = {}", desktop_ps1.display()));
    emit_verbose(&app, format!("⓷ code_ps1    = {}", code_ps1.display()));
    emit_verbose(&app, format!("⓸ desktop_ps1 = {}", desktop_ps1.display()));

    let need_code = matches!(mode, InstallMode::Code | InstallMode::Both);
    let need_desktop = matches!(mode, InstallMode::Desktop | InstallMode::Both);

    // ВАЖНО: Bridge должен подняться ПЕРВЫМ. Claude Code в шаге 3 прописывает
    // HTTPS_PROXY=http://127.0.0.1:8889 — если bridge ещё не работает, CLI
    // не сможет связаться с Anthropic. Поэтому desktop-setup (он же setup
    // bridge'а) идёт всегда — даже если режим = только Code.
    emit(&app, "");
    emit(&app, "▸ Запускаем локальный прокси-bridge…");
    log_line(&log_file, "--- claude-desktop-setup.ps1 (bridge) ---");
    run_ps_script(
        &app,
        &log_file,
        &desktop_ps1,
        &["-ProxyUrl", &proxy_url, "-Yes"],
    )
    .await?;
    emit(&app, "✓ Bridge запущен на 127.0.0.1:8889");

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

fn ensure_exists(primary: std::path::PathBuf, relative: &str) -> std::path::PathBuf {
    if primary.exists() {
        return primary;
    }
    // Fallback to source-tree path used during `tauri dev`.
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest.join(relative);
    if dev_path.exists() {
        return dev_path;
    }
    primary
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
pub fn shim_status() -> ShimStatus {
    shim::status()
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
    // Возвращаем путь только если файл фактически существует — это сигнал
    // "шим установлен", по нему App.tsx решает Dashboard vs Wizard.
    shim::shim_script_path()
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
}

// ─── Settings ────────────────────────────────────────────────────

#[tauri::command]
pub fn settings_get() -> Settings {
    settings::load()
}

#[tauri::command]
pub async fn settings_set_proxy(url: String) -> Result<Settings, String> {
    // 1) Validate proxy first — fail fast.
    proxy::parse(&url)?;

    // 2) Persist to settings.json.
    let saved = settings::set_proxy_url(url.clone())?;

    // 3) Write VBS with new proxy URL.
    let py = system::detect_python_cmd().unwrap_or_else(|| "python".to_string());
    let url_for_vbs = url.clone();
    tokio::task::spawn_blocking(move || {
        shim::write_startup_vbs(&url_for_vbs, Some(&py))
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;

    // 4) Restart shim so new URL applies immediately.
    tokio::task::spawn_blocking(shim::restart)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;

    Ok(saved)
}

