//! specs/0084 W2 — cancelling a speaker-identification pass.
//!
//! Each running pass owns a per-meeting flag ([`token`], created when the run slot is
//! claimed). [`request`] sets it: the queue's Cancel, the meeting page's Cancel, and
//! `api_delete_meeting`. The pass polls it with [`check`] at every stage boundary and once
//! more inside the persist transaction ([`ensure_live`]), so a cancelled or deleted meeting
//! never gets `speakers` rows or voiceprints written for it.
//!
//! In-process, the sherpa compute step itself can't be interrupted (its progress callback's
//! return value is ignored). So a cancel ends the pass's *visible* state at once
//! (`detach.rs`) and the compute call finishes unseen; when it returns, the pass ends as
//! cancelled and writes nothing. W3 (the helper process) makes the stop itself immediate.
//!
//! Kept out of `pipeline.rs`, which is over the file-size cap.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use anyhow::{anyhow, Context, Result};

use crate::llm_activity::registry::TaskHandle;

static CANCELS: LazyLock<Mutex<HashMap<String, Arc<PassCancel>>>> = LazyLock::new(Default::default);

fn cancels() -> MutexGuard<'static, HashMap<String, Arc<PassCancel>>> {
    CANCELS.lock().unwrap_or_else(|p| p.into_inner())
}

/// The error a cancelled pass returns. [`is_cancelled`] finds it anywhere in an
/// `anyhow` chain, so `.context(..)` wrapping doesn't hide it from the outcome routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("speaker identification was cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Whether `err` (or anything it wraps) is [`Cancelled`].
pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.downcast_ref::<Cancelled>().is_some())
}

/// One running pass: its cancel flag, plus what the user sees of it. The visible part (the
/// queue row, the "Identifying…" state) can end before the pass does: a cancel during
/// sherpa's compute step ends it at once while the compute call finishes unseen
/// (`detach.rs`).
#[derive(Debug, Default)]
pub struct PassCancel {
    flag: AtomicBool,
    visible: Mutex<Visible>,
}

#[derive(Default)]
struct Visible {
    /// The visible state was finished as cancelled. Only the first finish runs.
    finished: bool,
    /// The pass's activity-queue row, kept here so whichever ending comes first finishes it.
    task: Option<TaskHandle>,
}

impl std::fmt::Debug for Visible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Visible")
            .field("finished", &self.finished)
            .field("has_task", &self.task.is_some())
            .finish()
    }
}

impl PassCancel {
    fn visible(&self) -> MutexGuard<'_, Visible> {
        self.visible.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn is_requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Claim the one visible finish: `None` when it already happened, otherwise the queue
    /// row to finish (if the pass had registered one yet).
    pub fn claim_visible_finish(&self) -> Option<Option<TaskHandle>> {
        let mut v = self.visible();
        if v.finished {
            return None;
        }
        v.finished = true;
        Some(v.task.take())
    }

    /// Whether a cancel already finished the visible state.
    pub fn visibly_finished(&self) -> bool {
        self.visible().finished
    }
}

/// A running pass's cancel handle. Dropping it unregisters the pass (only its own: a newer
/// pass that registered meanwhile keeps its entry).
#[derive(Debug)]
pub struct CancelToken {
    meeting_id: String,
    pass: Arc<PassCancel>,
}

impl CancelToken {
    pub fn is_requested(&self) -> bool {
        self.pass.is_requested()
    }

    pub fn pass(&self) -> &Arc<PassCancel> {
        &self.pass
    }

    /// Hand the pass's queue row to the token. When a cancel already finished the visible
    /// state (it arrived before the row existed), the row is finished as cancelled at once.
    pub fn attach_task(&self, task: TaskHandle) {
        let mut v = self.pass.visible();
        if v.finished {
            drop(v);
            task.finish_skipped(super::launch::CANCELLED_REASON);
        } else {
            v.task = Some(task);
        }
    }

    /// Take the queue row back to finish it. `None` once a cancel finished it.
    pub fn take_task(&self) -> Option<TaskHandle> {
        self.pass.visible().task.take()
    }
}

impl Drop for CancelToken {
    fn drop(&mut self) {
        let mut map = cancels();
        if map
            .get(&self.meeting_id)
            .is_some_and(|p| Arc::ptr_eq(p, &self.pass))
        {
            map.remove(&self.meeting_id);
        }
    }
}

/// Register a fresh, unset cancel flag for a pass that is starting on `meeting_id`.
/// Call it when the run slot is claimed, before the pass is spawned, so a cancel that
/// arrives in between is not lost.
pub fn token(meeting_id: &str) -> CancelToken {
    let pass = Arc::new(PassCancel::default());
    cancels().insert(meeting_id.to_string(), Arc::clone(&pass));
    CancelToken {
        meeting_id: meeting_id.to_string(),
        pass,
    }
}

/// Ask the running pass on `meeting_id` to stop, returning it; `None` when no pass is
/// running (nothing to cancel). Idempotent.
pub fn request_pass(meeting_id: &str) -> Option<Arc<PassCancel>> {
    let pass = cancels().get(meeting_id).cloned()?;
    pass.flag.store(true, Ordering::SeqCst);
    Some(pass)
}

