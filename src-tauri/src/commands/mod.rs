//! Tauri-команды, разнесённые по темам. Все пункты переэкспортируются
//! отсюда: `commands::имя` в lib.rs и между модулями (через `super::*`).
//!
//! ВАЖНО: синхронные #[tauri::command] выполняются в ГЛАВНОМ потоке окна.
//! Команды, спавнящие процессы/сканирующие систему, — async + spawn_blocking.

mod setup;
mod control;
mod apps;
mod killswitch;
mod env;
mod bridge;
mod autostart;
mod wizard;
mod proxies;
mod windows;
mod sites;

pub(crate) use setup::*;
pub(crate) use sites::*;
pub(crate) use control::*;
pub(crate) use apps::*;
pub(crate) use killswitch::*;
pub(crate) use env::*;
pub(crate) use bridge::*;
pub(crate) use autostart::*;
pub(crate) use wizard::*;
pub(crate) use proxies::*;
pub(crate) use windows::*;
