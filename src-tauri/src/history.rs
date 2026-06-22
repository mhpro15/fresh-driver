//! Persistent record of drivers installed via the catalog, so they can be
//! rolled back later (even after restarting the app). Stored as JSON under
//! `%LOCALAPPDATA%\fresh-driver\install-history.json`.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistoryEntry {
    pub device_name: String,
    pub version: Option<String>,
    /// Driver-store package names (`oemNN.inf`) — the rollback handles.
    pub published_names: Vec<String>,
    pub installed_at_unix: u64,
}

fn history_path() -> std::path::PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("fresh-driver").join("install-history.json")
}

/// Load the install history (newest first). Returns empty on any error.
pub fn load() -> Vec<HistoryEntry> {
    match std::fs::read_to_string(history_path()) {
        Ok(c) => serde_json::from_str(c.trim_start_matches('\u{feff}').trim()).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save(entries: &[HistoryEntry]) {
    let path = history_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(entries) {
        let _ = std::fs::write(&path, json);
    }
}

/// Record an install at the front of the history (most recent first).
pub fn add(entry: HistoryEntry) {
    let mut entries = load();
    // Replace any prior entry that shares a published name.
    entries.retain(|e| {
        !e.published_names
            .iter()
            .any(|p| entry.published_names.contains(p))
    });
    entries.insert(0, entry);
    entries.truncate(50);
    save(&entries);
}

/// Drop every history entry that contains the given published name.
pub fn remove(published_name: &str) {
    let mut entries = load();
    entries.retain(|e| !e.published_names.iter().any(|p| p == published_name));
    save(&entries);
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
