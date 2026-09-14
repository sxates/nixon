//! Startup reconciliation of stranded processing markers (spec 0051 WS2).
//!
//! `meetings.processing_mode = 'live'` is written by the mid-meeting Live/Deferred
//! override and cleared by the deferred backlog once it has finished processing the
//! meeting. A row still carrying `'live'` when the app starts therefore means one
//! thing: processing never completed. That state is otherwise **terminal** — the
//! backlog's `list_deferred_candidates` matches only `'defer'` or sparse-transcript
//! meetings, so a stranded meeting with a partial-but-not-sparse transcript can never
//! be picked up again, and the auto-summary gate keeps skipping it silently.
//!
//! Rewriting `'live'` → `'defer'` at startup returns those meetings to the one state
//! the system already knows how to recover from. This runs at STARTUP specifically:
//! during a session the same marker legitimately means "recording right now, live",
//! and the backlog has no recording-state dependency to distinguish them.
//!
//! **Except for one meeting: the crash-interrupted one** (spec 0051 final review,
//! Finding 3). "No recording can be in progress at startup" is true only of *running*
//! recordings — an INTERRUPTED one can be, and `ResumeRecordingPrompt` (specs/0037)
//! offers to continue it into the same `meetingId` and folder. Rewriting its marker
//! before the user answers that prompt would (a) silently downgrade a Live session to
//! record-only — `decide_session_mode` reads the marker at start and
//! `effective_live_stt` maps `Some("defer") => false` — and (b) let the backlog
//! retranscribe the meeting UNDERNEATH the resumed recording, since `'defer'` matches
//! `list_deferred_candidates` unconditionally and the backlog auto-drains 5s after
//! launch on AC. That is precisely the hazard the spec avoided by choosing a sweep over
//! an `OR processing_mode='live'` query arm. So the sweep skips any meeting whose
//! recording folder is still `status: "recording"` — the same discriminator
//! `api_list_interrupted_recordings` reads. Those meetings are reconciled on a later
//! launch, once the folder is finalized ("completed" on a clean stop, "discarded" if
//! the user declines the resume).

use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

use crate::audio::processing_mode::{MODE_DEFER, MODE_LIVE};
use crate::state::AppState;

/// Rewrite every `processing_mode='live'` row to `'defer'`, except the meeting ids in
/// `skip_meeting_ids` (crash-interrupted recordings awaiting the resume prompt — see
/// the module docs). Returns how many rows changed (0 on a healthy install).
pub async fn reconcile_stranded_live_markers(
    pool: &SqlitePool,
    skip_meeting_ids: &[String],
) -> Result<u64, sqlx::Error> {
    let mut sql = String::from("UPDATE meetings SET processing_mode = ? WHERE processing_mode = ?");
    if !skip_meeting_ids.is_empty() {
        sql.push_str(" AND id NOT IN (");
        sql.push_str(&vec!["?"; skip_meeting_ids.len()].join(","));
        sql.push(')');
    }
    let mut query = sqlx::query(&sql).bind(MODE_DEFER).bind(MODE_LIVE);
    for id in skip_meeting_ids {
        query = query.bind(id);
    }
    Ok(query.execute(pool).await?.rows_affected())
}

/// The meeting ids whose recording folder is still mid-recording (crash/force-quit),
/// i.e. exactly what the specs/0037 resume prompt may offer to continue. Filesystem
/// only — safe to call before the resume prompt has been answered.
fn interrupted_meeting_ids(recordings_root: &Path) -> Vec<String> {
    super::recording_recovery::scan_interrupted_recordings(recordings_root)
        .into_iter()
        .map(|r| r.meeting_id)
        .collect()
}

/// The startup sweep against a given recordings root: skip the crash-interrupted
/// meetings, reconcile the rest. Split out from `spawn_startup_reconciliation` so the
/// scan→skip wiring is testable without a real app-data recordings folder.
pub async fn reconcile_at_startup(
    pool: &SqlitePool,
    recordings_root: &Path,
) -> Result<u64, sqlx::Error> {
    let skip = interrupted_meeting_ids(recordings_root);
    if !skip.is_empty() {
        log::info!(
            "processing-mode reconciliation: leaving {} interrupted recording(s) alone \
             until the resume prompt is answered",
            skip.len()
        );
    }
    reconcile_stranded_live_markers(pool, &skip).await
}

/// Best-effort startup sweep. A failure — or the database simply not being
/// initialized yet (see below) — logs and leaves the rows alone; the next launch
/// retries. It can never block or crash startup.
///
/// `AppState` is unmanaged during the first-launch onboarding window (see
/// `database::setup::initialize_database_on_startup`: the first-launch branch only
/// emits `first-launch-detected` and defers `app.manage(AppState)` to a later,
/// frontend-triggered command), so this uses `try_state` — the same pattern as the
/// sibling background jobs spawned right alongside it in `lib.rs`
/// (`audio::retention::run_retention_sweep`, `calendar::google::sync::db_pool`) —
/// rather than `state()`, which would panic on that path.
///
/// Returns the `JoinHandle` so tests can await completion; `lib.rs` (like the
/// sibling spawners) ignores it — the sweep is fire-and-forget from the caller's
/// perspective. `JoinHandle` is not `#[must_use]`, so the ignored return at the
/// `lib.rs` call site is not a clippy warning.
pub fn spawn_startup_reconciliation<R: Runtime>(
    app: AppHandle<R>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let Some(pool) = app
            .try_state::<AppState>()
            .map(|s| s.db_manager.pool().clone())
        else {
            log::debug!(
                "processing-mode reconciliation skipped: database not initialized yet \
                 (normal on a fresh install before onboarding completes; nothing to \
                 reconcile yet since no meeting can exist)"
            );
            return;
        };
        // The interrupted-recording scan touches the filesystem; keep it off the async
        // worker (mirrors `api_list_interrupted_recordings`).
        let root: PathBuf = match tokio::task::spawn_blocking(
            super::recording_preferences::recordings_root,
        )
        .await
        {
            Ok(root) => root,
            Err(e) => {
                log::warn!("processing-mode reconciliation: recordings-folder lookup failed: {e}");
                return;
            }
        };
        match reconcile_at_startup(&pool, &root).await {
            Ok(0) => log::debug!("processing-mode reconciliation: nothing stranded"),
            Ok(n) => log::info!(
                "processing-mode reconciliation: returned {n} stranded meeting(s) to the deferred backlog"
            ),
            Err(e) => log::warn!("processing-mode reconciliation failed (will retry next launch): {e}"),
        }
    })
}
