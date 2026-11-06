//! specs/0084 W2 — cancelling a speaker-identification pass.
//!
//! Each running pass owns a per-meeting flag ([`token`], created when the run slot is
//! claimed). [`request`] sets it: the queue's Cancel, the meeting page's Cancel, and
//! `api_delete_meeting`. The pass polls it with [`check`] at every stage boundary and once
//! more inside the persist transaction ([`ensure_live`]), so a cancelled or deleted meeting
//! never gets `speakers` rows or voiceprints written for it.
//!
//! In-process, the sherpa compute step itself can't be interrupted (its progress callback's
//! return value is ignored), so a cancel during compute takes effect when compute returns.
//! W3 (the helper process) makes it immediate.
//!
//! Kept out of `pipeline.rs`, which is over the file-size cap.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use anyhow::{anyhow, Context, Result};

static CANCELS: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = LazyLock::new(Default::default);

fn cancels() -> MutexGuard<'static, HashMap<String, Arc<AtomicBool>>> {
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

/// A running pass's cancel flag. Dropping it unregisters the flag (only its own: a newer
/// pass that registered meanwhile keeps its flag).
#[derive(Debug)]
pub struct CancelToken {
    meeting_id: String,
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn is_requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl Drop for CancelToken {
    fn drop(&mut self) {
        let mut map = cancels();
        if map
            .get(&self.meeting_id)
            .is_some_and(|f| Arc::ptr_eq(f, &self.flag))
        {
            map.remove(&self.meeting_id);
        }
    }
}

/// Register a fresh, unset cancel flag for a pass that is starting on `meeting_id`.
/// Call it when the run slot is claimed, before the pass is spawned, so a cancel that
/// arrives in between is not lost.
pub fn token(meeting_id: &str) -> CancelToken {
    let flag = Arc::new(AtomicBool::new(false));
    cancels().insert(meeting_id.to_string(), Arc::clone(&flag));
    CancelToken {
        meeting_id: meeting_id.to_string(),
        flag,
    }
}

/// Ask the running pass on `meeting_id` to stop. Returns `false` when no pass is running
/// (nothing to cancel). Idempotent.
pub fn request(meeting_id: &str) -> bool {
    match cancels().get(meeting_id) {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

/// Whether a cancel has been requested for the pass running on `meeting_id`.
pub fn is_requested(meeting_id: &str) -> bool {
    cancels()
        .get(meeting_id)
        .is_some_and(|f| f.load(Ordering::SeqCst))
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
/// meeting page's Cancel). Returns at once; the pass stops at its next stage boundary and
/// ends with a quiet `diarization-error` carrying `cancelled: true`.
#[tauri::command]
pub async fn api_cancel_diarization(meeting_id: String) -> Result<CancelDiarizationDto, String> {
    let cancelled = request(&meeting_id);
    if cancelled {
        log::info!("diarization for {meeting_id}: cancel requested");
    }
    Ok(CancelDiarizationDto { cancelled })
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
}
