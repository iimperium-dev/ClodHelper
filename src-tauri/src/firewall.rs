//! Killswitch через Windows Firewall — единственная НАСТОЯЩАЯ гарантия.
//!
//! Почему не env-переменные: HTTP_PROXY/HTTPS_PROXY — вежливая просьба. curl,
//! git, реальный python её уважают (fail-closed при мёртвом мосте), но Node.js
//! глобальный `fetch` и PowerShell Invoke-WebRequest её ИГНОРИРУЮТ и текут
//! напрямую под настоящим IP. Codex внутри тащит свой node → та же дыра.
//! Файрвол по имени процесса закрывает трафик физически, независимо от того,
//! уважает приложение прокси или нет.
//!
//! Механика fail-closed: вешаем outbound-BLOCK на каждый целевой процесс
//! (Code.exe, ChatGPT.exe, node.exe, git.exe, python.exe, codex-рантайм).
//! Ключевой факт Windows: трафик на loopback (127.0.0.0/8) файрволом НЕ
//! фильтруется — заблокированный процесс всё равно достаёт локальный мост
//! 127.0.0.1:8889, а мост уже сам решает, жив ли upstream. Если мост мёртв —
//! процесс никуда не выйдет (нет обходного пути наружу) = fail-closed.
//!
//! Правила по ИМЕНИ процесса, не по пути: MSIX-приложения (Codex) живут по
//! версионированному пути WindowsApps, который меняется при апдейте Store.
//!
//! Все изменения требуют прав администратора → выполняем через elevated
//! PowerShell (ShellExecute verb=runas → один UAC-промпт на операцию).

use std::process::Command;

use serde::Serialize;

/// Префикс DisplayName всех наших правил — по нему находим и удаляем свои.
const RULE_PREFIX: &str = "ClodHelper Killswitch";
/// Группа правил (для оптовых операций enable/remove).
const RULE_GROUP: &str = "ClodHelper";

/// Статус killswitch для UI.
#[derive(Debug, Clone, Serialize)]
pub struct KillswitchStatus {
    /// Активны ли наши block-правила прямо сейчас (по факту в файрволе).
    pub active: bool,
    /// Сколько наших правил найдено.
    pub rule_count: usize,
    /// Имена процессов, реально покрытых правилами (для показа в UI).
    pub blocked_processes: Vec<String>,
}

/// Прочитать фактическое состояние killswitch из Windows Firewall.
/// Read-only (Get-NetFirewallRule) — прав администратора НЕ требует.
pub fn status() -> KillswitchStatus {
    let script = format!(
        "$r = Get-NetFirewallRule -Group '{group}' -ErrorAction SilentlyContinue | \
           Where-Object {{ $_.Enabled -eq 'True' -and $_.Direction -eq 'Outbound' -and $_.Action -eq 'Block' }}; \
         if (-not $r) {{ Write-Output 'COUNT:0'; return }}; \
         Write-Output ('COUNT:' + @($r).Count); \
         foreach ($rule in $r) {{ \
           $app = ($rule | Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue).Program; \
           if ($app) {{ Write-Output ('PROC:' + (Split-Path $app -Leaf)) }} \
         }}",
        group = RULE_GROUP,
    );
    let out = run_ps_capture(&script).unwrap_or_default();

    let mut rule_count = 0usize;
    let mut blocked = Vec::new();
    for line in out.lines() {
        if let Some(n) = line.strip_prefix("COUNT:") {
            rule_count = n.trim().parse().unwrap_or(0);
        } else if let Some(p) = line.strip_prefix("PROC:") {
            let p = p.trim().to_string();
            if !p.is_empty() && !blocked.contains(&p) {
                blocked.push(p);
            }
        }
    }

    KillswitchStatus {
        active: rule_count > 0,
        rule_count,
        blocked_processes: blocked,
    }
}

