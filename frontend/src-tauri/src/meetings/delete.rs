//! Deleting a meeting (specs/0073, specs/0084 W2): the DB rows, then its recording folder.
//!
//! The folder is removed under the meeting's folder lease, so a delete never pulls a folder
//! out from under a retranscription, a move or speaker identification. Speaker
//! identification can hold that lease for minutes, so a delete no longer waits for it:
//! it cancels the pass, waits at most [`DELETE_LEASE_WAIT`], and if the pass still holds
//! the lease, deletes the rows now and removes the folder once the lease is released. The
//! pass's persist guard (`diarization::cancel::ensure_live`) sees the row is gone and
//! writes nothing. Other lease holders are still waited for, as before.

use std::time::Duration;

use log::{error as log_error, info as log_info, warn as log_warn};
use sqlx::SqlitePool;

use crate::audio::folder_lease::{self, FolderLease, LeaseHolder};
use crate::database::repositories::meeting::MeetingsRepository;

/// How long a delete waits for a speaker-identification pass to let go of the folder.
pub const DELETE_LEASE_WAIT: Duration = Duration::from_secs(3);

/// What a delete got from the folder lease.
enum DeleteLease {
    /// The lease is held: delete rows and folder now, as before 0084.
    Held(FolderLease),
    /// Speaker identification kept it past the wait: delete the rows now, the folder later.
    Deferred,
}

async fn take_delete_lease(meeting_id: &str, wait: Duration) -> DeleteLease {
    if crate::diarization::cancel::request(meeting_id) {
        log_info!("Delete of meeting {meeting_id}: cancelled its speaker identification");
    }
    match folder_lease::acquire_within(meeting_id, LeaseHolder::Delete, wait).await {
        Ok(lease) => DeleteLease::Held(lease),
        Err(LeaseHolder::Diarization) => {
            log_info!(
                "Delete of meeting {meeting_id}: speaker identification still holds the \
                 folder; deleting the meeting now and its folder when the pass lets go"
            );
            DeleteLease::Deferred
        }
        // Out of scope for 0084 (retranscription, a move, ...): wait for it as before.
        Err(_) => DeleteLease::Held(folder_lease::acquire(meeting_id, LeaseHolder::Delete).await),
    }
}

/// Delete `meeting_id` and its recording folder. `Ok(false)` when no such meeting exists.
/// Returns once the rows are deleted; it never waits for speaker identification (see the
/// module docs). [`DELETE_LEASE_WAIT`] in the app; tests pass a shorter `wait`.
pub async fn delete_meeting_and_folder(
    pool: &SqlitePool,
    meeting_id: &str,
    wait: Duration,
) -> Result<bool, String> {
    let lease = take_delete_lease(meeting_id, wait).await;

    // Read the folder now: under the lease it is the folder's current location, and on the
    // deferred path the mover can't move it either while speaker identification holds it.
    // If the lookup fails the DB delete still runs.
    let folder_path: Option<String> =
        sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
            .unwrap_or_else(|e| {
                log_warn!("Could not read folder_path for meeting {meeting_id} before delete: {e}");
                None
            })
            .flatten();

    match MeetingsRepository::delete_meeting(pool, meeting_id).await {
        Ok(true) => {
            log_info!("Successfully deleted meeting {meeting_id}");
            // DB rows are gone. A filesystem error removing the folder must NOT fail the
            // command or roll back the committed DB delete: it is logged.
            match lease {
                DeleteLease::Held(_lease) => {
                    delete_recording_folder_best_effort(meeting_id, folder_path.as_deref()).await
                }
                DeleteLease::Deferred => remove_folder_when_free(meeting_id, folder_path),
            }
            Ok(true)
        }
        Ok(false) => {
            log_warn!("Meeting not found or already deleted: {meeting_id}");
            Ok(false)
        }
        Err(e) => {
            log_error!("Error deleting meeting {meeting_id}: {e}");
            Err(format!("Failed to delete meeting: {e}"))
        }
    }
}

/// Remove a deleted meeting's folder once the folder lease is free (the cancelled pass
/// finishing its current stage). The row is already gone, so nothing is re-read. If the app
/// quits first, the folder stays behind with no row; the import flows treat such a folder
/// as importable, so nothing is lost.
fn remove_folder_when_free(meeting_id: &str, folder_path: Option<String>) {
    let meeting_id = meeting_id.to_string();
    tauri::async_runtime::spawn(async move {
        let _lease = folder_lease::acquire(&meeting_id, LeaseHolder::Delete).await;
        delete_recording_folder_best_effort(&meeting_id, folder_path.as_deref()).await;
    });
}

/// Best-effort removal of a meeting's on-disk recording folder after its DB rows
/// have been deleted.
///
/// Safety (specs/0073): the folder is removed only if it passes the ownership check
/// ([`is_meeting_folder`](crate::audio::meeting_folder::is_meeting_folder)): its
/// `metadata.json` names this meeting, or (a pre-0037 folder with no id) it lies inside
/// a recordings folder Nixon knows about, and it is never a root, `$HOME`, `/` or a volume
/// root. So a meeting left in an EARLIER recordings folder is still deleted, while we
/// never `remove_dir_all` an arbitrary stored path. A filesystem error (including the
/// folder already being gone) is logged and swallowed; it never fails the delete command.
async fn delete_recording_folder_best_effort(meeting_id: &str, folder_path: Option<&str>) {
    use crate::audio::meeting_folder::{remove_meeting_folder_in, RemoveOutcome};

    let folder_path = match folder_path {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => {
            log_info!("No recording folder to delete (meeting has no folder_path); skipping");
            return;
        }
    };
    let id = meeting_id.to_string();
    let target = folder_path.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let roots = crate::audio::recording_preferences::known_recording_roots();
        remove_meeting_folder_in(std::path::Path::new(&target), &id, &roots)
    })
    .await
    .unwrap_or_else(|e| RemoveOutcome::Failed(format!("delete task failed: {e}")));

    match outcome {
        RemoveOutcome::Removed => log_info!("Deleted recording folder: {folder_path}"),
        RemoveOutcome::AlreadyGone => {
            log_info!("Recording folder '{folder_path}' already gone; nothing to delete")
        }
        RemoveOutcome::NotOwned => log_warn!(
            "Recording folder '{folder_path}' is not this meeting's recording folder; \
             skipping file deletion for safety"
        ),
        RemoveOutcome::Failed(e) => log_warn!(
            "Failed to delete recording folder '{folder_path}' (DB delete already committed): {e}"
        ),
    }
}
