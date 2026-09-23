// Background meeting monitor (specs/0008 P1, generalised to Teams and Meet in specs/0074 W6).
//
// Polls every POLL_INTERVAL, classifies the raw signals (`classify`), debounces the answer
// (`debounce`) and emits Tauri events on confirmed transitions. Unchanged from the Zoom-only
// monitor: the 2-poll debounce, no prompt while recording, and the `zoom_auto_detect`
// setting re-read at every detection so toggling it takes effect without a relaunch.

use std::time::Duration;

use serde::Serialize;
use sysinfo::System;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::calendar::CalendarProbe;
use super::classify::{browser_mic_pids, calendar_corroborates, classify, AudioProc, Platform};
use super::debounce::{DebounceMachine, Transition};
use super::sample;
use crate::audio::recording_commands::is_recording;
use crate::zoom::settings;

/// How often to poll.
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Number of consecutive consistent polls required before flipping state.
/// 2 means a transition needs ~6s of agreement, which absorbs single-poll
/// flicker (e.g. a brief helper restart during screen-share / breakout churn).
const DEBOUNCE_POLLS: u8 = 2;

/// Emitted to the main window when a call is detected starting (and Nixon isn't already
/// recording and the setting is enabled). Renamed from `zoom-meeting-detected` in 0074 W6.
pub const EVENT_MEETING_DETECTED: &str = "meeting-detected";

/// Emitted to the main window when a detected call ends. Renamed from `zoom-meeting-ended`.
pub const EVENT_MEETING_ENDED: &str = "meeting-ended";

/// Payload for `meeting-detected` / `meeting-ended`. The frontend reads these snake_case
/// names (`lib/meeting-detect.ts::MeetingDetectEvent`).
#[derive(Debug, Clone, Serialize)]
pub struct MeetingEventPayload {
    /// The app the call is on: `"zoom"`, `"teams"` or `"meet"`.
    pub platform: Platform,
    /// `"detected"` or `"ended"` — convenience for the frontend.
    pub kind: &'static str,
    /// Unix epoch milliseconds at which the transition was confirmed. The frontend keys the
    /// OS banner on it, so it is unique per detection.
    pub timestamp_ms: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One poll's raw facts, sampled on the blocking pool.
struct Sampled {
    sys: System,
    zoom: bool,
    procs: Vec<AudioProc>,
}

fn sample_blocking(mut sys: System) -> Sampled {
    sample::refresh(&mut sys);
    let zoom = sample::zoom_meeting_process_present(&sys);
    // Zoom wins outright, so skip the Core Audio query while its helpers are up.
    let procs = if zoom {
        Vec::new()
    } else {
        sample::audio_processes()
    };
    Sampled { sys, zoom, procs }
}

/// Spawn the background meeting monitor. Called once from `lib.rs::run().setup`.
pub fn spawn_meeting_monitor<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        log::info!(
            "Meeting monitor started (poll every {:?}, debounce {} polls, zoom helpers {:?})",
            POLL_INTERVAL,
            DEBOUNCE_POLLS,
            sample::ZOOM_MEETING_PROCESSES
        );

        let own_pid = std::process::id() as i32;
        let mut sys = Some(System::new());
        let mut machine = DebounceMachine::<Platform>::new(DEBOUNCE_POLLS);
        let mut calendar = CalendarProbe::default();

        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let taken = sys.take().unwrap_or_default();
            let Sampled {
                sys: mut s,
                zoom,
                procs,
            } = match tokio::task::spawn_blocking(move || sample_blocking(taken)).await {
                Ok(sampled) => sampled,
                Err(e) => {
                    log::warn!("Meeting monitor: sampling task failed: {e}");
                    sys = Some(System::new());
                    continue;
                }
            };

            // Zoom and Teams need nothing else. A browser on the microphone needs
            // corroboration, which is read only now: the calendar first, then (Accessibility
            // already granted, never prompted) the browser's front window title. Neither is
            // read while detection is off or while recording: nothing would be prompted, and
            // sampling itself continues so Zoom's end still stops a recording.
            let browsers = browser_mic_pids(&procs, own_pid);
            let current = machine.state();
            let mut raw = classify(zoom, &procs, own_pid, false, None);
            if raw.is_none() && !browsers.is_empty() {
                let detection_on = settings::load_settings().await.zoom_auto_detect;
                if !may_corroborate(detection_on, is_recording().await) {
                    raw = uncorroborated(zoom, &procs, own_pid, current);
                } else {
                    let event_now = calendar_corroborates(calendar.event_now(&app).await, current);
                    let title = if event_now {
                        None
                    } else {
                        let helper = browsers[0];
                        let (back, title) = tokio::task::spawn_blocking(move || {
                            let title = sample::browser_app_pid(&s, helper)
                                .and_then(sample::front_window_title);
                            (s, title)
                        })
                        .await
                        .unwrap_or_else(|_| (System::new(), None));
                        s = back;
                        title
                    };
                    raw = classify(zoom, &procs, own_pid, event_now, title.as_deref());
                }
            }
            sys = Some(s);

