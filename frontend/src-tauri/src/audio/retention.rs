//! Audio retention sweep (specs/0029 WS7.1).
//!
//! When `RecordingPreferences::retention_days` is set, a background task
//! periodically deletes the **media files** of meetings older than the
//! retention window. Scope is deliberately narrow:
//!
//! - Only files with an audio extension (`AUDIO_EXTENSIONS`, e.g. `audio.mp4`,
//!   `mic.wav`, `system.wav`) directly inside the meeting's `folder_path` are
//!   removed. `metadata.json`, `transcripts.json`, subdirectories, the DB row,
//!   transcripts, notes, and summaries are never touched.
//! - Meetings whose audio is their *only* record — no/sparse transcript rows,
//!   e.g. record-only meetings awaiting deferred transcription (WS7.2) — are
//!   **exempt**. This exemption is mandatory, not an optimization.
//!
//! The decision core (`decide_sweep`) is pure and unit-tested; the executor
//! (`purge_media_files`) is a plain fs function tested against a tempdir.

use super::constants::AUDIO_EXTENSIONS;
use crate::audio::recording_preferences::load_recording_preferences;
use crate::state::AppState;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use log::{debug, info, warn};
use std::path::Path;
use tauri::{AppHandle, Manager, Runtime};

/// A meeting is considered "not really transcribed" below this many transcript
/// segments and is exempt from the sweep: its audio may be the only record of
/// the meeting (record-only mode / deferred transcription, specs/0029 WS7.2).
pub const MIN_TRANSCRIPT_SEGMENTS: i64 = 3;

/// First sweep shortly after startup so short-lived sessions still enforce.
const STARTUP_DELAY: std::time::Duration = std::time::Duration::from_secs(2 * 60);

/// Steady-state sweep cadence.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// One meeting as seen by the sweep decision.
#[derive(Debug, Clone)]
pub struct SweepCandidate {
    pub meeting_id: String,
    pub created_at: DateTime<Utc>,
    pub transcript_count: i64,
    /// `meetings.processing_mode` override. `Some("defer")` means the meeting
    /// still awaits its full deferred processing pass (low-power-mode spec §5)
    /// regardless of how many transcript rows it has accumulated so far — it
    /// must never be swept.
    pub processing_mode: Option<String>,
}

/// What the sweep should do with one meeting's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepDecision {
    /// Older than the retention window and transcribed — delete its media files.
    Purge,
    /// Retention is off (`retention_days = None`) — keep everything forever.
    KeepRetentionOff,
    /// Within the retention window — keep.
    KeepRecent,
    /// Old enough to purge, but it has no/sparse transcript rows: its audio may
    /// be awaiting deferred transcription (WS7.2), so it is exempt.
    ExemptUntranscribed,
}

/// Pure decision core: should this meeting's media be purged as of `now`?
///
/// Boundary semantics: a meeting is purged only when **strictly older** than
/// `retention_days` — a meeting exactly `retention_days` old is kept.
pub fn decide_sweep(
    now: DateTime<Utc>,
    retention_days: Option<u32>,
    candidate: &SweepCandidate,
) -> SweepDecision {
    let Some(days) = retention_days else {
        return SweepDecision::KeepRetentionOff;
    };
    let cutoff = now - Duration::days(i64::from(days));
    if candidate.created_at >= cutoff {
        return SweepDecision::KeepRecent;
    }
    if candidate.transcript_count < MIN_TRANSCRIPT_SEGMENTS {
        return SweepDecision::ExemptUntranscribed;
    }
    if candidate.processing_mode.as_deref() == Some(crate::audio::processing_mode::MODE_DEFER) {
        return SweepDecision::ExemptUntranscribed;
    }
    SweepDecision::Purge
}

/// What a purge actually removed.
#[derive(Debug, Clone, Copy, Default)]
pub struct PurgeStats {
    pub files_removed: usize,
    pub bytes_freed: u64,
}

