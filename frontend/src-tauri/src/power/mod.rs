//! Power-source awareness (low-power-mode spec §1): battery vs AC, macOS-only.
//!
//! Two consumers:
//! - the recording start path queries [`is_on_battery`] fresh to pick the
//!   effective processing mode;
//! - the frontend backlog prompt listens for the `power-source-changed` event
//!   emitted by the notification thread spawned in [`spawn_power_monitor`].
//!
//! Event-driven via `IOPSNotificationCreateRunLoopSource` — no polling. On
//! non-macOS builds everything degrades to `Unknown` / never-on-battery.

#[cfg(target_os = "macos")]
mod macos;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerSource {
    Ac,
    Battery,
    Unknown,
}

/// Fresh query of the providing power source.
pub fn current_power_source() -> PowerSource {
    #[cfg(target_os = "macos")]
    {
        macos::query_power_source()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PowerSource::Unknown
    }
}

/// `true` only when we positively know we're on battery — `Unknown` counts as
/// AC so a query failure can never silently degrade a meeting to deferred mode.
pub fn is_on_battery() -> bool {
    current_power_source() == PowerSource::Battery
}

/// Spawn the change-notification thread (call once at app setup). Each IOKit
/// power-source change re-queries and emits `power-source-changed`.
pub fn spawn_power_monitor<R: Runtime>(app: AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        macos::spawn_notification_thread(move || {
            let on_battery = is_on_battery();
            log::info!("power source changed: on_battery={on_battery}");
            let _ = app.emit(
                "power-source-changed",
                serde_json::json!({ "onBattery": on_battery }),
            );
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

/// Frontend query: current power state (used at mount and before prompting).
#[tauri::command]
pub async fn api_get_power_state() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "onBattery": is_on_battery() }))
}
