// Persisted settings for Zoom auto-detection.
//
// Stored as a small JSON file alongside the existing notification settings
// (config_dir()/<app-data>/zoom.json), matching the on-disk pattern used by
// `notifications::settings::ConsentManager`. Kept deliberately separate from
// the notification settings file so the monitor can read it cheaply each loop
// without touching unrelated state.

use anyhow::Result;
use log::info as log_info;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoomSettings {
    /// Detect Zoom meetings starting/ending and emit events to the frontend.
    pub zoom_auto_detect: bool,
    /// specs/0049: while recording, pause the owner microphone whenever you're muted
    /// in Zoom (read via macOS Accessibility). Off by default — it needs the extra
    /// Accessibility permission. `serde(default)` so pre-0049 files still parse.
    #[serde(default)]
    pub zoom_mute_gate: bool,
}

impl Default for ZoomSettings {
    fn default() -> Self {
        Self {
            // Opt-in by default per specs/0008 P1 (highest value/effort ratio).
            zoom_auto_detect: true,
            // Off by default — extra Accessibility permission + fragile (specs/0049).
            zoom_mute_gate: false,
        }
    }
}

/// Path of the zoom settings JSON file (`<app-data-dir>/zoom.json`), rooted at
/// the identifier-derived data dir so dev/prod stay isolated (ADR-0004).
fn settings_path() -> Result<PathBuf> {
    let path = crate::app_paths::app_data_dir().join("zoom.json");

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    Ok(path)
}

/// Load zoom settings from disk, falling back to defaults if absent/unreadable.
pub async fn load_settings() -> ZoomSettings {
    let path = match settings_path() {
        Ok(p) => p,
        Err(e) => {
            log_info!(
                "Could not resolve zoom settings path ({}), using defaults",
                e
            );
            return ZoomSettings::default();
        }
    };

    if !path.exists() {
        return ZoomSettings::default();
    }

    match tokio::fs::read_to_string(&path).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            log_info!("Failed to parse zoom settings ({}), using defaults", e);
            ZoomSettings::default()
        }),
        Err(e) => {
            log_info!("Failed to read zoom settings ({}), using defaults", e);
            ZoomSettings::default()
        }
    }
}

/// Persist zoom settings to disk.
pub async fn save_settings(settings: &ZoomSettings) -> Result<()> {
    let path = settings_path()?;
    let content = serde_json::to_string_pretty(settings)?;
    tokio::fs::write(&path, content).await?;
    log_info!(
        "Saved zoom settings (zoom_auto_detect={})",
        settings.zoom_auto_detect
    );
    Ok(())
}
