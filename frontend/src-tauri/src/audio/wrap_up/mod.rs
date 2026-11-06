//! A meeting wraps itself up in Rust (specs/0083 W2a).
//!
//! Every stop source (the stop button and rail via `stop_recording`, the tray, the Zoom
//! auto-stop, later the notification action) calls [`request_stop`]. It is single-flight: a
//! second request while one is stopping, draining or saving gets the same ticket. The task
//! it spawns (`run.rs`) stops capture, drains, saves the transcript, writes the defer
//! marker / hands off to the backlog, finishes the audio processing and starts the summary,
//! emitting one [`EVENT_WRAP_UP`] per stage. The webview only reflects it.

pub mod auto_summary;
pub mod handoff;
pub mod prefs;
pub mod reconcile;
mod run;
pub mod save;
pub mod stop_action;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use auto_summary::SkipReason;
use stop_action::Toast;

use super::recording_manager::RecordingManager;
use super::transcript_sink::DrainOutcome;
use crate::meeting_detect::classify::Platform;

/// One event per stage; the payload is the whole [`WrapUpState`].
pub const EVENT_WRAP_UP: &str = "meeting-wrap-up";

/// How long the frontend has to answer a `handoff` (spec 0051 WS2 degrade path after).
pub const HANDOFF_WAIT: Duration = Duration::from_secs(30);

/// A finished wrap-up stays readable this long, for a late-mounting UI.
const TERMINAL_LINGER: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Stopping,
    Draining,
    Saving,
    Saved,
    Discarded,
    Handoff,
    Diarizing,
    Summarizing,
    SummarySkipped,
    Done,
    Failed,
}

impl Stage {
    /// A new recording can't start until the last one is saved.
    pub fn blocks_start(self) -> bool {
        matches!(self, Stage::Stopping | Stage::Draining | Stage::Saving)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Stage::Done | Stage::Failed)
    }
}

/// Who asked for the stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSource {
    User,
    Tray,
    Shortcut,
    MeetingEnded(Platform),
    Notification,
}

impl StopSource {
    /// `stop_recording`'s optional `source` argument (default: the user).
    pub fn from_label(label: Option<&str>) -> Self {
        match label {
            Some("shortcut") => StopSource::Shortcut,
            Some("notification") => StopSource::Notification,
            Some("tray") => StopSource::Tray,
            _ => StopSource::User,
        }
    }

    fn label(self) -> String {
        match self {
            StopSource::User => "user".into(),
            StopSource::Tray => "tray".into(),
            StopSource::Shortcut => "shortcut".into(),
            StopSource::MeetingEnded(p) => format!(
                "meeting-ended:{}",
                serde_json::to_value(p)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default()
            ),
            StopSource::Notification => "notification".into(),
        }
    }
}

/// The current wrap-up, as emitted and as `api_get_wrap_up_state` returns it. `reason`,
/// `message`, `toast` and `failedStage` belong to the stage that set them and are cleared on
/// the next one, so a frontend shows a toast once per event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WrapUpState {
    pub id: u64,
    pub meeting_id: Option<String>,
    pub source: String,
    pub stage: Stage,
    pub title: Option<String>,
    pub transcription_incomplete: bool,
    pub reason: Option<SkipReason>,
    pub message: Option<String>,
    pub toast: Option<Toast>,
    pub failed_stage: Option<Stage>,
}

/// What `stop_recording` (the command) resolves with.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WrapUpTicket {
    pub accepted: bool,
    pub meeting_id: Option<String>,
}

impl WrapUpTicket {
    fn of(state: &WrapUpState) -> Self {
        Self {
            accepted: true,
            meeting_id: state.meeting_id.clone(),
        }
    }
}

/// The session as it stood just before the stop, read from `RECORDING_MANAGER`. If the stop
/// body fails, the wrap-up still saves from this plus the folder's `transcripts.json`.
#[derive(Debug, Clone, Default)]
pub struct SessionSnapshot {
    pub meeting_id: Option<String>,
    pub folder_path: Option<String>,
    pub meeting_name: Option<String>,
    pub resumed: bool,
    pub prior_audio_duration_seconds: f64,
    /// Resumed sessions' segments at the head of `transcripts.json` (already in the DB).
    pub prior_segment_count: usize,
}

