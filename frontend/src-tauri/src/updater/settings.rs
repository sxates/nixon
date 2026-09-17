//! specs/0058 — the one updater preference, persisted as `<app-data-dir>/updater.json`
//! (same on-disk pattern as `zoom/settings.rs`; identifier-derived dir, ADR-0004).

use anyhow::Result;
use log::info as log_info;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdaterSettings {
    /// Check GitHub on a schedule and download new versions in the background.
    /// Installing/restarting is always a user action. Manual checks ignore this.
    #[serde(default = "default_true")]
    pub auto_update: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UpdaterSettings {
    fn default() -> Self {
        Self { auto_update: true }
    }
}

fn settings_path() -> Result<PathBuf> {
    let path = crate::app_paths::app_data_dir().join("updater.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
}

pub async fn load_settings() -> UpdaterSettings {
    let path = match settings_path() {
        Ok(p) => p,
        Err(e) => {
            log_info!(
                "Could not resolve updater settings path ({}), using defaults",
                e
            );
            return UpdaterSettings::default();
        }
    };
    if !path.exists() {
        return UpdaterSettings::default();
    }
    match tokio::fs::read_to_string(&path).await {
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

pub async fn save_settings(settings: &UpdaterSettings) -> Result<()> {
    let path = settings_path()?;
    let content = serde_json::to_string_pretty(settings)?;
    tokio::fs::write(&path, content).await?;
    log_info!(
        "Saved updater settings (auto_update={})",
        settings.auto_update
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_auto_update_on_and_tolerates_empty_object() {
        assert!(UpdaterSettings::default().auto_update);
        let parsed: UpdaterSettings = serde_json::from_str("{}").unwrap();
        assert!(parsed.auto_update);
    }

    #[test]
    fn round_trips() {
        let s = UpdaterSettings { auto_update: false };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<UpdaterSettings>(&json).unwrap(), s);
    }
}
