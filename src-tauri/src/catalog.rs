//! Search the Microsoft Update Catalog (catalog.update.microsoft.com) for driver
//! updates newer than what's installed.
//!
//! This is the comprehensive "what's available online" source — far broader than
//! what the Windows Update Agent chooses to offer. The actual scan logic lives in
//! `scripts/catalog_scan.ps1` (enumerate devices → dedupe by VEN&DEV / VID&PID →
//! query the catalog concurrently via a runspace pool → keep only newer versions).

use serde::{Deserialize, Serialize};

use crate::ps;
use crate::wupdate::InstallResult;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OnlineUpdate {
    /// The hardware-id query used, e.g. `PCI\VEN_10EC&DEV_8168`.
    pub query: String,
    pub device_name: String,
    pub device_class: Option<String>,
    pub installed_version: Option<String>,
    pub available_version: Option<String>,
    pub available_date: Option<String>,
    /// Catalog UpdateID (GUID) used to download & install the package.
    pub update_id: Option<String>,
    pub catalog_url: Option<String>,
    pub found: bool,
}

const CATALOG_SCRIPT: &str = include_str!("../scripts/catalog_scan.ps1");
const INSTALL_TEMPLATE: &str = include_str!("../scripts/catalog_install.ps1");
const ROLLBACK_TEMPLATE: &str = include_str!("../scripts/catalog_rollback.ps1");

