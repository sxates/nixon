//! Diarization run lifecycle: claim the per-meeting run slot, spawn the pass,
//! report it on the background-activity queue, and emit the terminal event.
//!
//! This is split out of [`crate::diarization::pipeline`] because the run
//! lifecycle (slot guard, spawn, queue reporting, terminal emit) is a separable
//! concern from the pass itself (model load, decode, cluster, persist) — and
//! `pipeline.rs` is size-capped (specs/0042 WS6 file-size ratchet) with no
//! headroom left for new code.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, OnceLock};

use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::pipeline::{registry_finish, registry_try_begin, run, EVENT_ERROR};
use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;

/// Meetings whose in-flight pass started before a setting it reads changed (specs/0078:
/// the "Who was on the mic?" override), so one more pass runs when it finishes.
fn pending_reruns() -> MutexGuard<'static, HashSet<String>> {
    static PENDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PENDING
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// [`diarize_meeting`], or, when a pass is already in flight, queue one more pass to start
/// as soon as it finishes (success or failure). For a changed input the running pass
/// already read (specs/0078). Returns whether a pass started now; on `false` the caller
/// attaches to the running pass, and the follow-up announces itself with the usual
/// `diarization-progress` events.
///
/// The pending lock is held across the slot check so the running pass can't finish in
/// between and miss the mark: it releases its slot before it looks for a re-run.
pub fn diarize_meeting_or_queue<R: Runtime>(app: AppHandle<R>, meeting_id: String) -> bool {
    let mut pending = pending_reruns();
    if diarize_meeting(app, meeting_id.clone()) {
        return true;
    }
    log::info!("diarization for {meeting_id} is running; queued one more pass after it");
    pending.insert(meeting_id);
    false
}

/// After a pass ends: start the queued re-run, if one was asked for. Returns whether one
/// was pending. If another pass has claimed the slot meanwhile, it started after the
/// change, so it already reads the new input and the re-run is dropped.
fn start_pending_rerun<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> bool {
    if !pending_reruns().remove(meeting_id) {
        return false;
    }
    if diarize_meeting(app.clone(), meeting_id.to_string()) {
        log::info!("diarization for {meeting_id}: started the queued re-run");
    } else {
        log::info!(
            "diarization for {meeting_id}: a newer pass is already running; queued re-run dropped"
        );
    }
    true
}

