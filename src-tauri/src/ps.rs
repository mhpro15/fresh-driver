//! Helper for running PowerShell scripts from Rust and parsing their JSON output.
//!
//! We shell out to PowerShell because the underlying commands (WMI driver
//! enumeration, the Windows Update Agent COM API, System Restore, vendor
//! installers) are already battle-tested there, and this keeps the Rust side
//! free of fragile COM bindings. Scripts are written to a temp `.ps1` file and
//! executed with `-File` to sidestep all command-line quoting problems.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Don't pop up a console window when spawning powershell.exe (Windows only).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `script` to a uniquely-named temp `.ps1` file and return its path.
///
/// The file is written with a UTF-8 BOM: Windows PowerShell 5.1 otherwise reads
/// a BOM-less `.ps1` as ANSI (Windows-1252), which mangles non-ASCII characters
/// like `…` in string literals into mojibake (`â€¦`).
fn write_temp_script(script: &str) -> Result<std::path::PathBuf, String> {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!("fresh-driver-{}-{}.ps1", std::process::id(), n));
    let mut bytes = Vec::with_capacity(script.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]); // UTF-8 BOM
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(&path, &bytes).map_err(|e| format!("failed to write temp script: {e}"))?;
    Ok(path)
}

/// Run a PowerShell script and return its stdout as a string.
pub fn run_powershell(script: &str) -> Result<String, String> {
    let path = write_temp_script(script)?;

    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ])
    .arg(&path);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd.output();
    let _ = std::fs::remove_file(&path); // best-effort cleanup

    let output = output.map_err(|e| format!("failed to launch PowerShell: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "PowerShell exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse PowerShell `ConvertTo-Json` output into a `Vec<T>`.
///
/// PowerShell is inconsistent: zero items can produce empty output, a single
/// item produces a bare object (not an array), and multiple items produce an
/// array. We normalise all three cases here so callers always get a `Vec`.
pub fn parse_json_array<T: DeserializeOwned>(stdout: &str) -> Result<Vec<T>, String> {
    // PowerShell's `Set-Content -Encoding UTF8` (5.1) and console output can emit
    // a leading BOM, which serde_json rejects and `trim()` does not remove.
    let trimmed = stdout.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let value: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| format!("failed to parse JSON ({e}); raw output: {trimmed}"))?;

    match value {
        serde_json::Value::Array(_) => {
            serde_json::from_value(value).map_err(|e| format!("failed to deserialize array: {e}"))
        }
        serde_json::Value::Null => Ok(Vec::new()),
        other => {
            let single: T = serde_json::from_value(other)
                .map_err(|e| format!("failed to deserialize object: {e}"))?;
            Ok(vec![single])
        }
    }
}

/// Parse PowerShell `ConvertTo-Json` output into a single `T`.
pub fn parse_json<T: DeserializeOwned>(stdout: &str) -> Result<T, String> {
    let trimmed = stdout.trim_start_matches('\u{feff}').trim();
    serde_json::from_str(trimmed)
        .map_err(|e| format!("failed to parse JSON ({e}); raw output: {trimmed}"))
}

/// A path inside the system temp dir (used for the shared install result /
/// progress files between the normal app and its elevated instance).
pub fn temp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(name);
    p
}

/// Re-launch *this* executable elevated (UAC prompt shows the app, not
/// PowerShell) with the given arguments, and wait for it to finish.
///
/// A non-elevated PowerShell `Start-Process -Verb RunAs` is used purely as the
/// launcher — it shows no window and triggers no UAC itself; the consent prompt
/// is for our own exe. Returns Ok once the elevated instance exits.
pub fn run_self_elevated(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find own exe: {e}"))?;
    let exe_s = exe.to_string_lossy().replace('\'', "''");
    let arg_list = args
        .iter()
        .map(|a| format!("'{}'", a.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    let launcher = format!(
        "$ErrorActionPreference='Stop'\n\
         $p = Start-Process -FilePath '{exe_s}' -Verb RunAs -WindowStyle Hidden -PassThru -Wait -ArgumentList {arg_list}\n\
         exit $p.ExitCode\n"
    );
    run_powershell(&launcher).map(|_| ())
}

/// Run a PowerShell script body **elevated** (triggers a UAC prompt).
///
/// We elevate per-operation rather than running the whole app as admin. The
/// `inner_body` script is given one argument: the path of a result file it
/// should write its JSON outcome to. We launch it via `Start-Process -Verb
/// RunAs -Wait` (which shows the UAC dialog), wait for it to finish, then read
/// the result file back. This is the only way to capture output across the
/// elevation boundary.
pub fn run_powershell_elevated(inner_body: &str) -> Result<String, String> {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut result_path = std::env::temp_dir();
    result_path.push(format!("fresh-driver-result-{}-{}.json", std::process::id(), n));

    // The inner script writes its JSON outcome to $args[0] (the result file).
    let inner = format!(
        "$ErrorActionPreference='Stop'\n$ResultFile = $args[0]\n{inner_body}\n"
    );
    let inner_path = write_temp_script(&inner)?;

    // The launcher (runs unelevated) starts the inner script elevated and waits.
    let inner_path_str = inner_path.to_string_lossy().replace('\'', "''");
    let result_path_str = result_path.to_string_lossy().replace('\'', "''");
    let launcher = format!(
        "$ErrorActionPreference='Stop'\n\
         $p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden \
         -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{inner_path_str}','{result_path_str}'\n\
         exit $p.ExitCode\n"
    );

    let run = run_powershell(&launcher);
    let _ = std::fs::remove_file(&inner_path);

    // Read whatever the elevated process wrote, even if it reported an error,
    // so the caller gets a structured outcome.
    let contents = std::fs::read_to_string(&result_path).ok();
    let _ = std::fs::remove_file(&result_path);

    match (run, contents) {
        (Ok(_), Some(c)) => Ok(c),
        (Ok(_), None) => Err("elevated operation produced no result (it may have been cancelled at the UAC prompt)".into()),
        (Err(e), Some(c)) => {
            // Process reported nonzero but still left a result; prefer the result.
            if c.trim().is_empty() {
                Err(format!("elevated operation failed: {e}"))
            } else {
                Ok(c)
            }
        }
        (Err(e), None) => Err(format!("elevation failed or was cancelled: {e}")),
    }
}
