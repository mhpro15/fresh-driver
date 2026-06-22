//! Safety net: create a System Restore point before touching drivers, so a bad
//! install can be rolled back.

use serde::{Deserialize, Serialize};

use crate::ps;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RestorePointResult {
    pub success: bool,
    pub message: String,
}

/// Create a System Restore point (elevated). Note: Windows throttles restore
/// points to one per 24h by default, and System Protection must be enabled on
/// the system drive — both are surfaced in the returned message.
pub fn create_restore_point(description: &str) -> Result<RestorePointResult, String> {
    let safe_desc = description.replace('\'', "''");
    let inner = format!(
        r#"
$ErrorActionPreference = 'Stop'
try {{
  # Bypass the default 24h throttle so a point is actually created on demand.
  try {{
    New-ItemProperty -Path 'HKLM:\Software\Microsoft\Windows NT\CurrentVersion\SystemRestore' `
      -Name 'SystemRestorePointCreationFrequency' -Value 0 -PropertyType DWord -Force | Out-Null
  }} catch {{}}

  Checkpoint-Computer -Description '{safe_desc}' -RestorePointType 'DEVICE_DRIVER_INSTALL'
  $out = [pscustomobject]@{{ success = $true; message = 'Restore point created.' }}
  $out | ConvertTo-Json | Set-Content -Path $ResultFile -Encoding UTF8
}} catch {{
  $msg = $_.Exception.Message
  $out = [pscustomobject]@{{ success = $false; message = "Could not create restore point: $msg (System Protection may be disabled on C:)." }}
  $out | ConvertTo-Json | Set-Content -Path $ResultFile -Encoding UTF8
}}
"#
    );

    let stdout = ps::run_powershell_elevated(&inner)?;
    ps::parse_json(&stdout)
}