            match machine.observe(raw) {
                Transition::None => {}
                Transition::Started(platform) => on_started(&app, platform).await,
                Transition::Ended(platform) => on_ended(&app, platform),
                Transition::Switched { ended, started } => {
                    on_ended(&app, ended);
                    on_started(&app, started).await;
                }
            }
        }
    });
}

/// Whether a poll may read the calendar and the browser's window title to corroborate a
/// browser on the microphone. Only when detection is on and Nixon is not recording.
fn may_corroborate(detection_on: bool, recording: bool) -> bool {
    detection_on && !recording
}

/// The classifier's answer for a poll that may not corroborate: an already-confirmed Meet
/// call is held (so suppressing corroboration never fakes an end), and no new Meet call starts.
fn uncorroborated(
    zoom: bool,
    procs: &[AudioProc],
    own_pid: i32,
    current: Option<Platform>,
) -> Option<Platform> {
    classify(
        zoom,
        procs,
        own_pid,
        calendar_corroborates(false, current),
        None,
    )
}

async fn on_started<R: Runtime>(app: &AppHandle<R>, platform: Platform) {
    // Re-read the setting each transition so toggling takes effect live without a relaunch.
    if !settings::load_settings().await.zoom_auto_detect {
        log::info!("{platform:?} call detected but meeting detection is off; not emitting");
        return;
    }
    if is_recording().await {
        log::info!("{platform:?} call detected but Nixon is already recording; not prompting");
        return;
    }
    log::info!("{platform:?} call started; emitting {EVENT_MEETING_DETECTED}");
    emit(app, EVENT_MEETING_DETECTED, platform, "detected");
}

fn on_ended<R: Runtime>(app: &AppHandle<R>, platform: Platform) {
    log::info!(
        "{platform:?} call ended; emitting {EVENT_MEETING_ENDED} (stops a recording: {})",
        platform.end_stops_recording()
    );
    emit(app, EVENT_MEETING_ENDED, platform, "ended");
}

fn emit<R: Runtime>(app: &AppHandle<R>, event: &str, platform: Platform, kind: &'static str) {
    let payload = MeetingEventPayload {
        platform,
        kind,
        timestamp_ms: now_ms(),
    };
    match app.get_webview_window("main") {
        Some(window) => {
            if let Err(e) = window.emit(event, payload) {
                log::error!("Failed to emit {event}: {e}");
            }
        }
        None => log::warn!("No main window to receive {event}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser_on_mic() -> Vec<AudioProc> {
        vec![AudioProc {
            pid: 77,
            bundle_id: "com.google.Chrome.helper".into(),
            running_input: true,
        }]
    }

    #[test]
    fn corroboration_is_read_only_while_detection_is_on_and_not_recording() {
        assert!(may_corroborate(true, false));
        assert!(!may_corroborate(false, false), "detection off");
        assert!(!may_corroborate(true, true), "recording");
        assert!(!may_corroborate(false, true));
    }

    #[test]
    fn an_uncorroborated_poll_holds_a_confirmed_meet_call_and_starts_none() {
        let procs = browser_on_mic();
        assert_eq!(
            uncorroborated(false, &procs, 1, Some(Platform::Meet)),
            Some(Platform::Meet)
        );
        assert_eq!(uncorroborated(false, &procs, 1, None), None);
        // Zoom's helpers still win, so its end (the auto-stop) is still seen while recording.
        assert_eq!(uncorroborated(true, &procs, 1, None), Some(Platform::Zoom));
        assert_eq!(uncorroborated(false, &[], 1, Some(Platform::Zoom)), None);
    }

    #[test]
    fn the_payload_carries_the_platform_in_the_frontends_field_names() {
        let json = serde_json::to_value(MeetingEventPayload {
            platform: Platform::Teams,
            kind: "detected",
            timestamp_ms: 7,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "platform": "teams", "kind": "detected", "timestamp_ms": 7 })
        );
    }
}