/// Run offline diarization for a saved meeting on a background task, emitting
/// `diarization-{progress,complete,error}`. Returns immediately after spawning.
///
/// WS3.1 (specs/0029): at most ONE live run per meeting. Returns `true` when a new
/// run was started, `false` when one is already in flight (the caller should attach
/// to the running pass — its progress arrives on the shared events — rather than
/// treat this as a failure).
///
/// Gating on the opt-in setting (and on models-present, for an auto-run) lives at
/// the call site (P1-C); this entry point always runs when called.
pub fn diarize_meeting<R: Runtime>(app: AppHandle<R>, meeting_id: String) -> bool {
    if !registry_try_begin(&meeting_id) {
        log::info!(
            "diarization already running for meeting {meeting_id}; not starting a second run"
        );
        return false;
    }
    tauri::async_runtime::spawn(async move {
        // specs/0063 W3 — the queue is where the user finds out what the machine is busy
        // with, and a diarization pass is the most CPU-hungry thing it does. Registered
        // inside the spawn so a refused duplicate run never creates a phantom row. The
        // registration happens here (not before the spawn) precisely so this async lookup
        // of the meeting's title is available — mirrors `action_items::run_extraction` and
        // `summary::background`, which resolve the same metadata the same way.
        let task = match app.try_state::<crate::llm_activity::LlmActivityState>() {
            Some(state) => {
                let registry = std::sync::Arc::clone(&state.0);
                let title = match app.try_state::<AppState>() {
                    Some(app_state) => {
                        let pool = app_state.db_manager.pool().clone();
                        match MeetingsRepository::get_meeting_metadata(&pool, &meeting_id).await {
                            Ok(Some(meta)) => format!("Identifying speakers — {}", meta.title),
                            _ => "Identifying speakers".to_string(),
                        }
                    }
                    None => "Identifying speakers".to_string(),
                };
                Some(registry.start_for(
                    crate::llm_activity::registry::TaskKind::Diarization,
                    crate::llm_activity::registry::Origin::Background,
                    title,
                    Some(meeting_id.clone()),
                ))
            }
            None => None,
        };

        let outcome = run(app.clone(), meeting_id.clone()).await;
        crate::audio::lifecycle::on_diarization_outcome(&app, &meeting_id, outcome.is_ok()).await;
        if let Some(t) = task {
            t.finish(outcome.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
        }
        if let Err(e) = outcome {
            log::warn!("diarization failed for {meeting_id}: {e:#}");
            // Release the run slot BEFORE emitting so a status query racing the
            // event never sees a stale "running". (The success path does the same
            // inside `run`, just before its `diarization-complete` emit.)
            registry_finish(&meeting_id, "error", 0);
            let _ = app.emit(
                EVENT_ERROR,
                serde_json::json!({ "meeting_id": meeting_id, "error": format!("{e:#}") }),
            );
        }
        start_pending_rerun(&app, &meeting_id);
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_activity::registry::{LlmTaskRegistry, Origin, TaskKind};
    use crate::llm_activity::LlmActivityState;
    use std::sync::Arc;

    /// specs/0066 W2 — the reported "clicking Retry does nothing".
    ///
    /// A queue Retry on a failed diarization reaches `retry_task`, which takes the record
    /// out of the registry (clearing the row and the lamp) and then dispatches here. When a
    /// pass is already in flight for that meeting this refuses to start a second one — and
    /// the first version of the retry arm discarded that `false` and returned `Ok(())`. The
    /// row was gone, the lamp was clear, no work was running, and nothing had been said.
    ///
    /// This test lives in `diarization::launch` rather than beside `retry_task` because
    /// claiming the run slot needs `registry_try_begin`, which is `pub(super)` to this
    /// module tree. Claiming it first means `diarize_meeting` refuses on its first line, so
    /// the test never spawns a real pass.
    #[tokio::test]
    async fn a_retry_refused_as_already_running_is_an_error_not_a_silent_ok() {
        let meeting_id = "m-already-running";
        assert!(
            registry_try_begin(meeting_id),
            "the slot must be free before the test claims it"
        );

        let reg = Arc::new(LlmTaskRegistry::new());
        let task = Arc::clone(&reg).start_for(
            TaskKind::Diarization,
            Origin::Background,
            "Identifying speakers",
            Some(meeting_id.to_string()),
        );
        task.finish(Err("first attempt failed".into()));
        let id = reg.view().history[0].id;

        let app = tauri::test::mock_app();
        app.handle().manage(LlmActivityState(Arc::clone(&reg)));

        let result = crate::llm_activity::retry::retry_task(app.handle(), id).await;

        registry_finish(meeting_id, "done", 100);

        let message = result.expect_err("a refused launch must not report success");
        assert!(
            message.contains("already running"),
            "the message has to say why nothing started, got: {message}"
        );
    }

    /// specs/0078 review: changing "Who was on the mic?" while a pass runs used to do
    /// nothing, because that pass had already read the old setting. It now queues exactly
    /// one follow-up, which the end of the running pass consumes.
    #[tokio::test]
    async fn a_launch_refused_as_running_queues_one_rerun_for_the_end_of_the_pass() {
        let meeting_id = "m-rerun-queued";
        assert!(registry_try_begin(meeting_id), "the test holds the slot");
        let app = tauri::test::mock_app();

        assert!(!diarize_meeting_or_queue(
            app.handle().clone(),
            meeting_id.into()
        ));
        assert!(!diarize_meeting_or_queue(
            app.handle().clone(),
            meeting_id.into()
        ));
        assert!(pending_reruns().contains(meeting_id), "a re-run is queued");

        // The pass ends. The slot is still held here, so the re-run's launch is refused
        // (no real pass spawns), but the queued mark is consumed exactly once.
        assert!(start_pending_rerun(app.handle(), meeting_id));
        assert!(
            !start_pending_rerun(app.handle(), meeting_id),
            "only one re-run"
        );
        assert!(!pending_reruns().contains(meeting_id));

        registry_finish(meeting_id, "done", 100);
    }
}
