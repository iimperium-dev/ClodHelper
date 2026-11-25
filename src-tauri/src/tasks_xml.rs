//! Регистрация задачи моста через `schtasks /Create /XML`.
//!
//! Раньше задачу ставил Register-ScheduledTask, а запуск PowerShell на этой
//! машине стоит около 2 секунд. Регистрация идёт при каждой смене пула,
//! резервов, назначений и режима. XML ниже повторяет поле в поле и в том же
//! порядке файл задачи, который записывал PowerShell (сверено с
//! `%SystemRoot%\System32\Tasks\ClodBridge` и `schtasks /Query /XML`).
//! Если schtasks отказал - прежний путь через PowerShell: он и задачу
//! поставит, и вернёт понятный текст ошибки.

use std::path::PathBuf;

/// Пользователь в виде DOMAIN\user, как `$env:USERDOMAIN\$env:USERNAME`
/// у PowerShell-регистрации.
fn task_user() -> Option<String> {
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty())?;
    Some(match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => format!("{d}\\{user}"),
        _ => user,
    })
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// XML задачи. Настройки - как у New-ScheduledTaskSettingsSet
/// -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable
/// -RestartCount 999 -RestartInterval 1 мин -ExecutionTimeLimit 0
/// -MultipleInstances IgnoreNew; вход Interactive, права обычные.
pub(super) fn task_xml(user: &str, command: &str, arguments: &str) -> String {
    let user = esc(user);
    let command = esc(command);
    let arguments = esc(arguments);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo />
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <Duration>PT10M</Duration>
      <WaitTimeout>PT1H</WaitTimeout>
      <StopOnIdleEnd>true</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <DisallowStartOnRemoteAppSession>false</DisallowStartOnRemoteAppSession>
    <UseUnifiedSchedulingEngine>true</UseUnifiedSchedulingEngine>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
    .replace('\n', "\r\n")
}

/// Временный файл XML в UTF-16 LE с BOM: так schtasks читает кириллицу в
/// путях профиля без потерь. Удаляется при выходе из области видимости.
struct TempXml(PathBuf);

impl TempXml {
    fn write(xml: &str) -> Result<Self, String> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "clod-helper-task-{}-{}.xml",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let bytes: Vec<u8> = [0xFF_u8, 0xFE]
            .into_iter()
            .chain(xml.encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        std::fs::write(&path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for TempXml {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `schtasks /Create /XML`. Err - текст schtasks (для журнала).
fn register_via_schtasks(name: &str, command: &str, arguments: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let user = task_user().ok_or("нет имени пользователя в окружении")?;
    let file = TempXml::write(&task_xml(&user, command, arguments))?;
    let out = std::process::Command::new("schtasks")
        .arg("/Create")
        .arg("/TN")
        .arg(name)
        .arg("/XML")
        .arg(&file.0)
        .arg("/F")
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("schtasks не запустился: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let mut text = oem_to_string(&out.stderr);
    if text.trim().is_empty() {
        text = oem_to_string(&out.stdout);
    }
    Err(format!("schtasks {}: {}", out.status, text.trim()))
}

/// Вывод консольной программы в OEM-кодировке (cp866 на русской Windows).
fn oem_to_string(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP};
    if bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).to_string();
    }
    let mut wide = vec![0u16; bytes.len() * 2];
    // SAFETY: длины переданы явно, буфер с запасом.
    let n = unsafe {
        MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), bytes.len() as i32, wide.as_mut_ptr(), wide.len() as i32)
    };
    String::from_utf16_lossy(&wide[..n.max(0) as usize])
}

/// Прежняя регистрация через PowerShell - запасной путь.
fn register_via_powershell(name: &str, command: &str, arguments: &str) -> Result<(), String> {
    let q = |s: &str| s.replace('\'', "''");
    let script = format!(
        "$user = \"$env:USERDOMAIN\\$env:USERNAME\"; \
         $action = New-ScheduledTaskAction -Execute '{command}' -Argument '{arguments}'; \
         $trigger = New-ScheduledTaskTrigger -AtLogOn -User $user; \
         $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries \
           -StartWhenAvailable -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) \
           -ExecutionTimeLimit (New-TimeSpan) -MultipleInstances IgnoreNew; \
         $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited; \
         Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger \
           -Settings $settings -Principal $principal -Force | Out-Null; \
         Write-Output 'TASK_REGISTERED'",
        command = q(command),
        arguments = q(arguments),
        name = q(name),
    );
    let out = super::run_ps(&script)
        .ok_or_else(|| "powershell.exe не выполнил регистрацию задачи".to_string())?;
    if !out.contains("TASK_REGISTERED") {
        return Err(format!("Не удалось зарегистрировать задачу: {out}"));
    }
    Ok(())
}

/// Зарегистрировать (или заменить) задачу `name`, которая при входе
/// пользователя запускает `command arguments`.
pub(crate) fn register_task(name: &str, command: &str, arguments: &str) -> Result<(), String> {
    match register_via_schtasks(name, command, arguments) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!("регистрация задачи через schtasks не удалась ({e}), пробую PowerShell");
            register_via_powershell(name, command, arguments)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escapes_paths_and_args() {
        let xml = task_xml(r"PC\Мария & Co", r"C:\Users\Мария & Co\clod-bridge.exe", r#"--rules "C:\a<b>.json""#);
        assert!(xml.contains(r"<UserId>PC\Мария &amp; Co</UserId>"));
        assert!(xml.contains(r"<Command>C:\Users\Мария &amp; Co\clod-bridge.exe</Command>"));
        assert!(xml.contains(r"<Arguments>--rules &quot;C:\a&lt;b&gt;.json&quot;</Arguments>"));
        assert!(!xml.contains("\n\n") && xml.matches('\n').count() == xml.matches("\r\n").count());
    }
}