/// Включить killswitch: создать outbound-block правила для указанных программ.
///
/// `program_paths` — уже РАЗРЕШЁННЫЕ полные пути к exe (Windows Firewall
/// требует путь в `-Program`, имя процесса не принимает). Резолв имён процессов
/// в пути делает `apps::resolve_process_paths` до вызова этой функции — включая
/// версионированный путь MSIX (Codex в WindowsApps), который пересобирается при
/// каждом включении, поэтому апдейт Store не ломает killswitch.
///
/// Требует прав администратора → выполняется через elevated PowerShell (UAC).
pub fn enable(program_paths: &[String]) -> Result<(), String> {
    if program_paths.is_empty() {
        return Err("Нет процессов для блокировки — включите хотя бы одно приложение".into());
    }

    // Собираем один PS-скрипт: сначала чистим свои старые правила (идемпотентность),
    // потом создаём блок на каждый путь. Всё под одним UAC-промптом.
    let mut script = String::new();
    script.push_str(&format!(
        "Remove-NetFirewallRule -Group '{group}' -ErrorAction SilentlyContinue; ",
        group = RULE_GROUP,
    ));
    for (i, path) in program_paths.iter().enumerate() {
        let esc = path.replace('\'', "''");
        let leaf = std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("proc{i}"));
        let leaf_esc = leaf.replace('\'', "''");
        script.push_str(&format!(
            "New-NetFirewallRule -DisplayName '{prefix}: {leaf}' -Group '{group}' \
               -Direction Outbound -Action Block -Program '{path}' \
               -Profile Any -Enabled True -ErrorAction SilentlyContinue | Out-Null; ",
            prefix = RULE_PREFIX,
            leaf = leaf_esc,
            group = RULE_GROUP,
            path = esc,
        ));
    }
    script.push_str("Write-Output 'CLODHELPER_FW_DONE'");

    run_ps_elevated(&script)
}

/// Выключить killswitch: удалить все наши правила. Требует админ-прав (UAC).
pub fn disable() -> Result<(), String> {
    let script = format!(
        "Remove-NetFirewallRule -Group '{group}' -ErrorAction SilentlyContinue; \
         Write-Output 'CLODHELPER_FW_DONE'",
        group = RULE_GROUP,
    );
    run_ps_elevated(&script)
}

// ─── PowerShell execution ────────────────────────────────────────

/// Выполнить PS-скрипт БЕЗ elevation (read-only статус). Возвращает stdout.
fn run_ps_capture(script: &str) -> Option<String> {
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    apply_no_window(&mut cmd);
    let out = cmd.output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Выполнить PS-скрипт С elevation через ShellExecute verb=runas (один UAC).
///
/// Пишем скрипт во временный .ps1 и запускаем elevated `powershell -File`.
/// Дожидаемся завершения elevated-процесса и проверяем маркер успеха. Прямой
/// вызов `Start-Process -Verb RunAs -Wait` из НЕ-elevated процесса корректно
/// поднимает UAC и ждёт; exit-код elevated-дочки читаем через $LASTEXITCODE
/// обёртки. Маркер CLODHELPER_FW_DONE пишем в файл-результат, т.к. stdout
/// elevated-процесса нам напрямую недоступен.
fn run_ps_elevated(inner_script: &str) -> Result<(), String> {
    let tmp = std::env::temp_dir();
    let script_path = tmp.join("clodhelper-fw.ps1");
    let result_path = tmp.join("clodhelper-fw-result.txt");

    // Внутренний скрипт пишет свой результат в файл, чтобы мы могли проверить
    // успех после закрытия elevated-окна.
    let _ = std::fs::remove_file(&result_path);
    let full_inner = format!(
        "$ErrorActionPreference='Continue'; \
         try {{ {inner} }} catch {{ Set-Content -LiteralPath '{res}' -Value ('ERR:' + $_.Exception.Message); exit 1 }}; \
         Set-Content -LiteralPath '{res}' -Value 'OK'; exit 0",
        inner = inner_script,
        res = result_path.to_string_lossy().replace('\'', "''"),
    );
    std::fs::write(&script_path, &full_inner)
        .map_err(|e| format!("Не удалось записать временный скрипт: {e}"))?;

    // Обёртка: Start-Process elevated powershell -File <script>, -Wait.
    let wrapper = format!(
        "$p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden \
           -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File','{script}'; \
         exit $p.ExitCode",
        script = script_path.to_string_lossy().replace('\'', "''"),
    );

    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &wrapper,
    ]);
    apply_no_window(&mut cmd);

    let status = cmd
        .status()
        .map_err(|e| format!("Не удалось запустить UAC-запрос: {e}"))?;

    // Читаем файл-результат.
    let result = std::fs::read_to_string(&result_path).unwrap_or_default();
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&result_path);

    if result.trim() == "OK" {
        Ok(())
    } else if let Some(err) = result.trim().strip_prefix("ERR:") {
        Err(format!("Файрвол: {err}"))
    } else if !status.success() {
        // Пользователь мог нажать «Нет» в UAC → ShellExecute вернёт ошибку.
        Err("Запрос прав администратора отклонён или не выполнен".into())
    } else {
        Err("Не удалось подтвердить применение правил файрвола".into())
    }
}

fn apply_no_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = cmd;
    }
}