/// Scan the Microsoft Update Catalog for newer drivers. Takes ~10s (concurrent).
pub fn scan() -> Result<Vec<OnlineUpdate>, String> {
    let stdout = ps::run_powershell(CATALOG_SCRIPT)?;
    ps::parse_json_array(&stdout)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstallProgress {
    pub stage: String,
    pub percent: i32,
    pub message: String,
}

const RESULT_FILE: &str = "fresh-driver-install-result.json";
const PROGRESS_FILE: &str = "fresh-driver-install-progress.json";

fn is_guid(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// GUI side: install a catalog driver by re-launching this app elevated (so the
/// UAC prompt shows "Fresh Driver", not PowerShell) and waiting for it. Progress
/// is reported to a shared file polled via [`read_progress`].
pub fn install_via_elevation(
    update_id: &str,
    create_restore: bool,
    device_name: &str,
    version: &str,
) -> Result<InstallResult, String> {
    if !is_guid(update_id) {
        return Err("invalid update id".into());
    }
    let result_path = ps::temp_path(RESULT_FILE);
    let progress_path = ps::temp_path(PROGRESS_FILE);
    let _ = std::fs::remove_file(&result_path);
    let _ = std::fs::write(
        &progress_path,
        r#"{"stage":"start","percent":0,"message":"Requesting permission…"}"#,
    );

    let args = vec![
        "--elevated-install".to_string(),
        update_id.to_string(),
        if create_restore { "1" } else { "0" }.to_string(),
        result_path.to_string_lossy().to_string(),
        progress_path.to_string_lossy().to_string(),
    ];
    let elevated = ps::run_self_elevated(&args);

    // Read whatever the elevated instance wrote, regardless of its exit code.
    let contents = std::fs::read_to_string(&result_path).ok();
    let _ = std::fs::remove_file(&progress_path);
    let _ = std::fs::remove_file(&result_path);

    let parsed: Result<InstallResult, String> = match (elevated, contents) {
        (_, Some(c)) if !c.trim().is_empty() => ps::parse_json(&c),
        (Ok(_), _) => Err("The installer did not return a result.".into()),
        (Err(e), _) => {
            // A declined UAC prompt surfaces as "operation was canceled by the
            // user" — present that cleanly rather than as a PowerShell dump.
            if e.to_lowercase().contains("cancel") {
                Err("Installation cancelled.".into())
            } else {
                Err("Could not start the installer (administrator approval is required).".into())
            }
        }
    };

    // Record successful installs so they can be rolled back later.
    if let Ok(ref r) = parsed {
        if r.success && !r.published_names.is_empty() {
            crate::history::add(crate::history::HistoryEntry {
                device_name: device_name.to_string(),
                version: (!version.is_empty()).then(|| version.to_string()),
                published_names: r.published_names.clone(),
                installed_at_unix: crate::history::now_unix(),
            });
        }
    }
    parsed
}

fn is_oem_name(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.starts_with("oem")
        && lower.ends_with(".inf")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
}

/// Roll back a previously-installed driver by removing its driver-store package
/// (elevated). On success the device reverts to its previous driver.
pub fn rollback_via_elevation(published_name: &str) -> Result<InstallResult, String> {
    if !is_oem_name(published_name) {
        return Err("invalid driver name".into());
    }
    let result_path = ps::temp_path(RESULT_FILE);
    let _ = std::fs::remove_file(&result_path);

    let args = vec![
        "--elevated-rollback".to_string(),
        published_name.to_string(),
        result_path.to_string_lossy().to_string(),
    ];
    let elevated = ps::run_self_elevated(&args);
    let contents = std::fs::read_to_string(&result_path).ok();
    let _ = std::fs::remove_file(&result_path);

    let parsed: Result<InstallResult, String> = match (elevated, contents) {
        (_, Some(c)) if !c.trim().is_empty() => ps::parse_json(&c),
        (Ok(_), _) => Err("The rollback did not return a result.".into()),
        (Err(e), _) => {
            if e.to_lowercase().contains("cancel") {
                Err("Rollback cancelled.".into())
            } else {
                Err("Could not start the rollback (administrator approval is required).".into())
            }
        }
    };

    if let Ok(ref r) = parsed {
        if r.success {
            crate::history::remove(published_name);
        }
    }
    parsed
}

/// Elevated side: remove a driver-store package, writing the result file.
pub fn run_elevated_rollback_worker(published_name: &str, result_file: &str) {
    if !is_oem_name(published_name) {
        let _ = std::fs::write(
            result_file,
            r#"{"success":false,"reboot_required":false,"message":"invalid driver name","published_names":[]}"#,
        );
        return;
    }
    let script = ROLLBACK_TEMPLATE
        .replace("__OEM__", published_name)
        .replace("__RESULT_FILE__", &result_file.replace('\'', "''"));
    if let Err(e) = ps::run_powershell(&script) {
        let empty = std::fs::read_to_string(result_file)
            .map(|c| c.trim().is_empty())
            .unwrap_or(true);
        if empty {
            let msg = format!("rollback error: {e}").replace(['"', '\\'], "'");
            let _ = std::fs::write(
                result_file,
                format!("{{\"success\":false,\"reboot_required\":false,\"message\":\"{msg}\",\"published_names\":[]}}"),
            );
        }
    }
}

/// Elevated side: invoked from `main` when started with `--elevated-install`.
/// Runs the (already-elevated) install script, writing progress + result files.
pub fn run_elevated_worker(
    update_id: &str,
    create_restore: bool,
    result_file: &str,
    progress_file: &str,
) {
    if !is_guid(update_id) {
        let _ = std::fs::write(
            result_file,
            r#"{"success":false,"reboot_required":false,"message":"invalid update id"}"#,
        );
        return;
    }
    let script = INSTALL_TEMPLATE
        .replace("__UPDATE_ID__", update_id)
        .replace("__CREATE_RESTORE__", if create_restore { "1" } else { "0" })
        .replace("__RESULT_FILE__", &result_file.replace('\'', "''"))
        .replace("__PROGRESS_FILE__", &progress_file.replace('\'', "''"));

    // Already elevated → runs hidden with no additional UAC prompt.
    if let Err(e) = ps::run_powershell(&script) {
        let empty = std::fs::read_to_string(result_file)
            .map(|c| c.trim().is_empty())
            .unwrap_or(true);
        if empty {
            let msg = format!("installer error: {e}").replace(['"', '\\'], "'");
            let _ = std::fs::write(
                result_file,
                format!(
                    "{{\"success\":false,\"reboot_required\":false,\"message\":\"{msg}\"}}"
                ),
            );
        }
    }
}

/// Read the current install progress (polled by the UI during an install).
pub fn read_progress() -> InstallProgress {
    match std::fs::read_to_string(ps::temp_path(PROGRESS_FILE)) {
        Ok(c) => ps::parse_json::<InstallProgress>(&c).unwrap_or(InstallProgress {
            stage: "working".into(),
            percent: 0,
            message: "Working…".into(),
        }),
        Err(_) => InstallProgress {
            stage: "idle".into(),
            percent: 0,
            message: String::new(),
        },
    }
}