/// True when `path` is a media file the sweep may delete (by extension).
fn is_media_file(path: &Path) -> bool {
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_lowercase();
            AUDIO_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

/// Delete only the media files directly inside `folder`.
///
/// Never touches `metadata.json` (or any non-audio file), never recurses into
/// subdirectories, never removes the folder itself. Best-effort per file: one
/// undeletable file doesn't abort the rest.
pub fn purge_media_files(folder: &Path) -> Result<PurgeStats> {
    let mut stats = PurgeStats::default();
    let entries = std::fs::read_dir(folder).map_err(|e| {
        anyhow!(
            "Could not read recording folder {} (was it moved or deleted?): {}",
            folder.display(),
            e
        )
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_media_file(&path) {
            continue;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                stats.files_removed += 1;
                stats.bytes_freed += size;
                debug!("Retention sweep removed {}", path.display());
            }
            Err(e) => warn!("Retention sweep could not remove {}: {}", path.display(), e),
        }
    }
    Ok(stats)
}

/// True when the folder contains at least one media file (non-recursive).
fn folder_has_media(folder: &Path) -> bool {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().is_file() && is_media_file(&e.path()))
        })
        .unwrap_or(false)
}

/// Parse a `meetings.created_at` TEXT value tolerantly. sqlx writes chrono
/// `DateTime<Utc>` values; imported/legacy rows may carry other shapes, so try
/// RFC 3339, then offset-suffixed, then naive-UTC formats.
fn parse_created_at(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%d %H:%M:%S%.f %:z", "%Y-%m-%d %H:%M:%S%.f %z"] {
        if let Ok(dt) = DateTime::parse_from_str(raw, fmt) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(DateTime::from_naive_utc_and_offset(naive, Utc));
        }
    }
    None
}

/// One full sweep: read preferences, list meetings with a recording folder,
/// decide per meeting, purge, and log a summary line.
pub async fn run_retention_sweep<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    let prefs = load_recording_preferences(app).await?;
    let Some(days) = prefs.retention_days else {
        debug!("Retention sweep skipped: retention is off (keep audio forever)");
        return Ok(());
    };

    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| anyhow!("Retention sweep skipped: database is not initialized yet"))?;
    let pool = state.db_manager.pool();

    // Read-only query, deliberately local to this module (specs/0029 WS7.1):
    // every meeting that has a recording folder, with its transcript-row count
    // so the untranscribed exemption can be decided without a second query.
    let rows: Vec<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT m.id, m.created_at, m.folder_path, \
                (SELECT COUNT(*) FROM transcripts t WHERE t.meeting_id = m.id), \
                m.processing_mode \
         FROM meetings m \
         WHERE m.folder_path IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow!("Retention sweep could not list meetings: {}", e))?;

    let now = Utc::now();
    let mut totals = PurgeStats::default();
    let mut meetings_purged = 0usize;
    let mut meetings_exempt = 0usize;
    let mut meetings_kept = 0usize;

    for (meeting_id, created_at_raw, folder_path, transcript_count, processing_mode) in rows {
        let Some(created_at) = parse_created_at(&created_at_raw) else {
            warn!(
                "Retention sweep: could not parse created_at {:?} for meeting {}; keeping its audio",
                created_at_raw, meeting_id
            );
            meetings_kept += 1;
            continue;
        };

        let candidate = SweepCandidate {
            meeting_id,
            created_at,
            transcript_count,
            processing_mode,
        };

        match decide_sweep(now, Some(days), &candidate) {
            SweepDecision::Purge => {
                let folder = Path::new(&folder_path);
                if !folder.is_dir() {
                    // Folder already gone (moved, external cleanup) — nothing to do.
                    continue;
                }
                match purge_media_files(folder) {
                    Ok(stats) => {
                        if stats.files_removed > 0 {
                            meetings_purged += 1;
                            totals.files_removed += stats.files_removed;
                            totals.bytes_freed += stats.bytes_freed;
                        }
                    }
                    Err(e) => warn!(
                        "Retention sweep: could not purge media for meeting {}: {:#}",
                        candidate.meeting_id, e
                    ),
                }
            }
            SweepDecision::ExemptUntranscribed => {
                // Only count meetings that actually still hold audio — that's
                // the interesting "awaiting deferred transcription" set.
                if folder_has_media(Path::new(&folder_path)) {
                    meetings_exempt += 1;
                }
            }
            SweepDecision::KeepRecent | SweepDecision::KeepRetentionOff => {
                meetings_kept += 1;
            }
        }
    }

    info!(
        "Retention sweep ({} days): removed {} media file(s), freed {:.1} MB across {} meeting(s); {} meeting(s) exempt (no/sparse transcript — may await transcription); {} within retention",
        days,
        totals.files_removed,
        totals.bytes_freed as f64 / (1024.0 * 1024.0),
        meetings_purged,
        meetings_exempt,
        meetings_kept
    );
    Ok(())
}

