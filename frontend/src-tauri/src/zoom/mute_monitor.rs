//! Zoom self-mute → owner-mic gate poll (specs/0049).
//!
//! A single global task (spawned at setup, like the meeting monitor) that — WHILE a
//! recording is active AND the `zoom_mute_gate` setting is on — reads Zoom's mute
//! state via Accessibility and flips the live recording's `is_muted` flag. On no
//! recording / feature off / Zoom not in a meeting / AX not permitted it leaves the
//! mic UNGATED (fail toward recording — never silently drop the owner's audio on
//! uncertainty). Emits `zoom-mute-changed` for the UI indicator.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::audio::mute_gate;
use crate::audio::recording_commands::is_recording;
use crate::zoom::{mute, settings};

/// How often we read Zoom's mute state while recording — fast enough to feel
/// immediate, cheap because it reads one menu (not the window tree) and reuses a
/// cached Zoom PID between polls.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(700);

pub const EVENT_MUTE_CHANGED: &str = "zoom-mute-changed";

#[derive(Clone, Serialize)]
struct MutePayload {
    muted: bool,
}

pub fn spawn_zoom_mute_monitor<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        log::info!("Zoom mute-gate monitor started (poll every {POLL_INTERVAL:?})");
        let mut last_emitted: Option<bool> = None;
        let mut cached_pid: Option<i32> = None;

        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            // Only do anything while a recording is live.
            if !is_recording().await {
                last_emitted = None; // reset between recordings
                cached_pid = None;
                continue;
            }

            // Re-read the setting each poll so toggling takes effect without a relaunch.
            if !settings::load_settings().await.zoom_mute_gate {
                if mute_gate::is_muted() {
                    mute_gate::set_muted(false); // feature turned off mid-recording → ungate
                }
                continue;
            }

            // Read Zoom's mute state. Reading another app's AX tree needs the
            // Accessibility grant; we never PROMPT from the poll (the settings toggle
            // does). Reuse the cached PID; re-fetch if missing or the read fails
            // (Zoom relaunched / left the meeting).
            let muted = if mute::ensure_ax_trust(false) {
                if cached_pid.is_none() {
                    cached_pid = mute::zoom_app_pid();
                }
                let mut m = cached_pid.and_then(mute::read_zoom_mute_state);
                if m.is_none() {
                    cached_pid = mute::zoom_app_pid();
                    m = cached_pid.and_then(mute::read_zoom_mute_state);
                }
                m
            } else {
                None
            };

            // None (no active Zoom meeting / not readable) → ungate. We only gate on
            // a POSITIVE muted read, so an uncertain read never drops the owner's mic.
            let muted = muted.unwrap_or(false);
            if mute_gate::is_muted() != muted {
                mute_gate::set_muted(muted);
                log::info!(
                    "Zoom mute-gate: owner mic {}",
                    if muted { "MUTED (dropped)" } else { "unmuted" }
                );
            }
            if last_emitted != Some(muted) {
                last_emitted = Some(muted);
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.emit(EVENT_MUTE_CHANGED, MutePayload { muted });
                }
            }
        }
    });
}
