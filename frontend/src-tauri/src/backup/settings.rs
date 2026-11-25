//! `backup.json` in the app data dir: the scheduled-backup settings and their last results
//! (specs/0092, "Scheduled backups").

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::replace::write_atomic;
use super::BackupError;

pub const SETTINGS_FILE: &str = "backup.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupSettings {
    pub enabled: bool,
    pub folder: Option<PathBuf>,
    pub include_audio: bool,
    pub keep: u32,
    /// RFC 3339 with the local offset.
    pub last_success_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub last_error: Option<String>,
    /// Hex SHA-256 of the logical content at the last success.
    pub last_fingerprint: Option<String>,
    pub last_size: Option<u64>,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            folder: None,
            include_audio: false,
            keep: 7,
            last_success_at: None,
            last_attempt_at: None,
            last_error: None,
            last_fingerprint: None,
            last_size: None,
        }
    }
}

impl BackupSettings {
    /// What the settings screen controls. A different folder forgets the last backup, so the
    /// next run writes a fresh one there instead of skipping as "unchanged".
    pub fn apply_ui(&mut self, enabled: bool, folder: Option<String>, include_audio: bool) {
        let folder = folder.filter(|f| !f.trim().is_empty()).map(PathBuf::from);
        if folder != self.folder {
            self.last_fingerprint = None;
            self.last_success_at = None;
        }
        self.enabled = enabled;
        self.folder = folder;
        self.include_audio = include_audio;
    }
}

/// Read `backup.json`; missing or unreadable means defaults.
pub fn load(app_data_dir: &Path) -> BackupSettings {
    std::fs::read(app_data_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Serializes every load-modify-save of `backup.json` (UI changes and scheduler results).
static UPDATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Load, change and save `backup.json` under the process-wide lock; returns the saved value.
pub fn update(
    app_data_dir: &Path,
    f: impl FnOnce(&mut BackupSettings),
) -> Result<BackupSettings, BackupError> {
    let _guard = UPDATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = load(app_data_dir);
    f(&mut s);
    save(app_data_dir, &s)?;
    Ok(s)
}

pub fn save(app_data_dir: &Path, settings: &BackupSettings) -> Result<(), BackupError> {
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| BackupError::Io(format!("backup settings: {e}")))?;
    write_atomic(&app_data_dir.join(SETTINGS_FILE), &json)
}
