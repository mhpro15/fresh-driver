//! Manufacturer-gated OEM driver source (Dell / HP / Lenovo).
//!
//! Lenovo resolves tool-free from the public `catalogv2.xml`; Dell and HP use
//! their own tools (`dcu-cli` / HP CMSL) when installed; every supported brand
//! always gets an official support link. Non-OEM machines yield `brand = None`.

use serde::{Deserialize, Serialize};

use crate::ps;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OemUpdate {
    pub name: String,
    pub category: Option<String>,
    pub available_version: Option<String>,
    pub current_version: Option<String>,
    pub download_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OemResult {
    pub brand: Option<String>,
    pub note: Option<String>,
    pub support_url: Option<String>,
    #[serde(default)]
    pub updates: Vec<OemUpdate>,
}

const OEM_SCRIPT: &str = include_str!("../scripts/oem_scan.ps1");

/// Detect the PC manufacturer and return any OEM-official driver updates/links.
pub fn scan() -> Result<OemResult, String> {
    let stdout = ps::run_powershell(OEM_SCRIPT)?;
    ps::parse_json(&stdout)
}
