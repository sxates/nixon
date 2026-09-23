//! The lifecycle executor (specs/0072 task 8): apply [`disposition`] to meetings — delete
//! or compress — under the folder lease (specs/0073), and the dry-run preview that shares
//! every decision with it.
//!
//! Lease rules (0073 W4): the sweep is a background job, so it only ever `try_acquire`s
//! (`Retention` to delete, `Compression` to compress). A busy meeting is skipped and
//! counted, and the next tick picks it up. After acquiring, the row and the folder are
//! **read again** and the decision is remade: the folder may have moved, and a resumed
//! recording may have made the meeting pending again.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use super::compress::{compress_channels, CompressOutcome};
use super::policy::{disposition, AudioRetention, AudioState, Disposition, KeepReason};
use super::state::{self, scan_folder, AudioRow, FolderAudio};
use crate::audio::folder_lease::{current_holder, try_acquire, LeaseHolder};

/// What one sweep may do besides deleting.
#[derive(Debug, Clone, Default)]
pub struct SweepOptions {
    /// Compress kept WAV channels (off until every channel reader understands `.opus`).
    pub compress: bool,
    /// At most this many meetings compressed per sweep (backfill is one per tick, Q4).
    pub compress_limit: usize,
    /// A recording is in progress: don't compress (deletes still run, they're cheap).
    pub recording_active: bool,
    pub ffmpeg: Option<PathBuf>,
}

/// What a purge removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PurgeStats {
    pub files_removed: usize,
    pub bytes_freed: u64,
}

/// The result of applying the policy to one meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Nothing to do (kept, no folder, already purged, compression not allowed now).
    Nothing,
    /// Another job holds the folder. `had_audio` = a Delete that would have freed space.
    Busy {
        delete: bool,
        had_audio: bool,
    },
    Purged(PurgeStats),
    Compressed(CompressOutcome),
    /// The compressor failed; the WAVs are untouched.
    CompressFailed(String),
}

/// `api_apply_audio_retention_now`'s answer (camelCase for the UI).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionReport {
    pub meetings_purged: usize,
    pub bytes_freed: u64,
    pub skipped_busy: usize,
    #[serde(skip)]
    pub meetings_compressed: usize,
    /// Meetings whose `audio_state` this sweep wrote (for the state-changed event).
    #[serde(skip)]
    pub purged_ids: Vec<String>,
}

/// `api_preview_audio_retention`'s answer: what applying `policy` would delete now.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPreview {
    /// Meetings whose audio would be deleted.
    pub meetings: usize,
    pub bytes: u64,
    /// Meetings the policy would delete once processed, kept because processing hasn't
    /// finished (pending, awaiting transcription, or still recording).
    pub kept_pending: usize,
    /// Meetings the policy would delete, kept because speaker identification failed.
    pub kept_failed: usize,
    /// Of `meetings`, how many another job is working on right now (deleted shortly after).
    pub busy: usize,
}

fn decide(
    row: &AudioRow,
    folder: &FolderAudio,
    policy: AudioRetention,
    now: DateTime<Utc>,
) -> Option<Disposition> {
    let Some(facts) = state::facts(row, folder) else {
        log::warn!(
            "Audio sweep: could not parse created_at {:?} for meeting {}; keeping its audio",
            row.created_at,
            row.id
        );
        return None;
    };
    Some(disposition(policy, &facts, now))
}

