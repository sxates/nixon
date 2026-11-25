//! Which files a backup carries (specs/0092): an allowlist of settings, and a meeting
//! folder's files minus crash-recovery checkpoints (and minus media unless audio is included).

use std::path::{Path, PathBuf};

use crate::audio::lifecycle::is_media_file;

/// Settings files (relative to the app data dir) that travel. Anything else stays home.
/// `templates/` (legacy) travels too, recursively; see [`settings_entries`].
pub const SETTINGS_ALLOWLIST: &[&str] = &[
    "preferences.json",
    "recording_preferences.json",
    "notifications.json",
    "zoom.json",
    "diarization.json",
    "updater.json",
    "ollama-context.json",
];

const TEMPLATES_DIR: &str = "templates";

/// Existing allowlisted settings files as `(source path, zip path under settings/)`.
pub fn settings_entries(app_data_dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = SETTINGS_ALLOWLIST
        .iter()
        .map(|name| (app_data_dir.join(name), (*name).to_string()))
        .filter(|(p, _)| p.is_file())
        .collect();
    collect_recursive(&app_data_dir.join(TEMPLATES_DIR), TEMPLATES_DIR, &mut out);
    out
}

fn collect_recursive(dir: &Path, prefix: &str, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let Ok(ft) = entry.file_type() else { continue };
        let zip_path = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        if ft.is_dir() {
            collect_recursive(&entry.path(), &zip_path, out);
        } else if ft.is_file() {
            out.push((entry.path(), zip_path));
        }
    }
}

/// The files of one meeting folder to archive (top level only; `.checkpoints/` is never
/// exported; media files only when `include_audio`). Sorted for stable archives.
pub fn folder_entries(folder: &Path, include_audio: bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| include_audio || !is_media_file(p))
        .collect();
    out.sort();
    out
}
