//! specs/0084 W1 — the speaker-identification crash-loop breaker, app side.
//!
//! A native crash in sherpa-onnx kills the app before `on_diarization_outcome` runs, so the
//! meeting used to stay pending and the startup reconcile started the same pass, and the
//! same crash, on every launch. `state::mark_speaker_id_started` now stamps a marker before
//! every pass and every ending clears it, so a marker still set at launch means the app died
//! mid-pass. [`break_interrupted_passes`] runs before the reconcile lists anything: it fails
//! those meetings, announces each, and puts a failed row on the activity queue so its Retry
//! (`llm_activity/retry.rs`, which runs in safe mode once a crash is counted) is one click.

use tauri::{AppHandle, Manager, Runtime};

use super::hooks::{emit_state, pool};
use super::policy::AudioState;
use super::state;
use crate::llm_activity::registry::{Origin, TaskKind};
use crate::llm_activity::LlmActivityState;

/// The queue row's message for a pass that died with the app.
pub const INTERRUPTED_MESSAGE: &str =
    "Speaker identification stopped unexpectedly. Retry, or leave it — your transcript is saved.";

/// A graceful quit (`RunEvent::Exit`): the passes still running end on purpose, so their
/// markers go and the next launch's reconcile finishes them as plain pending meetings. Call
/// before the database is closed.
pub async fn clear_markers_on_quit<R: Runtime>(app: &AppHandle<R>) {
    let running = crate::diarization::pipeline::running_ids();
    if running.is_empty() {
        return;
    }
    let Some(pool) = pool(app) else { return };
    match state::clear_speaker_id_markers(&pool, &running).await {
        Ok(()) => log::info!(
            "Quit during speaker identification for {} meeting(s); they finish next launch",
            running.len()
        ),
        Err(e) => log::warn!("Speaker identification breaker: {e:#}"),
    }
}

/// Fail every meeting whose speaker-identification pass died with the app. Returns the
/// meetings now `failed`. Best-effort: an error is logged and nothing is failed.
pub async fn break_interrupted_passes<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let Some(pool) = pool(app) else {
        return Vec::new();
    };
    let interrupted = match state::fail_interrupted_speaker_id(&pool).await {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("Speaker identification breaker: {e:#}");
            return Vec::new();
        }
    };
    let mut failed = Vec::new();
    for pass in interrupted {
        let id = pass.meeting_id;
        log::warn!(
            "Speaker identification for meeting {id} stopped with the app (crash #{}); not \
             restarting it. The next run, if the user retries, is in safe mode.",
            pass.crashes
        );
        if !pass.failed {
            continue; // a re-run of an already processed meeting: its state stands
        }
        emit_state(app, &id, AudioState::Failed);
        if let Some(activity) = app.try_state::<LlmActivityState>() {
            let label = crate::diarization::launch::queue_label(app, &id).await;
            std::sync::Arc::clone(&activity.0)
                .start_for(
                    TaskKind::Diarization,
                    Origin::Background,
                    label,
                    Some(id.clone()),
                )
                .finish(Err(INTERRUPTED_MESSAGE.to_string()));
        }
        failed.push(id);
    }
    failed
}