/// Spawn the background retention sweeper. Call from `lib.rs::run().setup`
/// **after** database init (same pattern as `zoom::spawn_zoom_monitor`). Runs
/// once shortly after startup, then every 24 h; preferences are re-read on
/// every pass so changing the setting takes effect without a relaunch.
pub fn spawn_retention_sweeper<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        info!(
            "Retention sweeper started (first run in {:?}, then every {:?})",
            STARTUP_DELAY, SWEEP_INTERVAL
        );
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            if let Err(e) = run_retention_sweep(&app).await {
                warn!("Retention sweep failed: {:#}", e);
            }
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    });
}

/// Does this meeting still have an audio recording on disk? Lets the UI render
/// re-transcribe/diarize affordances honestly ("audio removed by retention
/// policy") instead of failing with a raw error (specs/0029 WS7.1).
#[tauri::command]
pub async fn api_meeting_audio_available<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<bool, String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "Database is not initialized yet".to_string())?;

    let folder_path: Option<Option<String>> =
        sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
            .bind(&meeting_id)
            .fetch_optional(state.db_manager.pool())
            .await
            .map_err(|e| format!("Could not look up meeting {}: {}", meeting_id, e))?;

    let Some(Some(folder_path)) = folder_path else {
        // Unknown meeting or no recording folder — no audio either way.
        return Ok(false);
    };

    Ok(folder_has_media(Path::new(&folder_path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn candidate(created_at: DateTime<Utc>, transcript_count: i64) -> SweepCandidate {
        SweepCandidate {
            meeting_id: "meeting-test".to_string(),
            created_at,
            transcript_count,
            processing_mode: None,
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap()
    }

    // --- decide_sweep: the pure decision core -----------------------------

    #[test]
    fn retention_off_keeps_everything() {
        let ancient = now() - Duration::days(10_000);
        assert_eq!(
            decide_sweep(now(), None, &candidate(ancient, 500)),
            SweepDecision::KeepRetentionOff
        );
    }

    #[test]
    fn recent_meeting_is_kept() {
        let recent = now() - Duration::days(3);
        assert_eq!(
            decide_sweep(now(), Some(7), &candidate(recent, 100)),
            SweepDecision::KeepRecent
        );
    }

    #[test]
    fn boundary_exactly_n_days_old_is_kept() {
        // Strictly-older-than semantics: exactly 7 days old is NOT purged.
        let exactly = now() - Duration::days(7);
        assert_eq!(
            decide_sweep(now(), Some(7), &candidate(exactly, 100)),
            SweepDecision::KeepRecent
        );
    }

    #[test]
    fn just_past_the_boundary_is_purged() {
        let just_over = now() - Duration::days(7) - Duration::seconds(1);
        assert_eq!(
            decide_sweep(now(), Some(7), &candidate(just_over, 100)),
            SweepDecision::Purge
        );
    }

    #[test]
    fn old_untranscribed_meeting_is_exempt() {
        // Mandatory exemption (specs/0029 WS7.1 x WS7.2): audio that is the only
        // record of the meeting must never be swept.
        let old = now() - Duration::days(30);
        assert_eq!(
            decide_sweep(now(), Some(7), &candidate(old, 0)),
            SweepDecision::ExemptUntranscribed
        );
    }

    #[test]
    fn old_sparsely_transcribed_meeting_is_exempt() {
        let old = now() - Duration::days(30);
        assert_eq!(
            decide_sweep(now(), Some(7), &candidate(old, MIN_TRANSCRIPT_SEGMENTS - 1)),
            SweepDecision::ExemptUntranscribed
        );
    }

    #[test]
    fn old_transcribed_meeting_at_min_segments_is_purged() {
        let old = now() - Duration::days(30);
        assert_eq!(
            decide_sweep(now(), Some(7), &candidate(old, MIN_TRANSCRIPT_SEGMENTS)),
            SweepDecision::Purge
        );
    }

    #[test]
    fn deferred_meeting_is_exempt_even_with_many_transcripts() {
        // Live→Defer mid-meeting leaves >MIN_TRANSCRIPT_SEGMENTS rows but the
        // meeting still awaits its full deferred pass — must never be swept.
        let candidate = SweepCandidate {
            meeting_id: "m1".into(),
            created_at: Utc::now() - Duration::days(100),
            transcript_count: 50,
            processing_mode: Some("defer".into()),
        };
        assert_eq!(
            decide_sweep(Utc::now(), Some(30), &candidate),
            SweepDecision::ExemptUntranscribed
        );
    }

    // --- purge_media_files: media-only deletion on a real tempdir ---------

    #[test]
    fn purge_removes_only_media_and_keeps_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();

        let media = ["audio.mp4", "mic.wav", "system.wav", "audio.m4a"];
        for name in media {
            std::fs::write(root.join(name), b"pcm-bytes").unwrap();
        }
        std::fs::write(root.join("metadata.json"), b"{\"keep\":true}").unwrap();
        std::fs::write(root.join("transcripts.json"), b"[]").unwrap();
        // A subdirectory containing audio must not be touched (non-recursive).
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/inner.wav"), b"pcm-bytes").unwrap();

        let stats = purge_media_files(root).expect("purge succeeds");

        assert_eq!(stats.files_removed, media.len());
        assert_eq!(stats.bytes_freed, (b"pcm-bytes".len() * media.len()) as u64);
        for name in media {
            assert!(!root.join(name).exists(), "{name} should be deleted");
        }
        assert!(
            root.join("metadata.json").exists(),
            "metadata.json must survive"
        );
        assert!(
            root.join("transcripts.json").exists(),
            "transcripts.json must survive"
        );
        assert!(
            root.join("nested/inner.wav").exists(),
            "subdirs must survive"
        );
        assert!(root.exists(), "the folder itself must survive");
    }

    #[test]
    fn purge_on_empty_folder_is_a_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stats = purge_media_files(dir.path()).expect("purge succeeds");
        assert_eq!(stats.files_removed, 0);
        assert_eq!(stats.bytes_freed, 0);
    }

    #[test]
    fn purge_missing_folder_errors_actionably() {
        let err = purge_media_files(Path::new("/definitely/not/a/real/folder"))
            .expect_err("missing folder should error");
        assert!(err.to_string().contains("recording folder"));
    }

    #[test]
    fn folder_has_media_reflects_purge() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(root.join("metadata.json"), b"{}").unwrap();
        assert!(!folder_has_media(root));
        std::fs::write(root.join("system.wav"), b"pcm").unwrap();
        assert!(folder_has_media(root));
        purge_media_files(root).unwrap();
        assert!(!folder_has_media(root));
    }

    // --- parse_created_at: tolerant timestamp parsing ----------------------

    #[test]
    fn parses_common_created_at_shapes() {
        for raw in [
            "2026-07-01T12:00:00Z",
            "2026-07-01T12:00:00.123+00:00",
            "2026-07-01 12:00:00.123 +00:00",
            "2026-07-01 12:00:00 +0000",
            "2026-07-01 12:00:00",
            "2026-07-01T12:00:00.123",
        ] {
            let parsed = parse_created_at(raw);
            assert!(parsed.is_some(), "should parse {raw:?}");
            assert_eq!(
                parsed.unwrap().date_naive(),
                Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0)
                    .unwrap()
                    .date_naive()
            );
        }
        assert!(parse_created_at("not a date").is_none());
    }
}
