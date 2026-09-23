// The command surface for OS notifications (specs/0068).
//
// Kept apart from `commands.rs`, which fronts the 0008-era manager: that file's commands
// are almost all unreachable from the frontend today and are on the list to go, and putting
// the live surface inside it would make the two indistinguishable.

use std::collections::HashMap;

use crate::notifications::macos::{self, Capability, DeliverRequest, CATEGORY_PLAIN};

/// Whether this build can show an OS notification at all, and if not, the sentence the
/// Settings row prints. False on a bare `tauri dev` binary — see `macos::capability`.
#[tauri::command]
pub fn notif_capability() -> Capability {
    macos::capability()
}

/// Read the macOS authorization state without prompting.
#[tauri::command]
pub async fn notif_authorization_status() -> String {
    macos::authorization_status().await
}

/// Ask macOS for permission. Shows its dialog once, ever; afterwards this returns the
/// stored answer, which is why the UI offers System Settings once the answer is "denied".
#[tauri::command]
pub async fn notif_request_authorization() -> Result<bool, String> {
    macos::request_authorization().await.map_err(|e| e.to_string())
}

/// Post a notification now.
#[tauri::command]
pub fn notif_deliver(request: DeliverRequest) -> Result<(), String> {
    macos::deliver(request).map_err(|e| e.to_string())
}

/// Open System Settings → Notifications.
///
/// A command of its own rather than `utils::open_external_url`, because that one keeps a
/// deliberate scheme allowlist (http/https/mailto/Zoom — specs/0028 hardening) and widening
/// it to admit `x-apple.systempreferences:` for a convenience link would trade a security
/// control for a shortcut. The URL here is a constant, so there is no input to police.
#[tauri::command]
pub fn notif_open_system_settings() -> Result<(), String> {
    const PANE: &str = "x-apple.systempreferences:com.apple.Notifications-Settings.extension";

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(PANE)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not open Notification settings: {e}"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = PANE;
        Err("Only macOS has a notification settings pane to open.".to_string())
    }
}

/// Banner when a recording starts or stops — but only when Nixon is not the front app
/// (specs/0068).
///
/// It lives in the recording path rather than the frontend because a recording can now be
/// started from a notification button while Nixon has no visible window, and that is the
/// case worth a banner. When Nixon *is* in front there is already a recording indicator on
/// screen and an in-app reminder toast, so a banner would be a third copy of the same news
/// — which is why this is a rule rather than another switch next to the reminder's.
///
/// Failures are logged, never propagated: a missing banner must not fail a recording.
pub async fn recording_banner<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    title: &str,
    body: &str,
    id: &str,
) {
    if is_frontmost(app) {
        return;
    }
    if let Err(error) = macos::deliver(DeliverRequest {
        id: id.to_string(),
        title: title.to_string(),
        body: body.to_string(),
        category: Some(CATEGORY_PLAIN.to_string()),
        user_info: HashMap::new(),
        // None => the category's default (transient). A recording-started banner is news,
        // not a decision, so it has no business sitting on screen until dismissed.
        auto_dismiss_ms: None,
    }) {
        log::debug!("notifications: recording banner not delivered: {error}");
    }
}

/// Is the main window focused? Treated as "no" when the window or the query is
/// unavailable, so an unknown state still gets the banner rather than silently dropping it.
fn is_frontmost<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    use tauri::Manager;
    app.get_webview_window("main")
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false)
}
