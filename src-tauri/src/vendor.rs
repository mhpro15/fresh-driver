//! Vendor-specific driver checks that go beyond what Windows Update offers.
//!
//! Windows Update lags well behind vendors for GPUs in particular, so we detect
//! the GPU directly and report its current driver. The "latest available"
//! lookup against the vendor is wired in per-vendor (NVIDIA first).

use serde::{Deserialize, Serialize};

use crate::ps;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VendorDriverStatus {
    pub vendor: String,
    pub device_name: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub download_url: Option<String>,
    /// Human-readable status, e.g. "Up to date", "Update available", or a note
    /// explaining why we couldn't determine the latest version automatically.
    pub status: String,
}

/// Detect NVIDIA GPU(s) and report current driver version.
///
/// The WMI `DriverVersion` for NVIDIA is the internal form (e.g. `32.0.15.7283`).
/// We convert it to the marketing version users recognise (e.g. `572.83`) by
/// taking the last five digits: `57283` -> `572.83`.
const NVIDIA_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }

function Get-Marketing($wmiVer) {
  if (-not $wmiVer) { return $null }
  $digits = ($wmiVer -replace '\.', '')
  if ($digits.Length -ge 5) {
    $l = $digits.Substring($digits.Length - 5)
    return $l.Substring(0,3) + '.' + $l.Substring(3,2)
  }
  return $null
}

$build = [int](Get-CimInstance Win32_OperatingSystem).BuildNumber
$osid = if ($build -ge 22000) { 135 } else { 57 }   # Windows 11 / Windows 10 64-bit

# GPU name -> NVIDIA product id (best-effort; community-maintained map).
$gpuMap = $null
try { $gpuMap = Invoke-RestMethod 'https://raw.githubusercontent.com/ZenitH-AT/nvidia-data/main/gpu-data.json' -TimeoutSec 20 -UserAgent 'Mozilla/5.0' } catch { }

$gpus = Get-CimInstance Win32_VideoController | Where-Object { $_.Name -match 'NVIDIA' }
$items = foreach ($g in $gpus) {
  $current = Get-Marketing $g.DriverVersion
  $latest = $null; $url = $null
  $status = 'Current version detected.'

  $name = ($g.Name -replace '^NVIDIA\s+', '').Trim()
  $pfid = $null
  if ($gpuMap) {
    $pfid = $gpuMap.desktop.$name
    if (-not $pfid) { $pfid = $gpuMap.notebook.$name }
  }
  if ($pfid) {
    try {
      $u = "https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php?func=DriverManualLookup&pfid=$pfid&osID=$osid&languageCode=1033&dch=1&upCRD=0&numberOfResults=1"
      $r = Invoke-RestMethod -Uri $u -TimeoutSec 30 -UserAgent 'Mozilla/5.0'
      if ($r.IDS) {
        $latest = $r.IDS[0].downloadInfo.Version
        $url = $r.IDS[0].downloadInfo.DownloadURL
      }
    } catch { }
  }

  if ($latest) {
    $newer = $false
    try { $newer = ([version]$latest -gt [version]$current) } catch { $newer = ($latest -ne $current) }
    if ($newer) { $status = "Update available - latest Game Ready driver is $latest." }
    else { $status = 'Up to date.' }
  } else {
    $url = 'https://www.nvidia.com/Download/index.aspx'
    $status = 'Could not reach NVIDIA - open the download page to check.'
  }

  [pscustomobject]@{
    vendor          = 'NVIDIA'
    device_name     = $g.Name
    current_version = $current
    latest_version  = $latest
    download_url    = $url
    status          = $status
  }
}
ConvertTo-Json -Depth 4 -InputObject @($items)
"#;

/// Return vendor driver status for supported vendors (currently NVIDIA).
pub fn scan() -> Result<Vec<VendorDriverStatus>, String> {
    let stdout = ps::run_powershell(NVIDIA_SCRIPT)?;
    ps::parse_json_array(&stdout)
}
