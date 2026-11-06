//! specs/0084 W2 follow-up — a cancel ends the pass for the user at once.
//!
//! sherpa's "identifying N%" compute step is one C call that ignores its progress
//! callback's return value, so it can't be stopped in-process (W3's helper process can be
//! killed; this is the stop-gap). A cancel therefore *detaches*: [`finish_cancelled`] ends
//! everything the user sees (queue row, run status, the quiet `diarization-error`, the
//! meeting's `failed`/`cancelled` state) the moment Cancel is clicked, progress events stop
//! ([`emit_progress`]), and the compute call finishes unseen. When it returns the pass ends
//! as cancelled; the persist guard writes nothing and the second [`finish_cancelled`] is a
//! no-op.
//!
//! The crash-loop marker (`speaker_id_started_at`) stays set until the call really returns,
//! so a crash in the hidden call is still caught by the startup breaker. Until then the
//! meeting is "still stopping": a new pass on it is refused with [`STILL_STOPPING`]
//! ([`refuse_if_stopping`]) rather than stacked, because the old pass's stage checks read
//! the cancel flag by meeting id and a newer pass's unset flag would let the old one write.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::cancel::{self, PassCancel};
use super::launch::{drop_pending_rerun, CANCELLED_REASON, STAGE_CANCELLED};
use super::pipeline::{registry_finish, EVENT_ERROR, EVENT_PROGRESS};
use crate::audio::lifecycle::policy::AudioState;
use crate::audio::lifecycle::state::{self, SpeakerIdFailure};
use crate::audio::lifecycle::{AudioStateChanged, EVENT_AUDIO_STATE_CHANGED};
use crate::state::AppState;

/// What a request to identify speakers says while a cancelled pass is still finishing.
pub const STILL_STOPPING: &str = "Still stopping the previous run — try again in a moment.";

/// `Err(STILL_STOPPING)` while a cancelled pass on this meeting hasn't returned yet.
pub fn refuse_if_stopping(meeting_id: &str) -> Result<(), String> {
    if cancel::is_stopping(meeting_id) {
        log::info!("diarization for {meeting_id}: the cancelled pass is still finishing");
        return Err(STILL_STOPPING.to_string());
    }
    Ok(())
}

/// Whether [`finish_cancelled`] also moves the meeting to `failed`/`cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Db {
    /// The Cancel click: the pass is still running unseen, so the crash-loop marker stays.
    KeepMarker,
    /// The pass has returned and `on_diarization_outcome` already did the DB side.
    Done,
}

/// End the pass's visible state as a completed cancelled pass would: the meeting becomes
/// `failed`/`cancelled` (when pending), the queue row is finished as skipped (no red lamp,
/// no toast), the run slot reads `cancelled`, any queued re-run is dropped, and the quiet
/// `diarization-error` (`cancelled: true`) goes out. Runs once per pass; later calls are
/// no-ops, so the late return of a detached pass adds nothing.
pub async fn finish_cancelled<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
    pass: &Arc<PassCancel>,
    db: Db,
) {
    let Some(task) = pass.claim_visible_finish() else {
        return;
    };
    if db == Db::KeepMarker {
        mark_cancelled_keeping_marker(app, meeting_id).await;
    }
    if let Some(t) = task {
        t.finish_skipped(CANCELLED_REASON);
    }
    // The run slot reads cancelled, so the page's status rehydrate shows the pass ended. A
    // new pass is still refused until the hidden call returns (`refuse_if_stopping`).
    registry_finish(meeting_id, STAGE_CANCELLED, 0);
    // A cancel means "stop": a queued follow-up would undo it.
    drop_pending_rerun(meeting_id);
    let _ = app.emit(
        EVENT_ERROR,
        serde_json::json!({
            "meeting_id": meeting_id,
            "error": CANCELLED_REASON,
            "cancelled": true,
        }),
    );
}

async fn mark_cancelled_keeping_marker<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) {
    let Some(app_state) = app.try_state::<AppState>() else {
        return;
    };
    let pool = app_state.db_manager.pool();
    match state::mark_failed_if_pending_keeping_marker(
        pool,
        meeting_id,
        SpeakerIdFailure::Cancelled,
    )
    .await
    {
        Ok(true) => {
            let _ = app.emit(
                EVENT_AUDIO_STATE_CHANGED,
                AudioStateChanged {
                    meeting_id: meeting_id.to_string(),
                    state: AudioState::Failed,
                },
            );
        }
        Ok(false) => {}
        Err(e) => log::warn!("diarization for {meeting_id}: {e:#}"),
    }
}

/// Emit a `diarization-progress` event, unless the pass was cancelled: a detached pass
/// must not keep climbing (or flip the page back to "Identifying…") after Cancel.
pub fn emit_progress<R: Runtime, S: Serialize + Clone>(
    app: &AppHandle<R>,
    meeting_id: &str,
    payload: S,
) {
    if cancel::is_requested(meeting_id) {
        return;
    }
    let _ = app.emit(EVENT_PROGRESS, payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Listener;

    /// Once Cancel is clicked, the hidden compute call's progress callback stays silent, so
    /// nothing keeps climbing (or flips the page back to "Identifying…").
    #[test]
    fn progress_goes_quiet_once_cancelled() {
        let id = "detach-unit-progress";
        let app = tauri::test::mock_app();
        let seen = Arc::new(std::sync::Mutex::new(0usize));
        let sink = Arc::clone(&seen);
        app.listen(EVENT_PROGRESS, move |_| *sink.lock().unwrap() += 1);
        let token = cancel::token(id);

        emit_progress(
            app.handle(),
            id,
            serde_json::json!({ "meeting_id": id, "pct": 10 }),
        );
        assert_eq!(*seen.lock().unwrap(), 1, "a live pass reports progress");
        assert!(cancel::request(id));
        emit_progress(
            app.handle(),
            id,
            serde_json::json!({ "meeting_id": id, "pct": 11 }),
        );
        assert_eq!(*seen.lock().unwrap(), 1, "nothing after Cancel");
        drop(token);
    }

    /// While the cancelled pass is still finishing, a request says so; after, it may start.
    #[test]
    fn a_request_is_refused_only_while_the_cancelled_pass_finishes() {
        let id = "detach-unit-stopping";
        assert!(refuse_if_stopping(id).is_ok());
        let token = cancel::token(id);
        assert!(
            refuse_if_stopping(id).is_ok(),
            "an uncancelled pass is not stopping"
        );
        assert!(cancel::request(id));
        assert_eq!(refuse_if_stopping(id), Err(STILL_STOPPING.to_string()));
        drop(token);
        assert!(refuse_if_stopping(id).is_ok());
    }
}