/// Load the row and scan its folder: the unit every decision is made on.
async fn observe(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<Option<(AudioRow, PathBuf, FolderAudio)>> {
    let Some(row) = state::load_row(pool, meeting_id).await? else {
        return Ok(None);
    };
    let Some(folder) = row.folder() else {
        return Ok(None);
    };
    let Some(audio) = scan_folder(&folder) else {
        return Ok(None); // gone, or its volume isn't mounted: leave it alone
    };
    Ok(Some((row, folder, audio)))
}

/// Delete exactly the files `state::deletable_files` names (`.checkpoints/` only when the
/// folder isn't `status: "recording"`), then remove an emptied `.checkpoints/`. Best-effort
/// per file.
pub fn purge_media_files(folder: &Path) -> PurgeStats {
    let mut stats = PurgeStats::default();
    for (path, size) in state::deletable_files(folder) {
        match std::fs::remove_file(&path) {
            Ok(()) => {
                stats.files_removed += 1;
                stats.bytes_freed += size;
            }
            Err(e) => log::warn!("Audio sweep could not remove {}: {e}", path.display()),
        }
    }
    let _ = std::fs::remove_dir(folder.join(".checkpoints")); // only succeeds when empty
    stats
}

/// Apply `policy` to one meeting. Shared by the sweep, "apply now" and the
/// post-processing hook, so they can't disagree.
pub async fn apply_one(
    pool: &SqlitePool,
    meeting_id: &str,
    policy: AudioRetention,
    now: DateTime<Utc>,
    opts: &SweepOptions,
) -> Result<ApplyOutcome> {
    let Some((row, _, audio)) = observe(pool, meeting_id).await? else {
        return Ok(ApplyOutcome::Nothing);
    };
    match decide(&row, &audio, policy, now) {
        Some(Disposition::Delete) => delete_leased(pool, meeting_id, policy, now, &audio).await,
        Some(Disposition::Compress) if compress_allowed(opts) => {
            compress_leased(pool, meeting_id, policy, now, opts).await
        }
        _ => Ok(ApplyOutcome::Nothing),
    }
}

fn compress_allowed(opts: &SweepOptions) -> bool {
    opts.compress && !opts.recording_active && opts.ffmpeg.is_some()
}

async fn delete_leased(
    pool: &SqlitePool,
    meeting_id: &str,
    policy: AudioRetention,
    now: DateTime<Utc>,
    seen: &FolderAudio,
) -> Result<ApplyOutcome> {
    let Some(_lease) = try_acquire(meeting_id, LeaseHolder::Retention) else {
        log::debug!(
            "Audio sweep: meeting {meeting_id} is busy ({:?}); retrying later",
            current_holder(meeting_id)
        );
        return Ok(ApplyOutcome::Busy {
            delete: true,
            had_audio: !seen.deletable.is_empty(),
        });
    };
    // Re-read under the lease: the folder may have moved, the meeting may be pending again.
    let Some((row, folder, audio)) = observe(pool, meeting_id).await? else {
        return Ok(ApplyOutcome::Nothing);
    };
    if decide(&row, &audio, policy, now) != Some(Disposition::Delete) {
        return Ok(ApplyOutcome::Nothing);
    }
    let (stats, left) = tokio::task::spawn_blocking(move || {
        let stats = purge_media_files(&folder);
        (stats, state::deletable_files(&folder).len())
    })
    .await?;
    if left == 0 {
        state::mark_purged(pool, meeting_id).await?;
    } // else a file resisted deletion: stay as we are, the next tick retries

    log::info!(
        "Audio sweep: deleted {} file(s), {:.1} MB, for meeting {meeting_id}",
        stats.files_removed,
        stats.bytes_freed as f64 / 1_048_576.0
    );
    Ok(ApplyOutcome::Purged(stats))
}

async fn compress_leased(
    pool: &SqlitePool,
    meeting_id: &str,
    policy: AudioRetention,
    now: DateTime<Utc>,
    opts: &SweepOptions,
) -> Result<ApplyOutcome> {
    let Some(ffmpeg) = opts.ffmpeg.clone() else {
        return Ok(ApplyOutcome::Nothing);
    };
    let Some(_lease) = try_acquire(meeting_id, LeaseHolder::Compression) else {
        return Ok(ApplyOutcome::Busy {
            delete: false,
            had_audio: false,
        });
    };
    let Some((row, folder, audio)) = observe(pool, meeting_id).await? else {
        return Ok(ApplyOutcome::Nothing);
    };
    if decide(&row, &audio, policy, now) != Some(Disposition::Compress) {
        return Ok(ApplyOutcome::Nothing);
    }
    let shown = folder.display().to_string();
    match tokio::task::spawn_blocking(move || compress_channels(&ffmpeg, &folder)).await? {
        Ok(outcome) => {
            log::info!(
                "Audio sweep: compressed {} channel file(s) for meeting {meeting_id}, {:.1} → {:.1} MB",
                outcome.files,
                outcome.bytes_before as f64 / 1_048_576.0,
                outcome.bytes_after as f64 / 1_048_576.0
            );
            Ok(ApplyOutcome::Compressed(outcome))
        }
        Err(e) => {
            log::warn!("Audio sweep: kept the WAVs in {shown}: {e:#}");
            Ok(ApplyOutcome::CompressFailed(format!("{e:#}")))
        }
    }
}

/// One full sweep with `policy` (spec §"Sweep executor"). No early return for "keep
/// forever": compression is part of the sweep.
pub async fn run_sweep(
    pool: &SqlitePool,
    policy: AudioRetention,
    now: DateTime<Utc>,
    opts: &SweepOptions,
) -> Result<RetentionReport> {
    let mut report = RetentionReport::default();
    for row in state::list_rows(pool).await? {
        let opts_now = SweepOptions {
            compress: opts.compress && report.meetings_compressed < opts.compress_limit,
            ..opts.clone()
        };
        match apply_one(pool, &row.id, policy, now, &opts_now).await {
            Ok(ApplyOutcome::Purged(stats)) => {
                if stats.files_removed > 0 {
                    report.meetings_purged += 1;
                    report.bytes_freed += stats.bytes_freed;
                }
                report.purged_ids.push(row.id);
            }
            Ok(ApplyOutcome::Busy {
                delete: true,
                had_audio: true,
            }) => report.skipped_busy += 1,
            Ok(ApplyOutcome::Compressed(o)) if o.files > 0 => report.meetings_compressed += 1,
            Ok(_) => {}
            Err(e) => log::warn!("Audio sweep: meeting {}: {e:#}", row.id),
        }
    }
    log::info!(
        "Audio sweep ({policy:?}): deleted audio from {} meeting(s), freed {:.1} MB; \
         compressed {}; {} busy",
        report.meetings_purged,
        report.bytes_freed as f64 / 1_048_576.0,
        report.meetings_compressed,
        report.skipped_busy
    );
    Ok(report)
}

/// The dry run: the same [`disposition`] over every meeting with `policy` as the
/// candidate. Deletes nothing and takes no lease.
pub async fn preview(
    pool: &SqlitePool,
    policy: AudioRetention,
    now: DateTime<Utc>,
) -> Result<RetentionPreview> {
    let mut out = RetentionPreview::default();
    for row in state::list_rows(pool).await? {
        let Some(folder) = row.folder() else { continue };
        let Some(audio) = scan_folder(&folder) else {
            continue;
        };
        if audio.deletable.is_empty() {
            continue;
        }
        let Some(facts) = state::facts(&row, &audio) else {
            continue;
        };
        match disposition(policy, &facts, now) {
            Disposition::Delete => {
                out.meetings += 1;
                out.bytes += audio.deletable_bytes();
                if current_holder(&row.id).is_some() {
                    out.busy += 1;
                }
            }
            Disposition::Keep(reason) => {
                // Would the policy delete it once processing finishes?
                let mut processed = facts.clone();
                processed.state = AudioState::Processed;
                processed.awaiting_transcription = false;
                processed.folder_recording = false;
                if disposition(policy, &processed, now) != Disposition::Delete {
                    continue;
                }
                match reason {
                    KeepReason::Failed => out.kept_failed += 1,
                    KeepReason::Processing
                    | KeepReason::AwaitingProcessing
                    | KeepReason::Recording => out.kept_pending += 1,
                    KeepReason::Policy | KeepReason::AlreadyPurged => {}
                }
            }
            Disposition::Compress => {}
        }
    }
    Ok(out)
}
