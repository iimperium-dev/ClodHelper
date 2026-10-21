//! Windows: действие с правами администратора — ровно одно окно UAC.
//!
//! Скрипты операции пишутся в свежую папку настроек Helper
//! (%APPDATA%\Clod Helper\<операция>-<pid>-<nonce>), а не в общий %TEMP%:
//! «скрытый elevated powershell из темпа» — хрестоматийная AV-эвристика, а
//! чистильщики темпа могут удалить файл между записью и запуском. Elevated
//! `powershell -File <главный скрипт>` пишет итог (JSON) в result.json рядом:
//! stdout elevated-процесса нам недоступен. Папка удаляется после операции.
//!
//! Итогу на слово не верим: каждый вызывающий перечитывает фактическое
//! состояние (реестр, правила брандмауэра) без прав администратора.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Итог elevated-операции.
pub struct Outcome {
    /// Пользователь отказал в окне UAC.
    pub cancelled: bool,
    /// Elevated-процесс завершился с кодом 0.
    pub success: bool,
    /// result.json, если скрипт успел его записать (в т.ч. с ошибкой).
    pub result: Option<String>,
}

/// Windows PowerShell 5.1 надёжно читает UTF-8 скрипты (русские тексты)
/// только с BOM.
pub fn with_bom(script: &str) -> String {
    format!("\u{feff}{}", script.trim_start_matches('\u{feff}'))
}

/// Записать `scripts` (имя файла, текст) в свежую папку операции и запустить
/// ПЕРВЫЙ с правами администратора:
/// `-File <первый> <arguments> -ResultPath <папка>\result.json`.
/// `arguments` собирает вызывающий из проверенных значений.
pub fn run(operation: &str, scripts: &[(&str, &str)], arguments: &str) -> Result<Outcome, String> {
    let (entry, _) = scripts
        .first()
        .ok_or_else(|| "Нет скрипта операции".to_string())?;
    let dir = OperationDir::create(operation)?;
    for (name, text) in scripts {
        std::fs::write(dir.path.join(name), with_bom(text))
            .map_err(|e| format!("Не удалось подготовить операцию: {e}"))?;
    }
    let result = dir.path.join("result.json");
    let output = powershell()
        .arg("-Command")
        .arg(wrapper(&dir.path.join(entry), arguments, &result))
        .output()
        .map_err(|e| format!("Не удалось запросить права администратора: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(Outcome {
        cancelled: stdout.lines().any(|l| l.trim() == "CLOD_UAC_CANCELLED"),
        success: output.status.success(),
        result: std::fs::read_to_string(&result).ok(),
    })
}

/// Обёртка без прав: поднимает скрытый elevated powershell и ждёт его.
/// Отказ в окне UAC (ERROR_CANCELLED, 1223) отличаем от прочих сбоев.
fn wrapper(script: &Path, arguments: &str, result: &Path) -> String {
    // В путях Windows не бывает кавычек: пути — в двойных кавычках внутри
    // командной строки, апострофы экранируются для PowerShell-литерала.
    let arguments = format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\" {} -ResultPath \"{}\"",
        script.to_string_lossy(),
        arguments,
        result.to_string_lossy()
    )
    .replace('\'', "''");
    format!(
        "$ErrorActionPreference='Stop'; \
         try {{ \
           $p = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') \
             -Verb RunAs -WindowStyle Hidden -PassThru -Wait -ArgumentList '{arguments}'; \
           exit $p.ExitCode \
         }} catch {{ \
           $e = $_.Exception; \
           while ($null -ne $e) {{ \
             if ($e -is [System.ComponentModel.Win32Exception] -and $e.NativeErrorCode -eq 1223) {{ \
               Write-Output 'CLOD_UAC_CANCELLED'; exit 1 \
             }}; \
             $e = $e.InnerException \
           }}; \
           Write-Output 'CLOD_UAC_FAILED'; exit 1 \
         }}"
    )
}

/// Windows PowerShell без профиля и без окна (64-битный, из System32).
pub fn powershell() -> Command {
    let executable = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
        .unwrap_or_else(|| PathBuf::from("powershell.exe"));
    let mut command = Command::new(executable);
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
    ]);
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

/// Прочитать состояние скриптом без прав администратора: тело скрипта —
/// в `-Command`, без временных файлов. stdout — одна строка JSON.
pub fn read_status(script: &str, arguments: &str) -> Result<String, String> {
    let output = powershell()
        .arg("-Command")
        .arg(format!(
            "& {{\n{}\n}} {arguments}",
            script.trim_start_matches('\u{feff}')
        ))
        .output()
        .map_err(|e| e.to_string())?;
    let payload = String::from_utf8_lossy(&output.stdout).to_string();
    if payload.trim().is_empty() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(payload)
}

struct OperationDir {
    path: PathBuf,
}

impl OperationDir {
    fn create(operation: &str) -> Result<Self, String> {
        let parent = dirs::config_dir()
            .ok_or_else(|| "Не найдена папка настроек Helper".to_string())?
            .join("Clod Helper");
        std::fs::create_dir_all(&parent)
            .map_err(|e| format!("Не удалось создать папку операции: {e}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = parent.join(format!("{operation}-{}-{nonce}", std::process::id()));
        // Никогда не переиспользуем скрипт или итог прошлой операции.
        std::fs::create_dir(&path)
            .map_err(|e| format!("Не удалось создать папку операции: {e}"))?;
        Ok(Self { path })
    }
}

impl Drop for OperationDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uac_wrapper_quotes_paths_and_hides_child_window() {
        let wrapper = wrapper(
            Path::new("C:\\O'Brien\\policy.ps1"),
            "-Action Enable -Services claude,telegram",
            Path::new("C:\\A B\\result.json"),
        );
        assert!(wrapper.contains("-Verb RunAs -WindowStyle Hidden"));
        assert!(wrapper.contains("-File \"C:\\O''Brien\\policy.ps1\" -Action Enable -Services claude,telegram"));
        assert!(wrapper.contains("-ResultPath \"C:\\A B\\result.json\""));
        assert!(wrapper.contains("NativeErrorCode -eq 1223"));
        assert!(wrapper.contains("'CLOD_UAC_CANCELLED'"));
    }

    #[test]
    fn scripts_get_exactly_one_bom() {
        assert_eq!(with_bom("x"), "\u{feff}x");
        assert_eq!(with_bom("\u{feff}x"), "\u{feff}x");
    }
}