/// [`request_pass`] for callers that only need to know whether a pass was running.
pub fn request(meeting_id: &str) -> bool {
    request_pass(meeting_id).is_some()
}

/// Whether a cancel has been requested for the pass running on `meeting_id`.
pub fn is_requested(meeting_id: &str) -> bool {
    cancels().get(meeting_id).is_some_and(|p| p.is_requested())
}

/// A cancelled pass on `meeting_id` has not returned yet. Its visible state may be over,
/// but no new pass may start until it ends: the old pass's stage checks and persist guard
/// look its flag up by meeting id, so a newer pass's unset flag would let it write.
pub fn is_stopping(meeting_id: &str) -> bool {
    is_requested(meeting_id)
}

/// A stage boundary: `Err(Cancelled)` once a cancel was requested.
pub fn check(meeting_id: &str) -> Result<()> {
    if is_requested(meeting_id) {
        log::info!("diarization for {meeting_id}: cancelled; stopping before the next stage");
        return Err(anyhow!(Cancelled));
    }
    Ok(())
}

/// The persist guard: `Err(Cancelled)` when the meeting row is gone (deleted mid-pass) or a
/// cancel was requested. Run it inside the write transaction, before the first write, so a
/// pass can never leave orphan `speakers` rows or voiceprints behind.
pub async fn ensure_live<'e, E>(executor: E, meeting_id: &str) -> Result<()>
where
    E: sqlx::SqliteExecutor<'e>,
{
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(executor)
        .await
        .context("check the meeting still exists")?;
    if exists.is_none() {
        log::info!("diarization for {meeting_id}: the meeting was deleted; writing nothing");
        return Err(anyhow!(Cancelled));
    }
    check(meeting_id)
}

/// Result of [`api_cancel_diarization`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct CancelDiarizationDto {
    /// `false` when no pass was running for the meeting.
    pub cancelled: bool,
}

/// Cancel the speaker-identification pass running on `meeting_id` (the queue row's and the
/// meeting page's Cancel). When this returns `cancelled: true` the pass is over as far as
/// the user can see: its queue row is finished as skipped and the quiet `diarization-error`
/// (`cancelled: true`) has been emitted. A pass inside sherpa's compute step keeps running
/// unseen until that call returns, then writes nothing (`detach.rs`).
#[tauri::command]
pub async fn api_cancel_diarization<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    meeting_id: String,
) -> Result<CancelDiarizationDto, String> {
    let Some(pass) = request_pass(&meeting_id) else {
        return Ok(CancelDiarizationDto { cancelled: false });
    };
    log::info!("diarization for {meeting_id}: cancel requested");
    super::detach::finish_cancelled(&app, &meeting_id, &pass, super::detach::Db::KeepMarker).await;
    Ok(CancelDiarizationDto { cancelled: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_reaches_only_a_running_pass() {
        let id = "cancel-unit-running";
        assert!(!request(id), "nothing is running yet");
        assert!(check(id).is_ok());

        let t = token(id);
        assert!(check(id).is_ok(), "a fresh token is unset");
        assert!(request(id));
        assert!(t.is_requested());
        let err = check(id).unwrap_err();
        assert!(is_cancelled(&err));
        assert!(is_cancelled(&err.context("wrapped")));

        drop(t);
        assert!(!request(id), "the finished pass's flag is gone");
        assert!(check(id).is_ok());
    }

    #[test]
    fn an_old_token_never_removes_a_newer_pass_flag() {
        let id = "cancel-unit-overlap";
        let old = token(id);
        let new = token(id);
        drop(old);
        assert!(request(id), "the newer pass is still cancellable");
        assert!(new.is_requested());
    }

    #[test]
    fn an_ordinary_error_is_not_a_cancel() {
        assert!(!is_cancelled(&anyhow!("model missing")));
    }

    /// The visible finish runs once, and a queue row that registers after it (the Cancel
    /// beat the pass's spawn) is finished as a skip at once, never left "running".
    #[test]
    fn the_visible_finish_runs_once_and_catches_a_late_queue_row() {
        use crate::llm_activity::registry::{LlmTaskRegistry, Origin, TaskKind, TaskOutcome};
        let id = "cancel-unit-visible";
        let t = token(id);
        let pass = request_pass(id).expect("running");
        assert!(is_stopping(id));
        assert!(pass.claim_visible_finish().is_some());
        assert!(
            pass.claim_visible_finish().is_none(),
            "only the first finish runs"
        );
        assert!(pass.visibly_finished());

        let reg = Arc::new(LlmTaskRegistry::new());
        t.attach_task(Arc::clone(&reg).start_for(
            TaskKind::Diarization,
            Origin::Background,
            "Identifying speakers",
            Some(id.to_string()),
        ));
        let view = reg.view();
        assert!(view.running.is_empty(), "the late row is finished at once");
        assert!(matches!(
            view.history[0].outcome,
            TaskOutcome::Skipped { .. }
        ));
        assert!(!view.has_failure);
        assert!(t.take_task().is_none());

        drop(t);
        assert!(!is_stopping(id), "the pass returned");
    }
}
