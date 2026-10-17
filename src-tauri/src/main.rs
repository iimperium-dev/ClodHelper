// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = clod_helper_lib::cli_mode() {
        std::process::exit(code);
    }
    clod_helper_lib::run()
}
