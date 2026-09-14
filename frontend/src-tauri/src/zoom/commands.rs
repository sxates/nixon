// Tauri commands for the Zoom settings (specs/0008 P1 auto-detect; specs/0049 mute gate).
//
// The monitors read the persisted settings fresh each poll, so toggling via these
// commands takes effect live without a relaunch. Setters load-modify-save so the two
// independent toggles never clobber each other.

use crate::zoom::settings;

/// Get whether Zoom meeting auto-detection is enabled.
#[tauri::command]
pub async fn api_get_zoom_auto_detect() -> Result<bool, String> {
    Ok(settings::load_settings().await.zoom_auto_detect)
}

/// Enable or disable Zoom meeting auto-detection. Persisted to disk.
#[tauri::command]
pub async fn api_set_zoom_auto_detect(enabled: bool) -> Result<(), String> {
    let mut s = settings::load_settings().await;
    s.zoom_auto_detect = enabled;
    settings::save_settings(&s)
        .await
        .map_err(|e| format!("Failed to save Zoom auto-detect setting: {e}"))
}

/// Get whether the Zoom mute gate (pause owner mic while muted in Zoom) is enabled.
#[tauri::command]
pub async fn api_get_zoom_mute_gate() -> Result<bool, String> {
    Ok(settings::load_settings().await.zoom_mute_gate)
}

/// Enable or disable the Zoom mute gate (specs/0049). Persisted to disk.
#[tauri::command]
pub async fn api_set_zoom_mute_gate(enabled: bool) -> Result<(), String> {
    let mut s = settings::load_settings().await;
    s.zoom_mute_gate = enabled;
    settings::save_settings(&s)
        .await
        .map_err(|e| format!("Failed to save Zoom mute-gate setting: {e}"))
}

/// Whether this app is trusted for the macOS Accessibility API (needed to read
/// Zoom's mute state). When `prompt` is true and it isn't trusted, macOS shows the
/// system "grant Accessibility" prompt. Returns the current trust state.
#[tauri::command]
pub async fn api_zoom_mute_ax_trusted(prompt: bool) -> Result<bool, String> {
    Ok(crate::zoom::mute::ensure_ax_trust(prompt))
}

/// Open System Settings → Privacy & Security → Accessibility so the user can grant
/// the permission. Best-effort; no-op error surfaced to the caller.
#[tauri::command]
pub async fn api_open_accessibility_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn()
            .map_err(|e| format!("Failed to open Accessibility settings: {e}"))?;
    }
    Ok(())
}
