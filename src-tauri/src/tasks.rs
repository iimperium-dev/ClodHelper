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
//!
//! macOS: вместо задачи — LaunchAgent `ru.iimperium.clodbridge` в
//! `~/Library/LaunchAgents` с KeepAlive (модуль `launchd` ниже). launchd сам
//! перезапускает упавший мост, поэтому `--supervise` там не используется.

use std::path::PathBuf;

use serde::Serialize;

use crate::shim;
#[cfg(target_os = "windows")]
use crate::shim::LISTEN_PORT;

#[cfg(target_os = "windows")]
pub const TASK_NAME: &str = "ClodBridge";
/// Runner-vbs в папке установки моста (%LOCALAPPDATA%\ClaudeDesktopProxy).
#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
fn runner_vbs_path() -> Option<PathBuf> {
    shim::install_dir().map(|d| d.join(RUNNER_VBS))
}

/// Группы AI-сервисов для назначения «сервис → конкретный прокси из пула».
/// Ключи хранятся в settings.proxy_assignments; домены разворачиваются в
/// rules-файл (мост матчит по суффиксу). Синхронизировано с UI (Настройки).
pub const SERVICE_GROUPS: &[(&str, &[&str])] = &[
    ("anthropic", &["anthropic.com", "claude.ai", "claude.com"]),
    (
        "openai",
        &["openai.com", "chatgpt.com", "oaistatic.com", "oaiusercontent.com"],
    ),
    ("google", &["generativelanguage.googleapis.com", "ai.google.dev"]),
    (
        "other_ai",
        &[
            "x.ai",
            "mistral.ai",
            "openrouter.ai",
            "perplexity.ai",
            "githubcopilot.com",
            "cursor.sh",
            "cursor.com",
        ],
    ),
    // Telegram ходит по IP дата-центров через SOCKS-вход моста: тот берёт
    // прокси, назначенный «telegram.org» (bridge socks_upstream). Остальные
    // домены — веб-версия и ссылки в браузере через общий вход.
    (
        "telegram",
        &["telegram.org", "t.me", "telegram.me", "telesco.pe", "tdesktop.com", "telegra.ph"],
    ),
];

/// Записать rules-файл моста из settings (route_mode + proxy_assignments).
/// Вызывается при регистрации задачи и при изменениях в UI. Возвращает путь
/// для аргумента --rules.
pub fn write_rules_file() -> Result<PathBuf, String> {
    let s = crate::settings::load();
    let mode = s
        .route_mode
        .clone()
        .filter(|m| m == "smart")
        .unwrap_or_else(|| "all".into());
    let pool_len = crate::settings::effective_proxies(&s).len();

    // Назначения: группа → индекс прокси; разворачиваем в домен → индекс.
    let mut assignments = serde_json::Map::new();
    for (group, idx) in &s.proxy_assignments {
        if *idx >= pool_len {
            continue;
        }
        if let Some((_, domains)) = SERVICE_GROUPS.iter().find(|(g, _)| g == group) {
            for d in *domains {
                assignments.insert((*d).to_string(), serde_json::json!(idx));
            }
        }
    }

    let json = serde_json::json!({ "mode": mode, "assignments": assignments });
    let path = shim::install_dir()
        .map(|d| d.join(RULES_FILE))
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&json).map_err(|e| format!("serialize rules: {e}"))?;
    // Атомарно (tmp + rename): мост читает этот файл при старте, а супервизор
    // может перезапустить его в любой момент — полузаписанный JSON для моста
    // 1.2.0 означает отказ запуска (fail-fast), для старых — потерю правил.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("replace {}: {e}", path.display()))?;
    Ok(path)
}

/// Зарегистрирована ли задача (быстрая проверка для ветвления start/stop).
pub fn is_registered() -> bool {
    status().registered
}

/// Все прокси пула → повторяемые `--upstream`. ПОРЯДОК СВЯЩЕНЕН: индексы
/// assignments в rules-файле считаются по этому же списку — перестановка
/// без пересчёта rules увела бы закреплённый сервис на другой прокси
/// (другой выходной IP). Поэтому пул берём как есть из настроек;
/// proxy_url — только fallback для пустого пула (легаси-установка).
fn upstream_pool(proxy_url: &str) -> Vec<String> {
    let s = crate::settings::load();
    let list = crate::settings::effective_proxies(&s);
    if list.is_empty() {
        vec![proxy_url.to_string()]
    } else {
        list
    }
}

#[cfg(target_os = "windows")]
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

/// Версия моста по `--version` ("clod-bridge 1.5.0" → "1.5.0"). None —
/// легаси-python или exe не запускается.
pub fn exe_version(exe: &std::path::Path) -> Option<String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--version");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .map(String::from)
}