impl SessionSnapshot {
    pub fn from_manager(manager: Option<&RecordingManager>, resumed: bool) -> Self {
        let Some(m) = manager else {
            return Self::default();
        };
        Self {
            meeting_id: m.get_meeting_id(),
            folder_path: m
                .get_meeting_folder()
                .map(|p| p.to_string_lossy().into_owned()),
            meeting_name: m.get_meeting_name(),
            resumed,
            prior_audio_duration_seconds: m.prior_audio_duration_seconds(),
            prior_segment_count: m.transcript_sink().prior_len(),
        }
    }
}

/// What the stop body (`recording_commands::stop_recording`) hands the wrap-up: the stop
/// payload plus the drained session's segments, in memory (this replaced the 0037 FIX C
/// transport through the frontend).
#[derive(Debug, Default)]
pub struct StoppedSession {
    pub folder_path: Option<String>,
    pub meeting_name: Option<String>,
    pub resumed: bool,
    pub prior_audio_duration_seconds: f64,
    pub meeting_id: Option<String>,
    /// This session only (resumed sessions' segments are already saved), blank text dropped,
    /// in transcript order.
    pub segments: Vec<crate::transcripts::TranscriptSegment>,
    pub drain: Option<DrainOutcome>,
}

impl StoppedSession {
    /// The `recording-stopped` payload, unchanged for `TranscriptContext`'s IndexedDB update.
    pub fn event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "message": "Recording stopped",
            "folder_path": self.folder_path,
            "meeting_name": self.meeting_name,
            "resumed": self.resumed,
            "prior_audio_duration_seconds": self.prior_audio_duration_seconds,
            "meeting_id": self.meeting_id,
        })
    }
}

static SLOT: Mutex<Option<WrapUpState>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn lock_slot() -> std::sync::MutexGuard<'static, Option<WrapUpState>> {
    SLOT.lock().unwrap_or_else(|e| e.into_inner())
}

enum Begin {
    Started(WrapUpState),
    InFlight(WrapUpState),
}

fn begin(source: StopSource, meeting_id: Option<String>) -> Begin {
    let mut slot = lock_slot();
    if let Some(current) = slot.as_ref().filter(|s| s.stage.blocks_start()) {
        return Begin::InFlight(current.clone());
    }
    let state = WrapUpState {
        id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
        meeting_id,
        source: source.label(),
        stage: Stage::Stopping,
        title: None,
        transcription_incomplete: false,
        reason: None,
        message: None,
        toast: None,
        failed_stage: None,
    };
    *slot = Some(state.clone());
    Begin::Started(state)
}

/// The wrap-up being stopped, drained or saved, if any.
fn in_flight() -> Option<WrapUpState> {
    lock_slot().clone().filter(|s| s.stage.blocks_start())
}

/// Recording start refuses while the last meeting is still being saved.
pub fn refuse_start_while_saving() -> Result<(), String> {
    match in_flight() {
        Some(_) => Err("Nixon is still saving the last meeting. Try again in a moment.".into()),
        None => Ok(()),
    }
}

/// The stop body's hook: capture has stopped, the transcription drain begins.
pub fn capture_stopped<R: Runtime>(app: &AppHandle<R>) {
    let advanced = {
        let mut slot = lock_slot();
        match slot.as_mut().filter(|s| s.stage == Stage::Stopping) {
            Some(s) => {
                s.stage = Stage::Draining;
                Some(s.clone())
            }
            None => None,
        }
    };
    if let Some(state) = advanced {
        emit(app, &state);
    }
}

fn emit<R: Runtime>(app: &AppHandle<R>, state: &WrapUpState) {
    log::info!(
        "Wrap-up {} ({}): {:?} meeting={:?}",
        state.id,
        state.source,
        state.stage,
        state.meeting_id
    );
    if let Err(e) = app.emit(EVENT_WRAP_UP, state) {
        log::warn!("Wrap-up: could not emit {EVENT_WRAP_UP}: {e}");
    }
}

