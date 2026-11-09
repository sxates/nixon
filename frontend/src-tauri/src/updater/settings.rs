//! specs/0058 — the updater preferences, persisted as `<app-data-dir>/updater.json`
//! (same on-disk pattern as `zoom/settings.rs`; identifier-derived dir, ADR-0004).
//! specs/0086 added `early_access`. Every setter goes through [`update_settings`], a
//! read-modify-write, so changing one field never wipes the other.

use anyhow::Result;
use log::info as log_info;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdaterSettings {
    /// Check for new versions on a schedule and download them in the background.
    /// Installing/restarting is always a user action. Manual checks ignore this.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// specs/0086 — poll the early access feed instead of the everyone feed (see
    /// `super::feed`). Off by default, and independent of `auto_update`: turning it on
    /// never downloads anything by itself while automatic downloads are off.
    #[serde(default)]
    pub early_access: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UpdaterSettings {
    fn default() -> Self {
        Self {
            auto_update: true,
            early_access: false,
        }
    }
}

fn settings_path() -> Result<PathBuf> {
    let path = crate::app_paths::app_data_dir().join("updater.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
}

/// Serialises read-modify-write cycles, so two setters racing each other cannot both
/// read the old file and have the second save drop the first one's change.
static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn load_settings() -> UpdaterSettings {
    match settings_path() {
        Ok(path) => load_from(&path).await,
        Err(e) => {
            log_info!(
                "Could not resolve updater settings path ({}), using defaults",
                e
            );
            UpdaterSettings::default()
        }
    }
}

/// Load the file, apply `change`, save it, and return what was saved. The only way the
/// commands write this file.
pub async fn update_settings(change: impl FnOnce(&mut UpdaterSettings)) -> Result<UpdaterSettings> {
    let path = settings_path()?;
    update_at(&path, change).await
}

async fn load_from(path: &Path) -> UpdaterSettings {
    if !path.exists() {
        return UpdaterSettings::default();
    }
    match tokio::fs::read_to_string(path).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            log_info!("Failed to parse updater settings ({}), using defaults", e);
            UpdaterSettings::default()
        }),
        Err(e) => {
            log_info!("Failed to read updater settings ({}), using defaults", e);
            UpdaterSettings::default()
        }
    }
}

async fn update_at(
    path: &Path,
    change: impl FnOnce(&mut UpdaterSettings),
) -> Result<UpdaterSettings> {
    let _guard = WRITE_LOCK.lock().await;
    let mut settings = load_from(path).await;
    change(&mut settings);
    let content = serde_json::to_string_pretty(&settings)?;
    tokio::fs::write(path, content).await?;
    log_info!(
        "Saved updater settings (auto_update={}, early_access={})",
        settings.auto_update,
        settings.early_access
    );
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_auto_update_on_and_early_access_off() {
        let d = UpdaterSettings::default();
        assert!(d.auto_update);
        assert!(!d.early_access);
    }

    #[test]
    fn files_written_before_early_access_parse_with_it_off() {
        let parsed: UpdaterSettings = serde_json::from_str("{}").unwrap();
        assert!(parsed.auto_update);
        assert!(!parsed.early_access);

        let parsed: UpdaterSettings = serde_json::from_str(r#"{"auto_update":false}"#).unwrap();
        assert!(!parsed.auto_update);
        assert!(!parsed.early_access);
    }

    #[test]
    fn round_trips() {
        for (auto_update, early_access) in [(false, false), (false, true), (true, true)] {
            let s = UpdaterSettings {
                auto_update,
                early_access,
            };
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<UpdaterSettings>(&json).unwrap(), s);
        }
    }

    #[tokio::test]
    async fn setting_one_field_preserves_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater.json");

        // auto_update off, then early access on: auto_update must stay off.
        update_at(&path, |s| s.auto_update = false).await.unwrap();
        update_at(&path, |s| s.early_access = true).await.unwrap();
        let saved = load_from(&path).await;
        assert!(!saved.auto_update, "setting early_access wiped auto_update");
        assert!(saved.early_access);

        // auto_update back on: early access must stay on.
        update_at(&path, |s| s.auto_update = true).await.unwrap();
        let saved = load_from(&path).await;
        assert!(saved.auto_update);
        assert!(saved.early_access, "setting auto_update wiped early_access");
    }

    #[tokio::test]
    async fn update_returns_what_it_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater.json");
        let returned = update_at(&path, |s| s.early_access = true).await.unwrap();
        assert_eq!(returned, load_from(&path).await);
    }
}
