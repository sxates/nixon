//! specs/0073 W1 — the per-meeting **folder lease**.
//!
//! Exclusive, per-meeting right to touch a meeting's recording folder on disk. Every job that
//! reads or writes inside a meeting folder, or rewrites `meetings.folder_path`, holds it:
//! the recording saver (start → finalization), retranscription, diarization, the retention
//! sweep, the audio compressor, the recordings mover, the transcript save's `folder_path`
//! write-back, meeting delete and interrupted-recording discard. It is the one exclusion
//! primitive for meeting folders; never define a second.
//!
//! The contract has two halves:
//!
//! 1. **Take the lease.** Interactive and foreground jobs [`acquire`] (wait). Periodic
//!    background jobs [`try_acquire`] and skip the meeting when it is held; the next tick
//!    picks it up.
//! 2. **Re-read `folder_path` after acquiring.** The folder may have moved while the job
//!    waited, so a path captured earlier (from the frontend, a list query, a previous tick) is
//!    only a hint. [`reread_folder_path`] / [`leased_folder_path`] are the re-read.
//!
//! Only the mover holds more than one lease, and it takes them in sorted meeting-id order,
//! so there is no lock-ordering hazard between meetings (ADR-0013).
//! A holder must also never wait for the engine-lifecycle lock while holding a lease: a
//! recording start holds that lock while it waits for its own meeting's lease.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Who holds a meeting's folder lease. Used for logs and for the mover's "waiting for …"
/// status, so every job that can hold a lease has a variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LeaseHolder {
    /// The recording saver, from folder creation through finalization.
    Recording,
    /// A batch retranscription of the meeting's audio.
    Retranscription,
    /// The offline speaker-diarization pass (audio reads).
    Diarization,
    /// The retention sweep deleting expired media.
    Retention,
    /// Post-processing audio compression (specs/0072).
    Compression,
    /// The recordings mover (specs/0073 W2).
    Mover,
    /// A transcript save writing `meetings.folder_path` back.
    FolderPathWrite,
    /// Deleting a meeting (removes its folder).
    Delete,
    /// Discarding an interrupted recording (rewrites its `metadata.json` status).
    Discard,
}

impl LeaseHolder {
    /// A short, user-readable name for the job ("Waiting for transcription to finish…").
    pub fn label(self) -> &'static str {
        match self {
            LeaseHolder::Recording => "recording",
            LeaseHolder::Retranscription => "transcription",
            LeaseHolder::Diarization => "speaker identification",
            LeaseHolder::Retention => "audio cleanup",
            LeaseHolder::Compression => "audio compression",
            LeaseHolder::Mover => "moving recordings",
            LeaseHolder::FolderPathWrite => "saving the transcript",
            LeaseHolder::Delete => "deleting the meeting",
            LeaseHolder::Discard => "discarding the recording",
        }
    }
}

/// One meeting's slot. `users` counts live leases plus pending waiters, so the entry can be
/// reaped exactly when nobody holds or waits for it.
struct Entry {
    lock: Arc<AsyncMutex<()>>,
    holder: Option<LeaseHolder>,
    users: usize,
}

static LEASES: LazyLock<Mutex<HashMap<String, Entry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Lock the table, recovering from poisoning: the table is bookkeeping, and a panicked
/// holder must never wedge every later job.
fn table() -> MutexGuard<'static, HashMap<String, Entry>> {
    LEASES.lock().unwrap_or_else(|p| p.into_inner())
}

/// Register one more user of `meeting_id`'s slot and return its mutex.
fn join(meeting_id: &str) -> Arc<AsyncMutex<()>> {
    let mut map = table();
    let entry = map.entry(meeting_id.to_string()).or_insert_with(|| Entry {
        lock: Arc::new(AsyncMutex::new(())),
        holder: None,
        users: 0,
    });
    entry.users += 1;
    entry.lock.clone()
}

/// Drop one user of `meeting_id`'s slot, clearing the holder if `was_holder`, and reap the
/// entry once nobody holds or waits for it.
fn leave(meeting_id: &str, was_holder: bool) {
    let mut map = table();
    let Some(entry) = map.get_mut(meeting_id) else {
        return;
    };
    if was_holder {
        entry.holder = None;
    }
    entry.users = entry.users.saturating_sub(1);
    if entry.users == 0 {
        map.remove(meeting_id);
    }
}

fn set_holder(meeting_id: &str, holder: LeaseHolder) {
    if let Some(entry) = table().get_mut(meeting_id) {
        entry.holder = Some(holder);
    }
}

/// The exclusive right to touch one meeting's recording folder. Released on drop.
///
/// A holder MUST re-read `meetings.folder_path` after acquiring: the folder may have moved.
pub struct FolderLease {
    meeting_id: String,
    holder: LeaseHolder,
    // Field order matters only for readability: `Drop` below clears the bookkeeping first,
    // then this guard is dropped and the next waiter wakes.
    _guard: OwnedMutexGuard<()>,
}

