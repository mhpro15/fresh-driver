//! Enumerate every signed driver currently installed on the machine.

use serde::{Deserialize, Serialize};

use crate::ps;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DriverInfo {
    pub device_name: String,
    pub manufacturer: Option<String>,
    pub device_class: Option<String>,
    pub driver_version: Option<String>,
    pub driver_date: Option<String>,
    pub hardware_id: Option<String>,
    pub device_id: String,
    pub inf_name: Option<String>,
    /// Canonical chip-vendor name from the pci.ids database (enrichment).
    #[serde(default)]
    pub vendor_name: Option<String>,
    /// Canonical chip/device name from pci.ids (enrichment).
    #[serde(default)]
    pub chip_name: Option<String>,
}

const SCAN_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$drivers = Get-CimInstance Win32_PnPSignedDriver |
  Where-Object { $_.DeviceName } |
  ForEach-Object {
    [pscustomobject]@{
      device_name    = $_.DeviceName
      manufacturer   = $_.Manufacturer
      device_class   = $_.DeviceClass
      driver_version = $_.DriverVersion
      driver_date    = if ($_.DriverDate) { $_.DriverDate.ToString('yyyy-MM-dd') } else { $null }
      hardware_id    = if ($_.HardwareID) { [string](@($_.HardwareID)[0]) } else { $null }
      device_id      = $_.DeviceID
      inf_name       = $_.InfName
    }
  }
ConvertTo-Json -Depth 4 -InputObject @($drivers)
"#;

/// Return all installed signed drivers, sorted by device class then name.
pub fn scan() -> Result<Vec<DriverInfo>, String> {
    let stdout = ps::run_powershell(SCAN_SCRIPT)?;
    let mut drivers: Vec<DriverInfo> = ps::parse_json_array(&stdout)?;
    // Enrich each device with canonical names from the pci.ids database.
    for d in &mut drivers {
        if let Some(hwid) = &d.hardware_id {
            let names = crate::hwids::lookup_from_hwid(hwid);
            d.vendor_name = names.vendor;
            d.chip_name = names.device;
        }
    }
    drivers.sort_by(|a, b| {
        let ca = a.device_class.as_deref().unwrap_or("");
        let cb = b.device_class.as_deref().unwrap_or("");
        ca.cmp(cb).then_with(|| a.device_name.cmp(&b.device_name))
    });
    Ok(drivers)
}