/// "1.5.0" → (1, 5, 0) для сравнения версий.
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim().split('.').map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

/// Закреплённые входы моста ≥1.7: 8892 + индекс прокси в пуле. Приложение,
/// которому выбран конкретный прокси, запускается с этим портом.
pub const PIN_PORT_BASE: u16 = 8892;

/// Мост ≥1.7 умеет закреплённые входы (`--pin-listen`).
#[cfg(not(target_os = "windows"))]
fn bridge_supports_pin_listen(exe: &std::path::Path) -> bool {
    exe_version(exe)
        .and_then(|v| parse_version(&v))
        .is_some_and(|v| v >= (1, 7, 0))
}

/// Мост ≥1.5 принимает пул прокси файлом (`--upstream-file`).
#[cfg(not(target_os = "windows"))]
fn bridge_supports_upstream_file(exe: &std::path::Path) -> bool {
    exe_version(exe)
        .and_then(|v| parse_version(&v))
        .is_some_and(|v| v >= (1, 5, 0))
}

/// Кэш пробы установленного exe. Ключ — mtime файла: после «Обновить мост»
/// mtime меняется и кэш сбрасывается сам. Проба PyInstaller-exe стоит
/// секунды (распаковка onefile) — без кэша UI-поллинг был бы болью.
static EXE_PROBE_CACHE: std::sync::Mutex<Option<(std::time::SystemTime, bool)>> =
    std::sync::Mutex::new(None);

