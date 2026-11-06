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

use super::outcome::PassOutcome;
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

/// specs/0084 W1: whether an automatic caller may start a pass on this meeting (none may
/// once a pass has died mid-run on it). An unreadable history allows it, as before 0084.
pub async fn automatic_pass_allowed<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> bool {
    let Some(app_state) = app.try_state::<AppState>() else {
        return true;
    };
    let pool = app_state.db_manager.pool();
    match crate::audio::lifecycle::state::automatic_pass_allowed(pool, meeting_id).await {
        Ok(true) => true,
        Ok(false) => {
            log::warn!(
                "diarization for {meeting_id}: a previous pass crashed the app; not starting \
                 one automatically (Retry or Identify speakers runs it in safe mode)"
            );
            false
        }
        Err(e) => {
            log::warn!("diarization for {meeting_id}: {e:#}");
            true
        }
    }
}

/// Whether each meeting's latest pass this session ran in safe mode (specs/0084 W1). For
/// diagnostics and tests; the log line in `pipeline.rs` is the user-visible evidence.
fn safe_mode_runs() -> MutexGuard<'static, std::collections::HashMap<String, bool>> {
    static RUNS: OnceLock<Mutex<std::collections::HashMap<String, bool>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Did the latest pass for this meeting (this session) run in safe mode?
pub fn last_pass_safe_mode(meeting_id: &str) -> Option<bool> {
    safe_mode_runs().get(meeting_id).copied()
}

/// The queue row's skip reason, and the `diarization-error` text, for a cancelled pass.
pub const CANCELLED_REASON: &str = "Cancelled";
/// The run-status stage a cancelled pass ends on (`api_diarization_status`).
pub const STAGE_CANCELLED: &str = "cancelled";

/// Whether the meeting's row still exists: a pass whose meeting was deleted mid-run ends
/// quietly, whatever error it hit. An unreadable answer counts as present (fail loudly).
async fn meeting_exists<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> bool {
    let Some(app_state) = app.try_state::<AppState>() else {
        return true;
    };
    sqlx::query_scalar::<_, i64>("SELECT 1 FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(app_state.db_manager.pool())
        .await
        .map(|row| row.is_some())
        .unwrap_or(true)
}

/// The activity-queue label for a meeting's speaker-identification row.
pub(crate) async fn queue_label<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> String {
    if let Some(app_state) = app.try_state::<AppState>() {
        let pool = app_state.db_manager.pool();
        if let Ok(Some(meta)) = MeetingsRepository::get_meeting_metadata(pool, meeting_id).await {
            return format!("Identifying speakers — {}", meta.title);
        }
    }
    "Identifying speakers".to_string()
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
    // specs/0084 W2: registered with the slot, before the spawn, so a cancel that arrives
    // before the task runs is not lost.
    let cancel = super::cancel::token(&meeting_id);
    tauri::async_runtime::spawn(async move {
        // specs/0063 W3 — the queue is where the user finds out what the machine is busy
        // with, and a diarization pass is the most CPU-hungry thing it does. Registered
        // inside the spawn so a refused duplicate run never creates a phantom row. The
        // registration happens here (not before the spawn) precisely so this async lookup
        // of the meeting's title is available — mirrors `action_items::run_extraction` and
        // `summary::background`, which resolve the same metadata the same way.
        let task = match app.try_state::<crate::llm_activity::LlmActivityState>() {
            Some(state) => Some(std::sync::Arc::clone(&state.0).start_for(
                crate::llm_activity::registry::TaskKind::Diarization,
                crate::llm_activity::registry::Origin::Background,
                queue_label(&app, &meeting_id).await,
                Some(meeting_id.clone()),
            )),
            None => None,
        };

        // specs/0084 W1: stamp the crash-loop marker before the pass touches audio. If the
        // app dies mid-pass, the next launch finds it and fails the meeting instead of
        // starting the same crash again. A counted crash runs this pass in safe mode.
        let safe_mode = match app.try_state::<AppState>() {
            Some(app_state) => {
                let pool = app_state.db_manager.pool();
                match crate::audio::lifecycle::state::mark_speaker_id_started(pool, &meeting_id)
                    .await
                {
                    Ok(crashes) => crashes >= 1,
                    Err(e) => {
                        log::warn!("diarization for {meeting_id}: {e:#}");
                        false
                    }
                }
            }
            None => false,
        };

        safe_mode_runs().insert(meeting_id.clone(), safe_mode);
        let result = run(app.clone(), meeting_id.clone(), safe_mode).await;
        let outcome = PassOutcome::from_result(&result, meeting_exists(&app, &meeting_id).await);
        crate::audio::lifecycle::on_diarization_outcome(&app, &meeting_id, &outcome).await;
        drop(cancel); // the pass is over: a later cancel has nothing to stop
        match outcome {
            PassOutcome::Ok => {
                if let Some(t) = task {
                    t.finish(Ok(()));
                }
            }
            PassOutcome::Cancelled => {
                log::info!("diarization for {meeting_id}: cancelled");
                if let Some(t) = task {
                    t.finish_skipped(CANCELLED_REASON);
                }
                registry_finish(&meeting_id, STAGE_CANCELLED, 0);
                // A cancel means "stop": the queued follow-up would undo it.
                pending_reruns().remove(&meeting_id);
                let _ = app.emit(
                    EVENT_ERROR,
                    serde_json::json!({
                        "meeting_id": meeting_id,
                        "error": CANCELLED_REASON,
                        "cancelled": true,
                    }),
                );
                return;
            }
            PassOutcome::Failed(failure) => {
                log::warn!("diarization failed for {meeting_id}: {}", failure.message);
                if let Some(t) = task {
                    t.finish(Err(failure.message.clone()));
                }
                // Release the run slot BEFORE emitting so a status query racing the
                // event never sees a stale "running". (The success path does the same
                // inside `run`, just before its `diarization-complete` emit.)
                registry_finish(&meeting_id, "error", 0);
                let _ = app.emit(
                    EVENT_ERROR,
                    serde_json::json!({
                        "meeting_id": meeting_id,
                        "error": failure.message,
                        "cancelled": false,
                    }),
                );
            }
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
