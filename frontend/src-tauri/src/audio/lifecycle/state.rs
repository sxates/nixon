//! The persisted half of the audio lifecycle (specs/0072): reading a meeting's facts from
//! the database and its folder, and every write to `meetings.audio_state` /
//! `speakers_identified_at`. No app handle and no events here, so each transition is
//! testable against a migrated pool; `hooks.rs` wraps these for the app and emits.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::SqlitePool;

use super::policy::{AudioState, MeetingAudioFacts};
use super::MIN_TRANSCRIPT_SEGMENTS;
use crate::audio::constants::AUDIO_EXTENSIONS;

/// The backlog predicate (spec task 12), shared by `list_deferred_candidates` and the
/// lifecycle: explicitly deferred, or a sparse transcript with no completed summary that
/// hasn't been through the lifecycle yet. The `audio_state IS NULL` term lets a meeting
/// that was transcribed and turned out silent leave the backlog.
pub fn awaiting_transcription_sql(alias: &str) -> String {
    format!(
        "(COALESCE({a}.processing_mode, '') = 'defer' \
          OR ((SELECT COUNT(*) FROM transcripts t WHERE t.meeting_id = {a}.id) < {min} \
              AND NOT EXISTS (SELECT 1 FROM summary_processes sp \
                              WHERE sp.meeting_id = {a}.id AND sp.status = 'completed') \
              AND {a}.audio_state IS NULL))",
        a = alias,
        min = MIN_TRANSCRIPT_SEGMENTS
    )
}

/// The lifecycle's "not transcribed yet": the backlog predicate, plus a `'live'` marker (a
/// stop-time handoff that hasn't finished; the startup reconcile turns a stranded one into
/// `'defer'`). Kept out of the backlog query on purpose — see `processing_reconcile`.
fn lifecycle_awaiting_sql(alias: &str) -> String {
    format!(
        "({} OR COALESCE({alias}.processing_mode, '') = 'live')",
        awaiting_transcription_sql(alias)
    )
}

/// One meeting's lifecycle row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AudioRow {
    pub id: String,
    pub created_at: String,
    pub folder_path: Option<String>,
    pub audio_state: Option<String>,
    pub speakers_identified_at: Option<String>,
    pub processing_mode: Option<String>,
    pub awaiting: bool,
}

impl AudioRow {
    pub fn state(&self) -> AudioState {
        AudioState::from_db(self.audio_state.as_deref())
    }

    pub fn folder(&self) -> Option<PathBuf> {
        self.folder_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
    }
}

fn row_select(filter: &str) -> String {
    format!(
        "SELECT m.id, m.created_at, m.folder_path, m.audio_state, m.speakers_identified_at, \
                m.processing_mode, {} AS awaiting \
         FROM meetings m WHERE {filter}",
        lifecycle_awaiting_sql("m")
    )
}

pub async fn load_row(pool: &SqlitePool, meeting_id: &str) -> Result<Option<AudioRow>> {
    sqlx::query_as::<_, AudioRow>(&row_select("m.id = ?"))
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| anyhow!("Could not read the audio state of meeting {meeting_id}: {e}"))
}