impl FolderLease {
    pub fn meeting_id(&self) -> &str {
        &self.meeting_id
    }

    pub fn holder(&self) -> LeaseHolder {
        self.holder
    }
}

impl std::fmt::Debug for FolderLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FolderLease")
            .field("meeting_id", &self.meeting_id)
            .field("holder", &self.holder)
            .finish()
    }
}

impl Drop for FolderLease {
    fn drop(&mut self) {
        leave(&self.meeting_id, true);
        log::debug!(
            "folder lease released: {} ({:?})",
            self.meeting_id,
            self.holder
        );
    }
}

/// A registered waiter. Un-registers itself if the waiting future is dropped before it gets
/// the lease (a cancelled or timed-out wait), so an abandoned wait never pins the entry.
struct Waiter<'a> {
    meeting_id: &'a str,
    armed: bool,
}

impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        if self.armed {
            leave(self.meeting_id, false);
        }
    }
}

/// Take `meeting_id`'s folder lease, waiting for the current holder to finish.
///
/// For interactive and foreground jobs. Re-read `folder_path` afterwards.
pub async fn acquire(meeting_id: &str, holder: LeaseHolder) -> FolderLease {
    if let Some(lease) = try_acquire(meeting_id, holder) {
        return lease;
    }
    if let Some(current) = current_holder(meeting_id) {
        log::info!("folder lease: {holder:?} for meeting {meeting_id} is waiting for {current:?}");
    }
    let lock = join(meeting_id);
    let mut waiter = Waiter {
        meeting_id,
        armed: true,
    };
    let guard = lock.lock_owned().await;
    waiter.armed = false; // ownership of the `users` slot passes to the lease
    set_holder(meeting_id, holder);
    log::debug!("folder lease acquired: {meeting_id} ({holder:?})");
    FolderLease {
        meeting_id: meeting_id.to_string(),
        holder,
        _guard: guard,
    }
}

/// [`acquire`], but give up after `wait`. On timeout returns the holder that kept the lease
/// (or `holder` itself if it was released just as the wait ran out), so the caller can say
/// what it was waiting for.
pub async fn acquire_within(
    meeting_id: &str,
    holder: LeaseHolder,
    wait: Duration,
) -> Result<FolderLease, LeaseHolder> {
    match tokio::time::timeout(wait, acquire(meeting_id, holder)).await {
        Ok(lease) => Ok(lease),
        Err(_) => Err(current_holder(meeting_id).unwrap_or(holder)),
    }
}

/// Take `meeting_id`'s folder lease only if nobody holds it. For periodic background jobs,
/// which skip a busy meeting and pick it up on their next tick.
pub fn try_acquire(meeting_id: &str, holder: LeaseHolder) -> Option<FolderLease> {
    let mut map = table();
    let entry = map.entry(meeting_id.to_string()).or_insert_with(|| Entry {
        lock: Arc::new(AsyncMutex::new(())),
        holder: None,
        users: 0,
    });
    match entry.lock.clone().try_lock_owned() {
        Ok(guard) => {
            entry.users += 1;
            entry.holder = Some(holder);
            drop(map);
            log::debug!("folder lease acquired: {meeting_id} ({holder:?})");
            Some(FolderLease {
                meeting_id: meeting_id.to_string(),
                holder,
                _guard: guard,
            })
        }
        Err(_) => {
            if entry.users == 0 {
                map.remove(meeting_id);
            }
            None
        }
    }
}

/// Who holds `meeting_id`'s lease right now, if anyone. For the UI's "waiting for …".
pub fn current_holder(meeting_id: &str) -> Option<LeaseHolder> {
    table().get(meeting_id).and_then(|e| e.holder)
}

/// Is any meeting's lease currently held by a job of `kind`?
pub fn any_held_by(kind: LeaseHolder) -> bool {
    table().values().any(|e| e.holder == Some(kind))
}

/// The re-read half of the lease contract: the meeting's stored `folder_path`, trimmed, or
/// `None` when it is NULL/empty.
pub async fn reread_folder_path(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let stored: Option<Option<String>> =
        sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await?;
    Ok(stored
        .flatten()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty()))
}

/// The folder a lease holder should use: the DB's `folder_path` when it is set, otherwise
/// `caller_path` (a path the caller was handed earlier). Logs when the two disagree, which
/// is exactly the case where the caller's copy went stale (a move finished while it waited).
///
/// A missing database (first-launch window) or a failed read falls back to `caller_path`.
pub async fn leased_folder_path<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
    caller_path: Option<&str>,
) -> Option<String> {
    let caller = caller_path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return caller;
    };
    match reread_folder_path(state.db_manager.pool(), meeting_id).await {
        Ok(Some(stored)) => {
            if let Some(c) = caller.as_deref().filter(|c| *c != stored) {
                log::info!(
                    "folder lease: meeting {meeting_id} was handed {c:?} but its folder is \
                     {stored:?}; using the stored folder"
                );
            }
            Some(stored)
        }
        Ok(None) => caller,
        Err(e) => {
            log::warn!("folder lease: could not re-read folder_path for {meeting_id}: {e}");
            caller
        }
    }
}