/// One wrap-up's view of its own state. Writes go to the slot only while it is the slot's
/// current wrap-up (a later recording's stop may have replaced it).
pub(crate) struct Reporter<R: Runtime> {
    app: AppHandle<R>,
    state: WrapUpState,
}

impl<R: Runtime> Reporter<R> {
    fn new(app: AppHandle<R>, state: WrapUpState) -> Self {
        Self { app, state }
    }

    /// Move to `stage`, clearing the previous stage's transient fields, then apply `f`.
    fn stage(&mut self, stage: Stage, f: impl FnOnce(&mut WrapUpState)) {
        let s = &mut self.state;
        s.stage = stage;
        s.reason = None;
        s.message = None;
        s.toast = None;
        s.failed_stage = None;
        f(s);
        {
            let mut slot = lock_slot();
            if slot.as_ref().is_some_and(|c| c.id == s.id) {
                *slot = Some(s.clone());
            }
        }
        emit(&self.app, s);
        if stage.is_terminal() {
            schedule_clear(s.id);
        }
    }

    fn fail(&mut self, at: Stage, message: String) {
        log::warn!("Wrap-up {} failed at {at:?}: {message}", self.state.id);
        self.stage(Stage::Failed, |s| {
            s.failed_stage = Some(at);
            s.message = Some(message);
        });
    }
}

fn schedule_clear(id: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(TERMINAL_LINGER).await;
        let mut slot = lock_slot();
        if slot
            .as_ref()
            .is_some_and(|s| s.id == id && s.stage.is_terminal())
        {
            *slot = None;
        }
    });
}

/// Stop the recording and wrap the meeting up in the background. Returns as soon as the
/// wrap-up is accepted; a request while one is already stopping/draining/saving returns the
/// same ticket.
pub async fn request_stop<R: Runtime>(
    app: &AppHandle<R>,
    source: StopSource,
) -> Result<WrapUpTicket, String> {
    if !super::recording_commands::is_recording().await {
        return match in_flight() {
            Some(state) => Ok(WrapUpTicket::of(&state)),
            None => Err("No recording in progress".into()),
        };
    }
    let snapshot = super::recording_commands::session_snapshot();
    let state = match begin(source, snapshot.meeting_id.clone()) {
        Begin::InFlight(state) => return Ok(WrapUpTicket::of(&state)),
        Begin::Started(state) => state,
    };
    // App Nap stays off until the terminal stage; the sleep assertion goes at capture stop.
    let hold = crate::power::keep_awake::hold(crate::power::keep_awake::HoldReason::WrapUp);
    let deferral = super::live_toggle::take_session_deferral();
    let ticket = WrapUpTicket::of(&state);
    let reporter = Reporter::new(app.clone(), state);
    emit(app, &reporter.state);
    let stop_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let stop = async move {
            let result = super::recording_commands::stop_recording(stop_app.clone()).await;
            if result.is_ok() {
                crate::notifications::os_commands::recording_banner(
                    &stop_app,
                    "Recording stopped",
                    "Nixon is processing the meeting.",
                    "nixon-recording",
                )
                .await;
            } else {
                crate::tray::update_tray_menu(&stop_app);
            }
            result
        };
        run::run(
            reporter,
            run::Inputs {
                snapshot,
                deferral,
                handoff_wait: HANDOFF_WAIT,
            },
            stop,
        )
        .await;
        drop(hold);
    });
    Ok(ticket)
}

#[tauri::command]
pub async fn api_get_wrap_up_state() -> Result<Option<WrapUpState>, String> {
    Ok(lock_slot().clone())
}

/// The frontend's `DeferredBacklogProvider` answers a `handoff` stage.
#[tauri::command]
pub async fn api_wrap_up_handoff_result(meeting_id: String, accepted: bool) -> Result<(), String> {
    if !handoff::answer(&meeting_id, accepted) {
        log::info!("Wrap-up: late handoff answer for {meeting_id} ignored (accepted={accepted})");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    //! The slot is process-wide, so every test that touches it holds this lock.
    pub static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    pub fn reset_slot() {
        *super::lock_slot() = None;
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