/// Every meeting the sweep or the preview has to look at: it has a folder and its audio
/// hasn't been purged yet.
pub async fn list_rows(pool: &SqlitePool) -> Result<Vec<AudioRow>> {
    sqlx::query_as::<_, AudioRow>(&row_select(
        "m.folder_path IS NOT NULL AND (m.audio_state IS NULL OR m.audio_state <> 'purged') \
         ORDER BY m.created_at ASC",
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow!("Could not list meetings for the audio sweep: {e}"))
}

/// What a meeting folder holds, as far as the lifecycle cares.
#[derive(Debug, Clone, Default)]
pub struct FolderAudio {
    /// `metadata.json` says `status: "recording"` (live, or crash-interrupted).
    pub recording: bool,
    /// A mix (`audio.mp4`, an imported file, a resume segment's `audio_seg*.mp4`, …).
    pub mix: bool,
    pub wav_channels: bool,
    pub opus_channels: bool,
    /// The system channel speaker identification reads, in either format.
    pub system_channel: bool,
    /// Everything a Delete removes, with sizes.
    pub deletable: Vec<(PathBuf, u64)>,
}

impl FolderAudio {
    pub fn deletable_bytes(&self) -> u64 {
        self.deletable.iter().map(|(_, n)| n).sum()
    }
}

/// `mic` / `system` / `mic_segNN` / `system_segNN` — the per-channel files.
pub fn is_channel_stem(stem: &str) -> bool {
    ["mic", "system"].iter().any(|c| {
        stem == *c
            || stem
                .strip_prefix(c)
                .and_then(|rest| rest.strip_prefix("_seg"))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })
}

fn is_audio_extension(ext: &str) -> bool {
    ext == "opus" || AUDIO_EXTENSIONS.contains(&ext)
}

/// The folder's `metadata.json` says the recording is still running (or was interrupted).
/// A missing or unreadable file is a finalized folder (pre-0037 layouts have none).
pub fn folder_is_recording(folder: &Path) -> bool {
    std::fs::read_to_string(folder.join("metadata.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("status").and_then(|s| s.as_str()).map(str::to_owned))
        .is_some_and(|s| s == "recording")
}

/// The files a Delete removes: every top-level audio file (any extension in
/// `AUDIO_EXTENSIONS`, `.opus`, resume segments, the decoder's `.nixon_decode_*.wav`
/// temps, the compressor's `*.opus.tmp`), plus `.checkpoints/` — but the checkpoints only
/// when the folder is NOT `status: "recording"`, because an interrupted session's
/// checkpoints are its only audio. `metadata.json` / `transcripts.json` never match.
pub fn deletable_files(folder: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if is_audio_extension(&ext) || name.ends_with(".opus.tmp") {
            out.push((path, meta.len()));
        }
    }
    if !folder_is_recording(folder) {
        let checkpoints = folder.join(".checkpoints");
        if let Ok(entries) = std::fs::read_dir(&checkpoints) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        out.push((entry.path(), meta.len()));
                    }
                }
            }
        }
    }
    out
}

