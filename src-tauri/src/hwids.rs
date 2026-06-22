//! Hardware-ID → human-readable names, from the bundled pci.ids database
//! (pciutils, BSD-3-licensed). Turns a raw `PCI\VEN_10EC&DEV_8168&…` string into
//! "Realtek Semiconductor Co., Ltd." + "RTL8111/8168/8411 …".

use std::collections::HashMap;
use std::sync::OnceLock;

const PCI_IDS: &str = include_str!("../data/pci.ids");

struct PciDb {
    vendors: HashMap<u16, String>,
    devices: HashMap<(u16, u16), String>,
}

fn db() -> &'static PciDb {
    static DB: OnceLock<PciDb> = OnceLock::new();
    DB.get_or_init(parse)
}

/// Parse a "ID␠␠name" line into (id, name). Returns None if the id isn't 16-bit hex.
fn split_id_name(s: &str) -> Option<(u16, String)> {
    let (id_str, name) = s.trim_end().split_once("  ")?;
    let id = u16::from_str_radix(id_str.trim(), 16).ok()?;
    Some((id, name.trim().to_string()))
}

fn parse() -> PciDb {
    let mut vendors = HashMap::new();
    let mut devices = HashMap::new();
    let mut cur_vendor: Option<u16> = None;

    for line in PCI_IDS.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("\t\t") {
            // subsystem line — ignore
            continue;
        } else if let Some(rest) = line.strip_prefix('\t') {
            // device line under the current vendor
            if let (Some(v), Some((id, name))) = (cur_vendor, split_id_name(rest)) {
                devices.insert((v, id), name);
            }
        } else {
            // top-level: a vendor line, or the "C nn  class" section (skip those)
            if line.starts_with('C') && !line.chars().next().map(|c| c.is_ascii_hexdigit()).unwrap_or(false) {
                cur_vendor = None;
                continue;
            }
            match split_id_name(line) {
                Some((id, name)) => {
                    cur_vendor = Some(id);
                    vendors.insert(id, name);
                }
                None => cur_vendor = None,
            }
        }
    }
    PciDb { vendors, devices }
}

#[derive(Default)]
pub struct HwName {
    pub vendor: Option<String>,
    pub device: Option<String>,
}

fn extract_hex(s: &str, key: &str) -> Option<u16> {
    let pos = s.find(key)? + key.len();
    let hex: String = s[pos..].chars().take(4).collect();
    u16::from_str_radix(&hex, 16).ok()
}

/// Resolve vendor + device names from a hardware id such as
/// `PCI\VEN_10EC&DEV_8168&SUBSYS_…`. Non-PCI ids simply return empty.
pub fn lookup_from_hwid(hwid: &str) -> HwName {
    let up = hwid.to_uppercase();
    let ven = extract_hex(&up, "VEN_");
    let dev = extract_hex(&up, "DEV_");
    let d = db();
    HwName {
        vendor: ven.and_then(|v| d.vendors.get(&v).cloned()),
        device: match (ven, dev) {
            (Some(v), Some(de)) => d.devices.get(&(v, de)).cloned(),
            _ => None,
        },
    }
}
