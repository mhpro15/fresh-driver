//! Query and install driver updates via the Windows Update Agent (WUA) COM API.
//!
//! Scanning is read-only and runs unelevated. Installing requires admin, so it
//! is routed through `ps::run_powershell_elevated` (one UAC prompt per install).

use serde::{Deserialize, Serialize};

use crate::ps;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateInfo {
    pub update_id: String,
    pub title: String,
    pub driver_class: Option<String>,
    pub driver_model: Option<String>,
    pub driver_manufacturer: Option<String>,
    pub driver_ver_date: Option<String>,
    pub is_downloaded: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstallResult {
    pub success: bool,
    pub reboot_required: bool,
    pub message: String,
    /// Published driver-store names (`oemNN.inf`) registered by this install —
    /// the handles used to roll the driver back. Empty for Windows Update installs.
    #[serde(default)]
    pub published_names: Vec<String>,
}

const SCAN_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$session = New-Object -ComObject Microsoft.Update.Session
$searcher = $session.CreateUpdateSearcher()
$searcher.ServerSelection = 2   # ssWindowsUpdate
$result = $searcher.Search("IsInstalled=0 and Type='Driver'")
$items = for ($i = 0; $i -lt $result.Updates.Count; $i++) {
  $u = $result.Updates.Item($i)
  [pscustomobject]@{
    update_id           = $u.Identity.UpdateID
    title               = $u.Title
    driver_class        = $(try { $u.DriverClass } catch { $null })
    driver_model        = $(try { $u.DriverModel } catch { $null })
    driver_manufacturer = $(try { $u.DriverManufacturer } catch { $null })
    driver_ver_date     = $(try { if ($u.DriverVerDate) { $u.DriverVerDate.ToString('yyyy-MM-dd') } else { $null } } catch { $null })
    is_downloaded       = [bool]$u.IsDownloaded
  }
}
ConvertTo-Json -Depth 4 -InputObject @($items)
"#;

/// Search Windows Update for applicable (not-yet-installed) driver updates.
pub fn scan() -> Result<Vec<UpdateInfo>, String> {
    let stdout = ps::run_powershell(SCAN_SCRIPT)?;
    ps::parse_json_array(&stdout)
}

/// Download and install a single driver update by its UpdateID (elevated).
pub fn install(update_id: &str) -> Result<InstallResult, String> {
    // Guard against script injection via the update id (it's a GUID).
    if !update_id
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c == '-')
        || update_id.is_empty()
    {
        return Err("invalid update id".into());
    }

    let inner = format!(
        r#"
$ErrorActionPreference = 'Stop'
try {{
  $session = New-Object -ComObject Microsoft.Update.Session
  $searcher = $session.CreateUpdateSearcher()
  $searcher.ServerSelection = 2
  $result = $searcher.Search("UpdateID='{update_id}' and IsInstalled=0")
  if ($result.Updates.Count -eq 0) {{
    $out = [pscustomobject]@{{ success = $false; reboot_required = $false; message = 'Update not found or already installed.' }}
    $out | ConvertTo-Json | Set-Content -Path $ResultFile -Encoding UTF8
    exit 0
  }}
  $toInstall = New-Object -ComObject Microsoft.Update.UpdateColl
  $toInstall.Add($result.Updates.Item(0)) | Out-Null

  $downloader = $session.CreateUpdateDownloader()
  $downloader.Updates = $toInstall
  $downloader.Download() | Out-Null

  $installer = $session.CreateUpdateInstaller()
  $installer.Updates = $toInstall
  $installResult = $installer.Install()

  $ok = ($installResult.ResultCode -eq 2)   # orcSucceeded
  $out = [pscustomobject]@{{
    success = $ok
    reboot_required = [bool]$installResult.RebootRequired
    message = if ($ok) {{ 'Driver installed successfully.' }} else {{ "Install finished with result code $($installResult.ResultCode)." }}
  }}
  $out | ConvertTo-Json | Set-Content -Path $ResultFile -Encoding UTF8
}} catch {{
  $out = [pscustomobject]@{{ success = $false; reboot_required = $false; message = "Error: $($_.Exception.Message)" }}
  $out | ConvertTo-Json | Set-Content -Path $ResultFile -Encoding UTF8
}}
"#
    );

    let stdout = ps::run_powershell_elevated(&inner)?;
    ps::parse_json(&stdout)
}