/// Scan a meeting folder. `None` when it isn't a directory (gone, or on a volume that isn't
/// mounted): the lifecycle then leaves the meeting alone.
pub fn scan_folder(folder: &Path) -> Option<FolderAudio> {
    if !folder.is_dir() {
        return None;
    }
    let mut audio = FolderAudio {
        recording: folder_is_recording(folder),
        deletable: deletable_files(folder),
        ..FolderAudio::default()
    };
    for (path, _) in &audio.deletable {
        if path.parent() != Some(folder) {
            continue; // checkpoints
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if stem.starts_with('.') || ext == "tmp" {
            continue; // decoder / compressor temps
        }
        if is_channel_stem(&stem) && (ext == "wav" || ext == "opus") {
            audio.wav_channels |= ext == "wav";
            audio.opus_channels |= ext == "opus";
            audio.system_channel |= stem == "system";
        } else {
            audio.mix = true;
        }
    }
    Some(audio)
}

/// Parse a `meetings.created_at` TEXT value tolerantly (moved from the 0029 sweep). sqlx
/// writes chrono values; imported/legacy rows may carry other shapes.
pub fn parse_created_at(raw: &str) -> Option<DateTime<Utc>> {
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

/// The policy's input for one meeting. `None` when `created_at` can't be parsed (the
/// meeting is then kept, as before 0072).
pub fn facts(row: &AudioRow, folder: &FolderAudio) -> Option<MeetingAudioFacts> {
    Some(MeetingAudioFacts {
        state: row.state(),
        created_at: parse_created_at(&row.created_at)?,
        folder_recording: folder.recording,
        awaiting_transcription: row.awaiting,
        has_wav_channels: folder.wav_channels,
    })
}

/// Whether speaker identification applies to a meeting (spec §Definitions 3): the setting
/// is on and the models are present (we never auto-download). The third condition, a
/// system channel, comes from the folder.
#[derive(Debug, Clone, Copy)]
pub struct DiarizationGate {
    pub enabled: bool,
    pub models_present: bool,
}

impl DiarizationGate {
    pub fn applies(self, has_system_channel: bool) -> bool {
        self.enabled && self.models_present && has_system_channel
    }
}

/// Write `processed` when the last of the spec's conditions 1–3 became true. Only a pending
/// or failed meeting moves. Returns the new state when it changed.
///
/// `transcribed` is set by callers that know a transcription pass just finished (the
/// backlog clearing `defer`, a retranscription): the sparse-transcript arm of the backlog
/// predicate then no longer holds a meeting back, which is how a silent meeting leaves the
/// backlog. An explicit `defer`/`live` marker still does.
pub async fn reevaluate(
    pool: &SqlitePool,
    meeting_id: &str,
    gate: DiarizationGate,
    transcribed: bool,
) -> Result<Option<AudioState>> {
    let Some(row) = load_row(pool, meeting_id).await? else {
        return Ok(None);
    };
    if !matches!(row.state(), AudioState::Pending | AudioState::Failed) {
        return Ok(None);
    }
    let Some(folder) = row.folder() else {
        return Ok(None); // notes-only: no audio, no lifecycle
    };
    if folder_is_recording(&folder) {
        return Ok(None); // condition 1
    }
    let marked = matches!(row.processing_mode.as_deref(), Some("defer") | Some("live"));
    if (transcribed && marked) || (!transcribed && row.awaiting) {
        return Ok(None); // condition 2
    }
    let has_system = scan_folder(&folder).is_some_and(|f| f.system_channel);
    if row.speakers_identified_at.is_none() && gate.applies(has_system) {
        return Ok(None); // condition 3
    }
    let changed = sqlx::query(
        "UPDATE meetings SET audio_state = 'processed' \
         WHERE id = ? AND (audio_state IS NULL OR audio_state = 'failed')",
    )
    .bind(meeting_id)
    .execute(pool)
    .await
    .map_err(|e| anyhow!("Could not mark meeting {meeting_id} processed: {e}"))?
    .rows_affected();
    Ok((changed > 0).then_some(AudioState::Processed))
}

/// Speaker identification succeeded: stamp it. The caller reevaluates next.
pub async fn mark_speakers_identified(pool: &SqlitePool, meeting_id: &str) -> Result<()> {
    sqlx::query("UPDATE meetings SET speakers_identified_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(meeting_id)
        .execute(pool)
        .await
        .map_err(|e| anyhow!("Could not record speaker identification for {meeting_id}: {e}"))?;
    Ok(())
}

/// Speaker identification errored. A pending meeting whose transcription is done becomes
/// `failed` (audio kept for a retry, then the grace rule). One still awaiting transcription
/// stays pending: the backlog runs speaker identification again after transcribing it.
pub async fn mark_failed_if_pending(pool: &SqlitePool, meeting_id: &str) -> Result<bool> {
    let sql = format!(
        "UPDATE meetings SET audio_state = 'failed' \
         WHERE id = ? AND audio_state IS NULL AND NOT {}",
        lifecycle_awaiting_sql("meetings")
    );
    let changed = sqlx::query(&sql)
        .bind(meeting_id)
        .execute(pool)
        .await
        .map_err(|e| anyhow!("Could not mark meeting {meeting_id} failed: {e}"))?
        .rows_affected();
    Ok(changed > 0)
}

/// The transcript rows were replaced: the old speaker labels no longer describe them.
pub async fn clear_speakers_identified(pool: &SqlitePool, meeting_id: &str) -> Result<()> {
    sqlx::query("UPDATE meetings SET speakers_identified_at = NULL WHERE id = ?")
        .bind(meeting_id)
        .execute(pool)
        .await
        .map_err(|e| anyhow!("Could not reset speaker identification for {meeting_id}: {e}"))?;
    Ok(())
}

/// A resumed recording adds a segment: the meeting is pending again. Returns true when the
/// state changed (it wasn't already pending).
pub async fn reset_for_resume(pool: &SqlitePool, meeting_id: &str) -> Result<bool> {
    let before = load_row(pool, meeting_id).await?.map(|r| r.state());
    sqlx::query(
        "UPDATE meetings SET audio_state = NULL, speakers_identified_at = NULL WHERE id = ?",
    )
    .bind(meeting_id)
    .execute(pool)
    .await
    .map_err(|e| anyhow!("Could not reset the audio state of {meeting_id}: {e}"))?;
    Ok(before.is_some_and(|s| s != AudioState::Pending))
}

pub async fn mark_purged(pool: &SqlitePool, meeting_id: &str) -> Result<()> {
    sqlx::query("UPDATE meetings SET audio_state = 'purged' WHERE id = ?")
        .bind(meeting_id)
        .execute(pool)
        .await
        .map_err(|e| anyhow!("Could not mark meeting {meeting_id} purged: {e}"))?;
    Ok(())
}

/// Pending meetings a quit interrupted mid-processing: finalized, transcribed, not purged.
/// The startup reconcile finishes each (spec §"Startup reconcile").
pub async fn list_unfinished(pool: &SqlitePool) -> Result<Vec<String>> {
    let rows = list_rows(pool).await?;
    Ok(rows
        .into_iter()
        .filter(|r| r.state() == AudioState::Pending && !r.awaiting)
        .filter(|r| {
            r.folder()
                .is_some_and(|f| f.is_dir() && !folder_is_recording(&f))
        })
        .map(|r| r.id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_stems_are_recognised() {
        for stem in ["mic", "system", "mic_seg00", "system_seg12"] {
            assert!(is_channel_stem(stem), "{stem}");
        }
        for stem in ["audio", "micro", "system_seg", "mic_segx", "audio_seg00"] {
            assert!(!is_channel_stem(stem), "{stem}");
        }
    }

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
            let parsed = parse_created_at(raw).unwrap_or_else(|| panic!("{raw:?}"));
            assert_eq!(parsed.format("%Y-%m-%d").to_string(), "2026-07-01");
        }
        assert!(parse_created_at("not a date").is_none());
    }

    #[test]
    fn a_delete_takes_audio_only_and_checkpoints_only_from_a_finalized_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["audio.mp4", "mic.opus", "system_seg00.wav", ".mic.opus.tmp"] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        for name in ["metadata.json", "transcripts.json"] {
            std::fs::write(root.join(name), b"{}").unwrap();
        }
        std::fs::create_dir(root.join(".checkpoints")).unwrap();
        std::fs::write(root.join(".checkpoints/audio_chunk_000.mp4"), b"x").unwrap();
        std::fs::write(root.join("metadata.json"), br#"{"status":"recording"}"#).unwrap();
        let names = |files: Vec<(PathBuf, u64)>| {
            let mut n: Vec<String> = files
                .iter()
                .map(|(p, _)| p.strip_prefix(root).unwrap().to_string_lossy().to_string())
                .collect();
            n.sort();
            n
        };
        assert_eq!(
            names(deletable_files(root)),
            [".mic.opus.tmp", "audio.mp4", "mic.opus", "system_seg00.wav"]
        );
        std::fs::write(root.join("metadata.json"), br#"{"status":"completed"}"#).unwrap();
        assert!(names(deletable_files(root)).contains(&".checkpoints/audio_chunk_000.mp4".into()));

        let scan = scan_folder(root).unwrap();
        assert!(scan.mix && scan.opus_channels && scan.wav_channels && !scan.system_channel);
    }

    #[test]
    fn the_shared_predicate_uses_the_segment_threshold() {
        assert!(awaiting_transcription_sql("m").contains(&format!("< {MIN_TRANSCRIPT_SEGMENTS}")));
    }
}