/// How long a recording start waits for its meeting's folder to be free before it gives
/// up with an explanation. Covers a same-volume move; a long job (a retranscription of this
/// meeting) gets an error the user can act on instead of a start button that hangs.
pub const RECORDING_LEASE_WAIT: Duration = Duration::from_secs(15);

/// Recording-start adoption: take the meeting's [`LeaseHolder::Recording`] lease and, when
/// resuming into an existing folder, re-read that folder from the DB (the frontend's copy
/// may predate a move). A recording with no meeting row takes no lease.
///
/// Returns the lease (handed to the saver, which holds it through finalization) and the
/// resume folder to use.
pub async fn lease_for_recording_start<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: Option<&str>,
    resume_folder: Option<String>,
) -> Result<(Option<FolderLease>, Option<String>), String> {
    let Some(meeting_id) = meeting_id.filter(|id| !id.trim().is_empty()) else {
        return Ok((None, resume_folder));
    };
    let lease = acquire_within(meeting_id, LeaseHolder::Recording, RECORDING_LEASE_WAIT)
        .await
        .map_err(|busy| {
            format!(
                "This meeting's recordings are busy with {} right now. Try again when that \
                 finishes.",
                busy.label()
            )
        })?;
    let resume_folder = match resume_folder {
        Some(folder) => {
            // specs/0072: a resumed recording adds a segment, so the meeting is pending again.
            super::lifecycle::reset_for_resume(app, meeting_id).await;
            leased_folder_path(app, meeting_id, Some(&folder)).await
        }
        None => None,
    };
    Ok((Some(lease), resume_folder))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests share the process-wide table, so each uses its own meeting id.
    fn tracked(meeting_id: &str) -> bool {
        table().contains_key(meeting_id)
    }

    #[tokio::test]
    async fn a_held_lease_excludes_a_second_holder() {
        let id = "lease-test-exclusion";
        let first = acquire(id, LeaseHolder::Mover).await;
        assert!(try_acquire(id, LeaseHolder::Retention).is_none());
        assert_eq!(current_holder(id), Some(LeaseHolder::Mover));
        drop(first);
        assert!(try_acquire(id, LeaseHolder::Retention).is_some());
    }

    #[tokio::test]
    async fn a_waiter_wakes_when_the_holder_drops() {
        let id = "lease-test-waiter";
        let first = acquire(id, LeaseHolder::Retranscription).await;
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            let lease = acquire(id, LeaseHolder::Mover).await;
            tx.send(current_holder(id)).unwrap();
            drop(lease);
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err(), "the waiter must not get in early");
        drop(first);
        assert_eq!(rx.await.unwrap(), Some(LeaseHolder::Mover));
        waiter.await.unwrap();
        assert!(!tracked(id));
    }

    #[tokio::test]
    async fn try_acquire_returns_none_under_contention() {
        let id = "lease-test-try";
        let _held = acquire(id, LeaseHolder::Mover).await;
        assert!(try_acquire(id, LeaseHolder::Compression).is_none());
        assert!(try_acquire(id, LeaseHolder::Retention).is_none());
    }

    #[tokio::test]
    async fn different_meetings_do_not_block_each_other() {
        let _a = acquire("lease-test-a", LeaseHolder::Mover).await;
        assert!(try_acquire("lease-test-b", LeaseHolder::Retention).is_some());
    }

    #[tokio::test]
    async fn uncontended_entries_are_reaped() {
        let id = "lease-test-reap";
        drop(acquire(id, LeaseHolder::Diarization).await);
        assert!(!tracked(id));
        drop(try_acquire(id, LeaseHolder::Retention));
        assert!(!tracked(id));

        // A failed try_acquire leaves nothing behind once the holder is gone.
        let held = acquire(id, LeaseHolder::Mover).await;
        assert!(try_acquire(id, LeaseHolder::Retention).is_none());
        drop(held);
        assert!(!tracked(id));
        assert_eq!(current_holder(id), None);
    }

    #[tokio::test]
    async fn an_abandoned_wait_is_reaped_and_reports_the_holder() {
        let id = "lease-test-timeout";
        let held = acquire(id, LeaseHolder::Retranscription).await;
        let busy = acquire_within(id, LeaseHolder::Recording, Duration::from_millis(30))
            .await
            .unwrap_err();
        assert_eq!(busy, LeaseHolder::Retranscription);
        drop(held);
        assert!(!tracked(id), "a timed-out waiter must not pin the entry");
        assert!(
            acquire_within(id, LeaseHolder::Recording, Duration::from_millis(30))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn any_held_by_sees_only_live_holders() {
        // Compression: the one holder kind no other unit test in this crate takes, so the
        // negative assertion can't race a parallel test.
        let lease = acquire("lease-test-kind", LeaseHolder::Compression).await;
        assert!(any_held_by(LeaseHolder::Compression));
        drop(lease);
        assert!(!any_held_by(LeaseHolder::Compression));
    }
}
