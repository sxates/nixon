//! `backup.json` in the app data dir: the scheduled-backup settings and their last results
//! (specs/0092, "Scheduled backups").

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::replace::write_atomic;
use super::BackupError;

pub const SETTINGS_FILE: &str = "backup.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupSettings {
    pub enabled: bool,
    pub folder: Option<PathBuf>,
    pub include_audio: bool,
    /// RFC 3339 with the local offset.
    pub last_success_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub last_error: Option<String>,
    /// Hex SHA-256 of the logical content at the last success.
    pub last_fingerprint: Option<String>,
    pub last_size: Option<u64>,
}

impl BackupSettings {
    /// The two switches on the settings screen.
    pub fn apply_ui(&mut self, enabled: bool, include_audio: bool) {
        self.enabled = enabled;
        self.include_audio = include_audio;
    }

    /// Choose the backup folder (`None` = the recordings folder). A different folder forgets
    /// the last backup, so the next run writes a fresh one there instead of skipping as
    /// "unchanged".
    pub fn set_folder(&mut self, folder: Option<String>) {
        let folder = folder.filter(|f| !f.trim().is_empty()).map(PathBuf::from);
        if folder != self.folder {
            self.last_fingerprint = None;
            self.last_success_at = None;
        }
        self.folder = folder;
    }

    /// The folder backups go to: the one chosen, else `default_folder` (the recordings folder).
    pub fn folder_or(&self, default_folder: &Path) -> PathBuf {
        self.folder.clone().unwrap_or_else(|| default_folder.to_path_buf())
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
