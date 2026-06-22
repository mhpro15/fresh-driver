// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // When re-launched elevated for a privileged operation, do it and exit
    // without starting the GUI.
    match args.get(1).map(String::as_str) {
        Some("--elevated-install") => {
            tauri_app_lib::elevated_install_entry(&args);
            return;
        }
        Some("--elevated-rollback") => {
            tauri_app_lib::elevated_rollback_entry(&args);
            return;
        }
        _ => {}
    }
    tauri_app_lib::run()
}