/// Понимает ли мост режим `--supervise` (1.3.0+)? Проба: `--supervise
/// --version` — новый мост ставит флаг и выходит 0 на --version, старый
/// падает на неизвестном аргументе. Если да — задача Планировщика вешается
/// ПРЯМО на exe (без VBS/wscript: малварь-паттерн для антивирусов, WSH
/// отключаем политиками, VBScript депрекейтнут в Win11 24H2+).
#[cfg(target_os = "windows")]
fn bridge_supports_supervise(exe: &std::path::Path) -> bool {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--supervise", "--version"]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

/// Установленный мост — легаси (старый PyInstaller-python, без /healthz и
/// --rules)? По этому флагу UI показывает «Обновить мост» вместо вечного
/// спиннера. false, если exe не найден вовсе.
pub fn bridge_exe_is_legacy() -> bool {
    let Some(exe) = shim::bridge_exe_path().filter(|p| p.exists()) else {
        return false;
    };
    let mtime = std::fs::metadata(&exe)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    if let Ok(guard) = EXE_PROBE_CACHE.lock() {
        if let Some((t, legacy)) = guard.as_ref() {
            if *t == mtime {
                return *legacy;
            }
        }
    }
    let legacy = !bridge_supports_rules(&exe);
    if let Ok(mut guard) = EXE_PROBE_CACHE.lock() {
        *guard = Some((mtime, legacy));
    }
    legacy
}

/// Зарегистрировать (или перерегистрировать) задачу автозапуска моста.
///
/// Пишет runner-vbs с актуальным upstream-URL, регистрирует задачу и — только
/// ПОСЛЕ успешной регистрации — удаляет легаси .vbs из Startup (порядок важен:
/// если регистрация упала, старый автозапуск остаётся рабочим fallback'ом).
#[cfg(target_os = "windows")]
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

    let pool = upstream_pool(proxy_url);

    let supports_rules = bridge_supports_rules(&exe);
    let supports_supervise = supports_rules && bridge_supports_supervise(&exe);

    // Действие задачи: мост 1.3.0+ супервизорит себя сам — задача вешается
    // ПРЯМО на exe, VBS/wscript исчезают (антивирусный малварь-паттерн
    // «exe пишет VBS + задача на wscript» + депрекация VBScript).
    // Старые мосты — прежний VBS-путь, до первого «Обновить мост».
    let (execute_e, argument_e) = if supports_supervise {
        let rules_path = write_rules_file()?;
        let mut parts: Vec<String> = vec![
            "--supervise".into(),
            "--listen".into(),
            format!("127.0.0.1:{LISTEN_PORT}"),
        ];
        for u in &pool {
            parts.push("--upstream".into());
            parts.push(format!("\"{}\"", u.replace('"', "")));
        }
        parts.push("--rules".into());
        parts.push(format!("\"{}\"", rules_path.to_string_lossy()));
        parts.push("--quiet".into());
        // Старый runner-vbs больше не нужен — подчищаем, чтобы не смущал AV.
        if vbs_path.exists() {
            let _ = std::fs::remove_file(&vbs_path);
        }
        (
            exe.to_string_lossy().replace('\'', "''"),
            parts.join(" ").replace('\'', "''"),
        )
    } else {
        // Легаси: runner-СУПЕРВИЗОР на VBS — скрытый запуск, перезапуск через
        // 5с. --rules только для Rust-моста (см. bridge_supports_rules).
        let args_expr = if supports_rules {
            let rules_path = write_rules_file()?;
            format!(
                "\" --listen 127.0.0.1:{port} --rules \" & Chr(34) & \"{rules}\" & Chr(34) & \" --quiet\"",
                port = LISTEN_PORT,
                rules = rules_path.to_string_lossy(),
            )
        } else {
            format!("\" --listen 127.0.0.1:{port} --quiet\"", port = LISTEN_PORT)
        };
        let upstreams_expr: String = pool
            .iter()
            .map(|u| {
                format!(
                    " & \" --upstream \" & Chr(34) & \"{}\" & Chr(34)",
                    u.replace('"', "")
                )
            })
            .collect();
        let vbs = format!(
            "' Clod bridge supervisor: держит мост запущенным; упавший процесс\r\n\
             ' перезапускается через 5 секунд. Остановка — Stop-ScheduledTask.\r\n\
             Set sh = CreateObject(\"WScript.Shell\")\r\n\
             Do\r\n\
             code = sh.Run(Chr(34) & \"{exe}\" & Chr(34){upstreams} & {args}, 0, True)\r\n\
             WScript.Sleep 5000\r\n\
             Loop\r\n",
            exe = exe.to_string_lossy(),
            upstreams = upstreams_expr,
            args = args_expr,
        );
        // UTF-16LE с BOM: wscript без BOM читает .vbs в ANSI-кодировке системы,
        // и кириллический путь профиля превращается в кракозябры — мост не
        // стартовал бы ни при одном логине.
        let vbs_utf16: Vec<u8> = [0xFF_u8, 0xFE]
            .into_iter()
            .chain(vbs.encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        std::fs::write(&vbs_path, vbs_utf16)
            .map_err(|e| format!("write {}: {e}", vbs_path.display()))?;
        (
            "wscript.exe".to_string(),
            format!("\"{}\"", vbs_path.to_string_lossy()).replace('\'', "''"),
        )
    };

    let script = format!(
        "$user = \"$env:USERDOMAIN\\$env:USERNAME\"; \
         $action = New-ScheduledTaskAction -Execute '{execute_e}' -Argument '{argument_e}'; \
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
#[cfg(target_os = "windows")]
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
#[cfg(target_os = "windows")]
pub fn stop_ignore_errors() {
    let _ = run_ps(&format!(
        "Stop-ScheduledTask -TaskName '{TASK_NAME}' -ErrorAction SilentlyContinue; \
         Write-Output 'DONE'"
    ));
}

/// Миграция старой установки: есть легаси .vbs в Startup, задачи ещё нет,
/// upstream известен из settings → регистрируем задачу (она удалит .vbs).
/// Вызывается фоново при старте приложения; все ошибки — в лог, не наружу.
#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
fn run_ps(script: &str) -> Option<String> {
    use std::process::Command;
    // UTF-8-прелюдия обязательна: PowerShell 5.1 пишет в пайп OEM-кодировкой
    // (cp866 на русской Windows), и from_utf8_lossy превращает кириллицу
    // (пути профиля, тексты ошибок) в кашу U+FFFD.
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
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(not(target_os = "windows"))]
pub use launchd::{migrate_from_startup_vbs, register, start, status, stop_ignore_errors};

/// Автозапуск моста на macOS: LaunchAgent с KeepAlive.
#[cfg(not(target_os = "windows"))]
mod launchd {
    use std::path::PathBuf;

    use super::{
        bridge_supports_pin_listen, bridge_supports_rules, bridge_supports_upstream_file,
        upstream_pool, write_rules_file, BridgeTaskStatus, PIN_PORT_BASE,
    };
    use crate::macos;
    use crate::shim::{self, LISTEN_PORT};

    pub const LABEL: &str = "ru.iimperium.clodbridge";

    fn plist_path() -> Option<PathBuf> {
        macos::launch_agents_dir().map(|d| d.join(format!("{LABEL}.plist")))
    }

    pub fn status() -> BridgeTaskStatus {
        let registered = plist_path().is_some_and(|p| p.exists());
        BridgeTaskStatus {
            registered,
            state: registered
                .then(|| macos::agent_state(LABEL).unwrap_or_else(|| "NotLoaded".into())),
        }
    }

    /// Записать plist агента с актуальным пулом и правилами. launchd читает
    /// plist только при bootstrap, поэтому загруженный агент выгружаем —
    /// вызывающие сразу делают shim::start/restart (как с задачей на Windows).
    pub fn register(proxy_url: &str) -> Result<(), String> {
        let exe = shim::bridge_exe_path()
            .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
        if !exe.exists() {
            return Err(format!(
                "Не найден {}. Запустите установщик заново.",
                exe.display()
            ));
        }
        if !bridge_supports_rules(&exe) {
            return Err(format!(
                "Мост {} не запускается. Переустановите Clod Helper.",
                exe.display()
            ));
        }
        let rules_path = write_rules_file()?;
        let mut args: Vec<String> = vec![
            exe.to_string_lossy().to_string(),
            "--listen".into(),
            format!("127.0.0.1:{LISTEN_PORT}"),
        ];
        let pool = upstream_pool(proxy_url);
        let pool_len = pool.len();
        let modern = bridge_supports_upstream_file(&exe);
        if modern {
            // Пул — файлом 0600: argv процесса видит любой пользователь Mac
            // через ps, а в URL лежит пароль прокси.
            let file = write_upstream_file(&pool)?;
            args.push("--upstream-file".into());
            args.push(file.to_string_lossy().to_string());
        } else {
            for u in pool {
                args.push("--upstream".into());
                args.push(u);
            }
        }
        args.push("--rules".into());
        args.push(rules_path.to_string_lossy().to_string());
        args.push("--quiet".into());
        if bridge_supports_pin_listen(&exe) {
            for i in 0..pool_len {
                args.push("--pin-listen".into());
                args.push(format!("127.0.0.1:{}={i}", PIN_PORT_BASE + i as u16));
            }
        }

        let plist = plist_path().ok_or_else(|| "Не нашёл домашнюю папку".to_string())?;
        // stderr моста (в --quiet — только ошибки) — для диагностики. launchd
        // дописывает бесконечно: мост ≥1.5 сам ротирует файл (`--log`, 1 МБ →
        // .1); для старого моста обрезаем при каждой перерегистрации.
        let log = shim::install_dir().map(|d| d.join("bridge.log"));
        if let (true, Some(path)) = (modern, log.as_ref()) {
            args.push("--log".into());
            args.push(path.to_string_lossy().to_string());
        }
        if let Some(path) = log.as_ref() {
            if std::fs::metadata(path).is_ok_and(|m| m.len() > 1024 * 1024) {
                let _ = std::fs::remove_file(path);
            }
        }
        macos::write_agent_plist(
            &plist,
            &macos::Agent {
                label: LABEL,
                program_arguments: &args,
                keep_alive: true,
                stderr_log: log.as_deref(),
            },
        )?;
        macos::bootout(LABEL);
        Ok(())
    }

    /// `bridge-upstreams.txt` (0600, атомарно): по URL в строке, порядок пула
    /// сохраняется — индексы assignments считаются по нему.
    fn write_upstream_file(pool: &[String]) -> Result<PathBuf, String> {
        let path = shim::install_dir()
            .map(|d| d.join("bridge-upstreams.txt"))
            .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }
        let mut text = String::from("# Пул прокси Clod Helper (порядок важен). Файл перезаписывается.\n");
        for u in pool {
            text.push_str(u);
            text.push('\n');
        }
        let tmp = path.with_extension("txt.tmp");
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| format!("write {}: {e}", tmp.display()))?;
            f.write_all(text.as_bytes())
                .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        }
        macos::set_mode(&tmp, 0o600);
        std::fs::rename(&tmp, &path).map_err(|e| format!("replace {}: {e}", path.display()))?;
        Ok(path)
    }

    pub fn start() -> Result<(), String> {
        let plist = plist_path().ok_or_else(|| "Не нашёл домашнюю папку".to_string())?;
        if !plist.exists() {
            return Err("Автозапуск моста не настроен".into());
        }
        if macos::agent_loaded(LABEL) {
            let target = format!("{}/{LABEL}", macos::gui_domain());
            let (ok, out) = macos::launchctl(&["kickstart", &target]);
            return if ok {
                Ok(())
            } else {
                Err(format!("launchctl kickstart: {}", out.trim()))
            };
        }
        macos::bootstrap(LABEL, &plist)
    }

    /// Выгрузить агент — иначе KeepAlive поднял бы убитый мост через 5 секунд.
    pub fn stop_ignore_errors() {
        macos::bootout(LABEL);
    }

    /// На macOS легаси-автозапуска (.vbs) не было — мигрировать нечего.
    pub fn migrate_from_startup_vbs() {}
}

#[cfg(test)]
mod version_tests {
    use super::parse_version;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("1.5.0"), Some((1, 5, 0)));
        assert_eq!(parse_version("1.10"), Some((1, 10, 0)));
        assert!(parse_version("1.10.0") > parse_version("1.9.9"));
        assert_eq!(parse_version("abc"), None);
        assert_eq!(parse_version(""), None);
    }
}
