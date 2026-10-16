//! Killswitch на macOS не реализован.
//!
//! Windows Firewall блокирует исходящий трафик по пути процесса; встроенный
//! pf на macOS так не умеет (правила — по адресам/портам/пользователю), а
//! application firewall фильтрует только ВХОДЯЩИЕ. Честный per-app блок
//! требует Network Extension с подписью Apple — это отдельный проект.
//! API совпадает с firewall.rs, чтобы команды и UI не ветвились.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct KillswitchStatus {
    pub active: bool,
    pub rule_count: usize,
    pub blocked_processes: Vec<String>,
}

pub fn status() -> KillswitchStatus {
    KillswitchStatus {
        active: false,
        rule_count: 0,
        blocked_processes: Vec::new(),
    }
}

pub fn enable(_program_paths: &[String]) -> Result<(), String> {
    Err("Killswitch на macOS пока недоступен: системный файрвол не умеет блокировать трафик отдельных приложений".into())
}

pub fn disable() -> Result<(), String> {
    Ok(())
}
