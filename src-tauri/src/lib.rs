//! Fresh Driver — desktop driver manager.
//!
//! Backend command surface exposed to the web frontend. The heavy lifting lives
//! in the submodules; each command runs its (blocking) work on a background
//! thread so the UI stays responsive.

mod catalog;
mod drivers;
mod history;
mod hwids;
mod oem;
mod ps;
mod safety;
mod vendor;
mod wupdate;

use catalog::{InstallProgress, OnlineUpdate};
use drivers::DriverInfo;
use history::HistoryEntry;
use oem::OemResult;
use safety::RestorePointResult;
use vendor::VendorDriverStatus;
use wupdate::{InstallResult, UpdateInfo};

/// List every installed signed driver on the machine.
#[tauri::command]
async fn scan_drivers() -> Result<Vec<DriverInfo>, String> {
    tauri::async_runtime::spawn_blocking(drivers::scan)
        .await
        .map_err(|e| format!("scan task failed: {e}"))?
}

/// Search Windows Update for applicable driver updates.
#[tauri::command]
async fn check_windows_update() -> Result<Vec<UpdateInfo>, String> {
    tauri::async_runtime::spawn_blocking(wupdate::scan)
        .await
        .map_err(|e| format!("windows update task failed: {e}"))?
}

/// Check supported vendors (currently NVIDIA) for driver status.
#[tauri::command]
async fn scan_vendors() -> Result<Vec<VendorDriverStatus>, String> {
    tauri::async_runtime::spawn_blocking(vendor::scan)
        .await
        .map_err(|e| format!("vendor scan task failed: {e}"))?
}

/// Detect the PC manufacturer (Dell/HP/Lenovo) and return OEM-official updates.
#[tauri::command]
async fn scan_oem() -> Result<OemResult, String> {
    tauri::async_runtime::spawn_blocking(oem::scan)
        .await
        .map_err(|e| format!("oem scan task failed: {e}"))?
}

/// Search the Microsoft Update Catalog online for driver updates newer than
/// what's installed (the comprehensive source — broader than Windows Update).
#[tauri::command]
async fn check_online_updates() -> Result<Vec<OnlineUpdate>, String> {
    tauri::async_runtime::spawn_blocking(catalog::scan)
        .await
        .map_err(|e| format!("catalog scan task failed: {e}"))?
}

/// Download & install a catalog driver by UpdateID. Re-launches the app elevated
/// (UAC shows the app), optionally creating a restore point first. Records the
/// install so it can be rolled back.
#[tauri::command]
async fn install_catalog_update(
    update_id: String,
    create_restore: bool,
    device_name: String,
    version: String,
) -> Result<InstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        catalog::install_via_elevation(&update_id, create_restore, &device_name, &version)
    })
    .await
    .map_err(|e| format!("catalog install task failed: {e}"))?
}

/// Current progress of an in-flight catalog install (polled by the UI).
#[tauri::command]
fn get_install_progress() -> InstallProgress {
    catalog::read_progress()
}

/// Drivers installed via the catalog that can be rolled back (newest first).
#[tauri::command]
fn get_install_history() -> Vec<HistoryEntry> {
    history::load()
}

/// Roll back a previously-installed driver by its published name (prompts UAC).
#[tauri::command]
async fn rollback_driver(published_name: String) -> Result<InstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || catalog::rollback_via_elevation(&published_name))
        .await
        .map_err(|e| format!("rollback task failed: {e}"))?
}

/// Install a single Windows Update driver by its UpdateID (prompts for UAC).
#[tauri::command]
async fn install_update(update_id: String) -> Result<InstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || wupdate::install(&update_id))
        .await
        .map_err(|e| format!("install task failed: {e}"))?
}

/// Create a System Restore point before making changes (prompts for UAC).
#[tauri::command]
async fn create_restore_point() -> Result<RestorePointResult, String> {
    tauri::async_runtime::spawn_blocking(|| safety::create_restore_point("Fresh Driver - before driver update"))
        .await
        .map_err(|e| format!("restore point task failed: {e}"))?
}

/// Entry point for the elevated install helper. When the app is re-launched with
/// `--elevated-install <update_id> <0|1> <result_file> <progress_file>`, `main`
/// routes here instead of starting the GUI — this instance is already elevated.
pub fn elevated_install_entry(args: &[String]) {
    // args[0] = exe, args[1] = "--elevated-install", then the four parameters.
    let update_id = args.get(2).map(String::as_str).unwrap_or("");
    let create_restore = args.get(3).map(String::as_str) == Some("1");
    let result_file = args.get(4).map(String::as_str).unwrap_or("");
    let progress_file = args.get(5).map(String::as_str).unwrap_or("");
    catalog::run_elevated_worker(update_id, create_restore, result_file, progress_file);
}

/// Entry point for the elevated rollback helper:
/// `--elevated-rollback <oemNN.inf> <result_file>`.
pub fn elevated_rollback_entry(args: &[String]) {
    let published_name = args.get(2).map(String::as_str).unwrap_or("");
    let result_file = args.get(3).map(String::as_str).unwrap_or("");
    catalog::run_elevated_rollback_worker(published_name, result_file);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            scan_drivers,
            check_windows_update,
            check_online_updates,
            install_catalog_update,
            get_install_progress,
            get_install_history,
            rollback_driver,
            scan_vendors,
            scan_oem,
            install_update,
            create_restore_point,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
