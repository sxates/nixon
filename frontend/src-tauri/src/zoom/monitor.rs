// Background Zoom-meeting monitor (specs/0008 P1).
//
// Polls the process list (~every POLL_INTERVAL) for a running Zoom
// meeting-helper process, debounces the raw signal, and emits Tauri events on
// confirmed transitions. The debounce state machine is split out into
// `DebounceMachine` so it can be unit-tested without any process/Tauri deps.

use std::time::Duration;

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::audio::recording_commands::is_recording;
use crate::zoom::settings;

/// How often to poll the process list.
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Number of consecutive consistent polls required before flipping state.
/// 2 means a transition needs ~6s of agreement, which absorbs single-poll
/// flicker (e.g. a brief helper restart during screen-share / breakout churn).
const DEBOUNCE_POLLS: u8 = 2;

/// Zoom meeting-helper process names that exist ONLY during an active meeting
/// (case-insensitive match). `caphost` was removed — it's Zoom's screen-share/capture
/// helper that keeps running while Zoom is open with NO meeting (verified: idle → only
/// caphost present), which caused false "meeting detected" prompts. `cpthost` (caption
/// host) and `aomhost` (audio) spawn for an actual meeting and exit on leave. Isolated
/// here so a Zoom rename is a one-line change.
const ZOOM_MEETING_PROCESSES: [&str; 2] = ["cpthost", "aomhost"];

/// Tauri event emitted to the main window when a Zoom meeting is detected
/// starting (and Nixon isn't already recording and the setting is enabled).
pub const EVENT_MEETING_DETECTED: &str = "zoom-meeting-detected";

/// Tauri event emitted to the main window when a detected Zoom meeting ends.
pub const EVENT_MEETING_ENDED: &str = "zoom-meeting-ended";

/// Payload for `zoom-meeting-detected` / `zoom-meeting-ended`.
/// Kept intentionally small; carries a unix-millis timestamp of the transition.
#[derive(Debug, Clone, Serialize)]
pub struct ZoomEventPayload {
    /// Unix epoch milliseconds at which the transition was confirmed.
    pub timestamp_ms: u64,
    /// `"detected"` or `"ended"` — convenience for the frontend.
    pub kind: &'static str,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Confirmed meeting state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingState {
    Idle,
    InMeeting,
}

/// What the machine wants the caller to do after a poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// No confirmed change this poll.
    None,
    /// Confirmed Idle -> InMeeting.
    Started,
    /// Confirmed InMeeting -> Idle.
    Ended,
}

/// Pure debounced Idle<->InMeeting state machine. No process or Tauri deps so
/// it can be unit-tested deterministically.
#[derive(Debug)]
pub struct DebounceMachine {
    state: MeetingState,
    /// Count of consecutive polls whose raw signal disagrees with `state`.
    pending: u8,
    threshold: u8,
}

impl DebounceMachine {
    pub fn new(threshold: u8) -> Self {
        Self {
            state: MeetingState::Idle,
            // threshold must be >= 1; a 0 threshold would flip on every poll.
            pending: 0,
            threshold: threshold.max(1),
        }
    }

    pub fn state(&self) -> MeetingState {
        self.state
    }

    /// Feed one raw observation (`true` = a meeting-helper process is present).
    /// Returns the confirmed transition, if any.
    pub fn observe(&mut self, meeting_present: bool) -> Transition {
        let agrees_with_state = matches!(
            (self.state, meeting_present),
            (MeetingState::Idle, false) | (MeetingState::InMeeting, true)
        );

        if agrees_with_state {
            // Raw signal matches current confirmed state: reset any pending flip.
            self.pending = 0;
            return Transition::None;
        }

        // Raw signal disagrees: count toward a flip.
        self.pending += 1;
        if self.pending < self.threshold {
            return Transition::None;
        }

        // Confirmed flip.
        self.pending = 0;
        match self.state {
            MeetingState::Idle => {
                self.state = MeetingState::InMeeting;
                Transition::Started
            }
            MeetingState::InMeeting => {
                self.state = MeetingState::Idle;
                Transition::Ended
            }
        }
    }
}

/// Returns true if a Zoom meeting-helper process is currently running.
fn zoom_meeting_process_present(sys: &mut System) -> bool {
    // Scope the refresh to process names only (cheapest refresh — no cpu/mem/
    // disk/env), and only update process metadata, not networks/disks/etc.
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());

    sys.processes().values().any(|proc_| {
        let name = proc_.name().to_string_lossy();
        ZOOM_MEETING_PROCESSES
            .iter()
            .any(|target| name.eq_ignore_ascii_case(target))
    })
}

