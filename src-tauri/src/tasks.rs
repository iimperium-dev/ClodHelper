//! Автозапуск моста через Планировщик задач Windows (Scheduled Task).
//!
//! Зачем: .vbs в Startup-папке запускал мост ОДИН раз при логине; если процесс
//! падал — поднять его мог только watchdog внутри GUI хелпера. GUI закрыт →
//! мост лежит до ручного вмешательства.
//!
//! Механика: задача запускает wscript.exe с runner-vbs-СУПЕРВИЗОРОМ, который
//! в бесконечном цикле стартует clod-bridge.exe СКРЫТО (окна нет), ждёт его
//! завершения и через 5 секунд перезапускает. Падение моста лечится за ~5с
//! на уровне ОС, без нашего GUI. Прямой запуск console-exe из задачи показал
//! бы окно консоли при логине — поэтому wscript.
//!
//! Почему цикл в vbs, а не RestartOnFailure Планировщика: проверено вживую на
//! Win11 — у Interactive-задачи ненулевой код выхода НЕ триггерит рестарт
//! (задача умирает молча). RestartOnFailure оставлен в настройках задачи как
//! best-effort на случай гибели самого wscript, но рассчитывать на него нельзя.
//!
//! Остановка: Stop-ScheduledTask убивает супервизор (иначе цикл поднимет мост
//! через 5с после kill) — см. stop_ignore_errors и shim::stop.
//!
//! Регистрация НЕ требует админа: задача в пользовательской папке, LogonType
//! Interactive, RunLevel Limited (проверено вживую на Win11).

use std::path::PathBuf;

use serde::Serialize;

use crate::shim::{self, LISTEN_PORT};

pub const TASK_NAME: &str = "ClodBridge";
/// Runner-vbs в папке установки моста (%LOCALAPPDATA%\ClaudeDesktopProxy).
const RUNNER_VBS: &str = "clod-bridge-task.vbs";
/// Правила маршрутизации моста (mode smart/all) — читает clod-bridge при старте.
const RULES_FILE: &str = "bridge-rules.json";

/// Статус задачи автозапуска для UI.
#[derive(Debug, Clone, Serialize)]
pub struct BridgeTaskStatus {
    pub registered: bool,
    /// Ready / Running / Disabled — как отдаёт Планировщик.
    pub state: Option<String>,
}

fn runner_vbs_path() -> Option<PathBuf> {
    shim::install_dir().map(|d| d.join(RUNNER_VBS))
}