/// Spawn the background Zoom monitor. Call from `lib.rs::run().setup` inside the
/// existing `tauri::async_runtime::spawn` pattern.
pub fn spawn_zoom_monitor<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        log::info!(
            "Zoom monitor started (poll every {:?}, debounce {} polls, processes {:?})",
            POLL_INTERVAL,
            DEBOUNCE_POLLS,
            ZOOM_MEETING_PROCESSES
        );

        // A minimal System; we only ever refresh process names on it.
        let mut sys = System::new();
        let mut machine = DebounceMachine::new(DEBOUNCE_POLLS);

        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let present = zoom_meeting_process_present(&mut sys);
            match machine.observe(present) {
                Transition::None => {}
                Transition::Started => {
                    // Re-read the setting each transition so toggling takes
                    // effect live without a relaunch.
                    let enabled = settings::load_settings().await.zoom_auto_detect;
                    if !enabled {
                        log::info!(
                            "Zoom meeting detected but zoom_auto_detect is off; not emitting"
                        );
                        continue;
                    }

                    if is_recording().await {
                        log::info!(
                            "Zoom meeting detected but Nixon is already recording; not prompting"
                        );
                        continue;
                    }

                    log::info!(
                        "Zoom transition Idle -> InMeeting; emitting {EVENT_MEETING_DETECTED}"
                    );
                    let payload = ZoomEventPayload {
                        timestamp_ms: now_ms(),
                        kind: "detected",
                    };
                    if let Some(window) = app.get_webview_window("main") {
                        if let Err(e) = window.emit(EVENT_MEETING_DETECTED, payload) {
                            log::error!("Failed to emit {EVENT_MEETING_DETECTED}: {e}");
                        }
                    } else {
                        log::warn!(
                            "Zoom meeting detected but no main window to receive {EVENT_MEETING_DETECTED}"
                        );
                    }
                }
                Transition::Ended => {
                    log::info!("Zoom transition InMeeting -> Idle; emitting {EVENT_MEETING_ENDED}");
                    let payload = ZoomEventPayload {
                        timestamp_ms: now_ms(),
                        kind: "ended",
                    };
                    if let Some(window) = app.get_webview_window("main") {
                        if let Err(e) = window.emit(EVENT_MEETING_ENDED, payload) {
                            log::error!("Failed to emit {EVENT_MEETING_ENDED}: {e}");
                        }
                    } else {
                        log::warn!(
                            "Zoom meeting ended but no main window to receive {EVENT_MEETING_ENDED}"
                        );
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_idle() {
        let m = DebounceMachine::new(2);
        assert_eq!(m.state(), MeetingState::Idle);
    }

    #[test]
    fn requires_consecutive_polls_to_enter_meeting() {
        let mut m = DebounceMachine::new(2);
        // First positive observation: pending, not yet confirmed.
        assert_eq!(m.observe(true), Transition::None);
        assert_eq!(m.state(), MeetingState::Idle);
        // Second consecutive positive: confirmed start.
        assert_eq!(m.observe(true), Transition::Started);
        assert_eq!(m.state(), MeetingState::InMeeting);
    }

    #[test]
    fn single_poll_flicker_does_not_flip() {
        let mut m = DebounceMachine::new(2);
        // One stray positive then back to negative: no transition.
        assert_eq!(m.observe(true), Transition::None);
        assert_eq!(m.observe(false), Transition::None);
        assert_eq!(m.state(), MeetingState::Idle);
    }

    #[test]
    fn requires_consecutive_polls_to_end_meeting() {
        let mut m = DebounceMachine::new(2);
        // Enter a meeting.
        m.observe(true);
        assert_eq!(m.observe(true), Transition::Started);
        // One missing poll (helper blip): not yet ended.
        assert_eq!(m.observe(false), Transition::None);
        assert_eq!(m.state(), MeetingState::InMeeting);
        // Second consecutive missing: confirmed end.
        assert_eq!(m.observe(false), Transition::Ended);
        assert_eq!(m.state(), MeetingState::Idle);
    }

    #[test]
    fn blip_during_meeting_resets_pending_end() {
        let mut m = DebounceMachine::new(2);
        m.observe(true);
        assert_eq!(m.observe(true), Transition::Started);
        // Process disappears for one poll then comes back: stays InMeeting,
        // and a later single disappearance must not immediately end it.
        assert_eq!(m.observe(false), Transition::None);
        assert_eq!(m.observe(true), Transition::None);
        assert_eq!(m.observe(false), Transition::None);
        assert_eq!(m.state(), MeetingState::InMeeting);
    }

    #[test]
    fn threshold_zero_is_clamped_to_one() {
        let mut m = DebounceMachine::new(0);
        // With threshold clamped to 1, a single positive confirms immediately.
        assert_eq!(m.observe(true), Transition::Started);
    }

    #[test]
    fn full_cycle() {
        let mut m = DebounceMachine::new(2);
        assert_eq!(m.observe(true), Transition::None);
        assert_eq!(m.observe(true), Transition::Started);
        assert_eq!(m.observe(true), Transition::None); // steady InMeeting
        assert_eq!(m.observe(false), Transition::None);
        assert_eq!(m.observe(false), Transition::Ended);
        assert_eq!(m.observe(false), Transition::None); // steady Idle
    }
}