/// Записать rules-файл моста из settings.route_mode. Вызывается при регистрации
/// задачи и при переключении режима в UI. Возвращает путь для аргумента --rules.
pub fn write_rules_file() -> Result<PathBuf, String> {
    let mode = crate::settings::load()
        .route_mode
        .filter(|m| m == "smart")
        .unwrap_or_else(|| "all".into());
    let path = shim::install_dir()
        .map(|d| d.join(RULES_FILE))
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let json = format!("{{\n  \"mode\": \"{mode}\"\n}}\n");
    std::fs::write(&path, json).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// Зарегистрирована ли задача (быстрая проверка для ветвления start/stop).
pub fn is_registered() -> bool {
    status().registered
}

pub fn status() -> BridgeTaskStatus {
    let script = format!(
        "$t = Get-ScheduledTask -TaskName '{TASK_NAME}' -ErrorAction SilentlyContinue; \
         if ($t) {{ Write-Output ('TASK_STATE:' + $t.State) }} else {{ Write-Output 'TASK_MISSING' }}"
    );
    let out = run_ps(&script).unwrap_or_default();
    if let Some(state) = out.lines().find_map(|l| l.trim().strip_prefix("TASK_STATE:")) {
        BridgeTaskStatus {
            registered: true,
            state: Some(state.trim().to_string()),
        }
    } else {
        BridgeTaskStatus {
            registered: false,
            state: None,
        }
    }
}

/// Понимает ли установленный мост новые флаги (--rules)?
///
/// КРИТИЧНО: старый PyInstaller-мост на неизвестный аргумент падает (argparse
/// exit 2) — супервизор вечно рестартил бы мёртвый мост, и прокси лежал бы
/// НАМЕРТВО. Проба: `--version` есть только у Rust-моста (exit 0); старый
/// на него падает → пишем легаси-набор аргументов без --rules.
fn bridge_supports_rules(exe: &std::path::Path) -> bool {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--version");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

/// Зарегистрировать (или перерегистрировать) задачу автозапуска моста.
///
/// Пишет runner-vbs с актуальным upstream-URL, регистрирует задачу и — только
/// ПОСЛЕ успешной регистрации — удаляет легаси .vbs из Startup (порядок важен:
/// если регистрация упала, старый автозапуск остаётся рабочим fallback'ом).
pub fn register(proxy_url: &str) -> Result<(), String> {
    let exe = shim::bridge_exe_path()
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
    if !exe.exists() {
        return Err(format!(
            "Не найден {}. Запустите установщик заново.",
            exe.display()
        ));
    }
    let vbs_path = runner_vbs_path()
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;

    // --rules только для нового Rust-моста (см. bridge_supports_rules).
    let args_expr = if bridge_supports_rules(&exe) {
        let rules_path = write_rules_file()?;
        format!(
            "\" --listen 127.0.0.1:{port} --rules \" & Chr(34) & \"{rules}\" & Chr(34) & \" --quiet\"",
            port = LISTEN_PORT,
            rules = rules_path.to_string_lossy(),
        )
    } else {
        format!("\" --listen 127.0.0.1:{port} --quiet\"", port = LISTEN_PORT)
    };

    // Runner-СУПЕРВИЗОР: скрытый запуск, ожидание, перезапуск упавшего моста
    // через 5с. Цикл бесконечный — умирает только вместе с wscript (Stop-
    // ScheduledTask). Если exe исчез (деинсталляция) — sh.Run кидает ошибку,
    // vbs завершается, цикл не молотит вхолостую.
    let vbs = format!(
        "' Clod bridge supervisor: держит мост запущенным; упавший процесс\r\n\
         ' перезапускается через 5 секунд. Остановка — Stop-ScheduledTask.\r\n\
         Set sh = CreateObject(\"WScript.Shell\")\r\n\
         Do\r\n\
         code = sh.Run(Chr(34) & \"{exe}\" & Chr(34) & \" --upstream \" & Chr(34) & \"{proxy}\" & Chr(34) & {args}, 0, True)\r\n\
         WScript.Sleep 5000\r\n\
         Loop\r\n",
        exe = exe.to_string_lossy(),
        proxy = proxy_url.replace('"', ""),
        args = args_expr,
    );
    std::fs::write(&vbs_path, vbs)
        .map_err(|e| format!("write {}: {e}", vbs_path.display()))?;

    let vbs_e = vbs_path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$user = \"$env:USERDOMAIN\\$env:USERNAME\"; \
         $action = New-ScheduledTaskAction -Execute 'wscript.exe' -Argument ('\"' + '{vbs_e}' + '\"'); \
         $trigger = New-ScheduledTaskTrigger -AtLogOn -User $user; \
         $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries \
           -StartWhenAvailable -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) \
           -ExecutionTimeLimit (New-TimeSpan) -MultipleInstances IgnoreNew; \
         $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited; \
         Register-ScheduledTask -TaskName '{TASK_NAME}' -Action $action -Trigger $trigger \
           -Settings $settings -Principal $principal -Force | Out-Null; \
         Write-Output 'TASK_REGISTERED'"
    );
    let out = run_ps(&script)
        .ok_or_else(|| "powershell.exe не выполнил регистрацию задачи".to_string())?;
    if !out.contains("TASK_REGISTERED") {
        return Err(format!("Не удалось зарегистрировать задачу: {out}"));
    }

    // Миграция: легаси-автозапуск из Startup больше не нужен.
    if let Some(legacy) = shim::startup_vbs_path() {
        if legacy.exists() {
            let _ = std::fs::remove_file(&legacy);
        }
    }
    Ok(())
}

/// Запустить задачу (мост поднимется скрыто через runner-vbs).
pub fn start() -> Result<(), String> {
    let script = format!(
        "Start-ScheduledTask -TaskName '{TASK_NAME}'; Write-Output 'TASK_STARTED'"
    );
    let out = run_ps(&script)
        .ok_or_else(|| "powershell.exe не выполнил запуск задачи".to_string())?;
    if out.contains("TASK_STARTED") {
        Ok(())
    } else {
        Err(format!("Не удалось запустить задачу: {out}"))
    }
}

/// Остановить задачу. ОБЯЗАТЕЛЬНО вызывать ПЕРЕД kill процессов моста:
/// runner-супервизор перезапустил бы убитый мост через 5с, и «Остановить»
/// в UI не работало бы. Stop-ScheduledTask убивает супервизор (wscript) —
/// после этого мост можно гасить спокойно. Ошибки глотаем (задачи может
/// не быть на легаси-установке).
pub fn stop_ignore_errors() {
    let _ = run_ps(&format!(
        "Stop-ScheduledTask -TaskName '{TASK_NAME}' -ErrorAction SilentlyContinue; \
         Write-Output 'DONE'"
    ));
}

/// Миграция старой установки: есть легаси .vbs в Startup, задачи ещё нет,
/// upstream известен из settings → регистрируем задачу (она удалит .vbs).
/// Вызывается фоново при старте приложения; все ошибки — в лог, не наружу.
pub fn migrate_from_startup_vbs() {
    let legacy_exists = shim::startup_vbs_path().map(|p| p.exists()).unwrap_or(false);
    if !legacy_exists || is_registered() {
        return;
    }
    let Some(url) = crate::settings::load().proxy_url else {
        return;
    };
    match register(&url) {
        Ok(()) => tracing::info!("миграция автозапуска: Startup .vbs → задача {TASK_NAME}"),
        Err(e) => tracing::warn!("миграция автозапуска не удалась: {e}"),
    }
}

fn run_ps(script: &str) -> Option<String> {
    use std::process::Command;
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
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
