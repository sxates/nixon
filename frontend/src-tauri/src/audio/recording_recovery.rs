//! specs/0037 — crash/quit recovery scan.
//!
//! A recording writes `metadata.json` with `status: "recording"` at start and only
//! flips it to `"completed"` on a clean stop (`RecordingSaver`). If the app crashes
//! or is force-quit mid-recording, the folder is left `"recording"` with its captured
//! `.checkpoints/` audio intact. This module scans for exactly that state so the app
//! can offer to **resume** the interrupted recording on relaunch (always prompt —
//! owner decision 2026-07-05). Since specs/0037 the folder's `metadata.json` carries
//! the DB `meeting_id`, so a match can reattach to the original meeting.

use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{info, warn};

/// Minimal, forward-compatible view of a folder's `metadata.json` — deliberately
/// decoupled from the full `MeetingMetadata` so this scan doesn't churn when that
/// struct grows (extra fields are ignored).
#[derive(Debug, Deserialize)]
struct RecoveryMeta {
    #[serde(default)]
    meeting_id: Option<String>,
    #[serde(default)]
    meeting_name: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    segments: Vec<serde_json::Value>,
}

/// An interrupted recording eligible to resume, surfaced to the relaunch prompt.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InterruptedRecording {
    pub meeting_id: String,
    pub folder_path: String,
    pub meeting_name: Option<String>,
    pub started_at: String,
    pub segment_count: u32,
}

/// Scan `root` for recordings that never finalized (metadata `status == "recording"`)
/// and still hold captured audio (a surviving `.checkpoints/` dir) — the crash/quit
/// signal. Requires a `meeting_id` in metadata (written at start since specs/0037) so
/// the resume can reattach; pre-0037 folders without one are skipped. Pure filesystem;
/// the command layer additionally filters by DB existence. Newest first.
pub fn scan_interrupted_recordings(root: &Path) -> Vec<InterruptedRecording> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) => {
            // A missing root just means "no recordings yet" — not an error.
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "recovery: could not read recordings root {}: {}",
                    root.display(),
                    e
                );
            }
            return out;
        }
    };

    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        let Ok(bytes) = std::fs::read(folder.join("metadata.json")) else {
            continue; // no metadata → not a recording folder we own
        };
        let meta: RecoveryMeta = match serde_json::from_slice(&bytes) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "recovery: unreadable metadata.json in {}: {}",
                    folder.display(),
                    e
                );
                continue;
            }
        };

        if meta.status.as_deref() != Some("recording") {
            continue; // cleanly finished, discarded, or errored — not interrupted
        }
        let Some(meeting_id) = meta.meeting_id.filter(|s| !s.is_empty()) else {
            continue; // pre-0037 folder without a DB link — cannot reattach
        };
        if !folder.join(".checkpoints").is_dir() {
            continue; // no captured audio to resume — don't offer an empty shell
        }

        out.push(InterruptedRecording {
            meeting_id,
            folder_path: folder.to_string_lossy().into_owned(),
            meeting_name: meta.meeting_name,
            started_at: meta.created_at.unwrap_or_default(),
            segment_count: meta.segments.len() as u32,
        });
    }

    // Newest first (RFC3339 `created_at` sorts lexically).
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out
}

/// [`scan_interrupted_recordings`] over every root in `roots` (specs/0073: an interrupted
/// recording in an earlier recordings folder is still offered). Newest first; a folder
/// reachable through two roots is reported once.
pub fn scan_interrupted_in_roots(roots: &[std::path::PathBuf]) -> Vec<InterruptedRecording> {
    let mut out: Vec<InterruptedRecording> = Vec::new();
    for root in roots {
        for found in scan_interrupted_recordings(root) {
            if !out.iter().any(|o| o.folder_path == found.folder_path) {
                out.push(found);
            }
        }
    }
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out
}

/// Mark an interrupted recording as no longer offerable WITHOUT deleting its captured
/// audio/transcripts — "finalize-and-keep": nothing the user recorded is lost, the
/// partial simply stays on disk (importable later) and stops appearing in the relaunch
/// prompt. Flips `metadata.json` `status` to `"discarded"`. Returns `Ok(false)` when no
/// folder matches `meeting_id`.
pub fn discard_interrupted_recording(root: &Path, meeting_id: &str) -> std::io::Result<bool> {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };

    for entry in entries.flatten() {
        let folder = entry.path();
        let meta_path = folder.join("metadata.json");
        let Ok(bytes) = std::fs::read(&meta_path) else {
            continue;
        };
        let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        if value.get("meeting_id").and_then(|v| v.as_str()) != Some(meeting_id) {
            continue;
        }
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "status".to_string(),
                serde_json::Value::String("discarded".to_string()),
            );
        }
        // Atomic rewrite (temp + rename), mirroring RecordingSaver::write_metadata.
        let tmp = folder.join(".metadata.json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_string_pretty(&value).unwrap_or_default(),
        )?;
        std::fs::rename(&tmp, &meta_path)?;
        info!(
            "recovery: discarded interrupted recording {} ({})",
            meeting_id,
            folder.display()
        );
        return Ok(true);
    }
    Ok(false)
}

/// [`discard_interrupted_recording`] under whichever of `roots` holds the folder.
pub fn discard_interrupted_in_roots(
    roots: &[std::path::PathBuf],
    meeting_id: &str,
) -> std::io::Result<bool> {
    for root in roots {
        if discard_interrupted_recording(root, meeting_id)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Minimal view of the folder's `transcripts.json` (written incrementally by
/// `RecordingSaver` during a session) — the crashed session's only durable
/// transcript copy, since DB rows are written by the frontend at clean stop.
#[derive(Debug, Deserialize)]
struct FolderTranscripts {
    #[serde(default)]
    segments: Vec<FolderSegment>,
}

#[derive(Debug, Deserialize)]
struct FolderSegment {
    #[serde(default)]
    id: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    audio_start_time: Option<f64>,
    #[serde(default)]
    audio_end_time: Option<f64>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    display_time: Option<String>,
    #[serde(default)]
    channel: Option<String>,
}

/// Read + map a folder's `transcripts.json` segments into API transcript segments
/// (blank-text rows dropped; `display_time` becomes the row timestamp). Empty vec on
/// a missing/unreadable file.
fn load_folder_transcript_segments(folder: &Path) -> Vec<crate::transcripts::TranscriptSegment> {
    let path = folder.join("transcripts.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new(); // no transcripts.json → nothing to import
    };
    let parsed: FolderTranscripts = match serde_json::from_slice(&bytes) {
        Ok(p) => p,
        Err(e) => {
            warn!(
                "resume: unreadable transcripts.json in {}: {e}",
                folder.display()
            );
            return Vec::new();
        }
    };
    parsed
        .segments
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| crate::transcripts::TranscriptSegment {
            id: s.id,
            text: s.text,
            timestamp: s.display_time.unwrap_or_default(),
            audio_start_time: s.audio_start_time,
            audio_end_time: s.audio_end_time,
            duration: s.duration,
            speaker: None,
            channel: s.channel,
            // transcripts.json (write_transcripts_json) never carries word_timestamps.
            word_timestamps: None,
        })
        .collect()
}

/// specs/0037 (review-2): when RESUMING a crash-interrupted recording, import the
/// crashed session's `transcripts.json` into the meeting's DB rows — otherwise the
/// meeting's transcript starts at the resume point and the pre-crash speech is lost
/// to search/summary/diarization even though its audio was recovered.
///
/// Deliberately routed through the WS6.7-GUARDED attach
/// (`save_transcripts_for_meeting`), which refuses a populated meeting — so a clean
/// "Continue recording" (whose session-0 transcripts were saved at its stop) is a
/// natural no-op, and a double import can't duplicate rows. Best-effort: failures are
/// logged, never block the resume (the on-disk copy is preserved by the saver).
pub async fn import_prior_folder_transcripts<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    meeting_id: &str,
    folder: &Path,
) {
    use tauri::Manager;

    let segments = load_folder_transcript_segments(folder);
    if segments.is_empty() {
        return;
    }

    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    let pool = state.db_manager.pool();

    // Keep the meeting's existing title — the attach refreshes title for
    // non-authoritative rows, so pass the current one back (no-op update).
    let title = match crate::database::repositories::meeting::MeetingsRepository::get_meeting(
        pool, meeting_id,
    )
    .await
    {
        Ok(Some(m)) => m.title,
        _ => return, // meeting gone → nothing to attach to
    };

    match crate::database::repositories::transcript::TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        meeting_id,
        &title,
        &segments,
        None,
    )
    .await
    {
        Ok(true) => info!(
            "resume: imported {} pre-crash transcript segment(s) from {} into meeting {}",
            segments.len(),
            folder.display(),
            meeting_id
        ),
        // Ok(false) = the meeting already has transcripts (clean continue) — correct no-op.
        Ok(false) => {}
        Err(e) => warn!(
            "resume: could not import pre-crash transcripts for {}: {e}",
            meeting_id
        ),
    }
}

/// List interrupted recordings offerable for resume on relaunch (specs/0037). Scans
/// the recordings root off-thread, then drops any whose meeting row no longer exists
/// (deleted since the crash) so the prompt never offers a dead resume.
#[tauri::command]
pub async fn api_list_interrupted_recordings(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Vec<InterruptedRecording>, String> {
    let roots = crate::audio::recording_preferences::known_recording_roots();
    let candidates = tokio::task::spawn_blocking(move || scan_interrupted_in_roots(&roots))
        .await
        .map_err(|e| format!("recovery scan task failed: {e}"))?;

    let pool = state.db_manager.pool();
    let mut kept = Vec::with_capacity(candidates.len());
    for c in candidates {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?)")
            .bind(&c.meeting_id)
            .fetch_one(pool)
            .await
            .unwrap_or(false);
        if exists {
            kept.push(c);
        } else {
            info!(
                "recovery: skipping interrupted folder for deleted meeting {}",
                c.meeting_id
            );
        }
    }
    info!(
        "recovery: {} interrupted recording(s) offerable",
        kept.len()
    );
    Ok(kept)
}

/// Discard an interrupted recording (specs/0037) — flips its metadata to "discarded"
/// so it stops being offered, WITHOUT deleting the captured audio/transcripts.
#[tauri::command]
pub async fn api_discard_interrupted_recording(meeting_id: String) -> Result<(), String> {
    // specs/0073: the metadata rewrite must not race a move/copy of the same folder.
    let _folder_lease = crate::audio::folder_lease::acquire(
        &meeting_id,
        crate::audio::folder_lease::LeaseHolder::Discard,
    )
    .await;
    // specs/0073: the folder may be under any known recordings root, not just the current.
    let roots = crate::audio::recording_preferences::known_recording_roots();
    let found =
        tokio::task::spawn_blocking(move || discard_interrupted_in_roots(&roots, &meeting_id))
            .await
            .map_err(|e| format!("discard task failed: {e}"))?
            .map_err(|e| format!("Could not discard the interrupted recording: {e}"))?;
    if !found {
        warn!("recovery: discard requested for an unknown interrupted recording");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nixon-0037-{name}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn write_meta(folder: &Path, json: &str, checkpoints: bool) {
        fs::create_dir_all(folder).unwrap();
        fs::write(folder.join("metadata.json"), json).unwrap();
        if checkpoints {
            fs::create_dir_all(folder.join(".checkpoints")).unwrap();
        }
    }

    #[test]
    fn scan_offers_only_interrupted_with_audio_and_id_newest_first() {
        let root = scratch("scan");
        // interrupted + checkpoints + id → offered
        write_meta(
            &root.join("Standup_2026-07-05_10-00"),
            r#"{"meeting_id":"m-1","meeting_name":"Standup","created_at":"2026-07-05T10:00:00Z","status":"recording","segments":[{"index":0}]}"#,
            true,
        );
        // a newer interrupted one → should sort first
        write_meta(
            &root.join("Sync_2026-07-05_14-00"),
            r#"{"meeting_id":"m-2","meeting_name":"Sync","created_at":"2026-07-05T14:00:00Z","status":"recording","segments":[{"index":0},{"index":1}]}"#,
            true,
        );
        // cleanly completed → skipped
        write_meta(
            &root.join("Done_2026-07-05_09-00"),
            r#"{"meeting_id":"m-3","status":"completed"}"#,
            true,
        );
        // recording but NO checkpoints → skipped (nothing to resume)
        write_meta(
            &root.join("Empty_2026-07-05_11-00"),
            r#"{"meeting_id":"m-4","status":"recording"}"#,
            false,
        );
        // recording + checkpoints but NO meeting_id (pre-0037) → skipped
        write_meta(
            &root.join("Legacy_2026-07-05_12-00"),
            r#"{"status":"recording"}"#,
            true,
        );

        let found = scan_interrupted_recordings(&root);
        assert_eq!(found.len(), 2, "only the two interrupted+audio+id folders");
        assert_eq!(found[0].meeting_id, "m-2", "newest first");
        assert_eq!(found[0].segment_count, 2);
        assert_eq!(found[1].meeting_id, "m-1");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_in_roots_finds_interrupted_recordings_in_every_root_once() {
        let current = scratch("roots-current");
        let earlier = scratch("roots-earlier");
        write_meta(
            &current.join("New_2026-07-06_10-00"),
            r#"{"meeting_id":"m-new","created_at":"2026-07-06T10:00:00Z","status":"recording"}"#,
            true,
        );
        write_meta(
            &earlier.join("Old_2026-07-05_10-00"),
            r#"{"meeting_id":"m-old","created_at":"2026-07-05T10:00:00Z","status":"recording"}"#,
            true,
        );
        // The current root listed twice (a duplicate known root) must not double-report.
        let found = scan_interrupted_in_roots(&[current.clone(), earlier.clone(), current.clone()]);
        let ids: Vec<&str> = found.iter().map(|r| r.meeting_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["m-new", "m-old"],
            "both roots, newest first, no duplicates"
        );

        // Discard finds the folder in the earlier root.
        assert!(
            discard_interrupted_in_roots(&[current.clone(), earlier.clone()], "m-old").unwrap()
        );
        let left: Vec<String> = scan_interrupted_in_roots(&[current.clone(), earlier.clone()])
            .into_iter()
            .map(|r| r.meeting_id)
            .collect();
        assert_eq!(left, vec!["m-new".to_string()]);
        let _ = fs::remove_dir_all(&current);
        let _ = fs::remove_dir_all(&earlier);
    }

    #[test]
    fn scan_of_missing_root_is_empty() {
        let root = scratch("missing").join("does-not-exist");
        assert!(scan_interrupted_recordings(&root).is_empty());
    }

    #[test]
    fn loads_folder_transcripts_dropping_blank_segments() {
        let root = scratch("transcripts");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("transcripts.json"),
            r#"{"version":"1.0","segments":[
                {"id":"s1","text":"hello world","audio_start_time":0.0,"audio_end_time":2.5,"duration":2.5,"display_time":"2026-07-05T10:00:01Z","channel":"microphone"},
                {"id":"s2","text":"   ","audio_start_time":2.5},
                {"id":"s3","text":"still here","audio_start_time":3.0,"audio_end_time":5.0}
            ],"total_segments":3}"#,
        )
        .unwrap();

        let segs = load_folder_transcript_segments(&root);
        assert_eq!(segs.len(), 2, "blank segment dropped");
        assert_eq!(segs[0].text, "hello world");
        assert_eq!(segs[0].timestamp, "2026-07-05T10:00:01Z");
        assert_eq!(segs[0].audio_start_time, Some(0.0));
        assert_eq!(segs[0].channel.as_deref(), Some("microphone"));
        assert_eq!(segs[1].id, "s3");

        // Missing file / unreadable JSON → empty, never a panic.
        assert!(load_folder_transcript_segments(&root.join("nope")).is_empty());
        fs::write(root.join("transcripts.json"), b"{not json").unwrap();
        assert!(load_folder_transcript_segments(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn discard_flips_status_keeps_audio_and_stops_offering() {
        let root = scratch("discard");
        let folder = root.join("Standup_2026-07-05_10-00");
        write_meta(
            &folder,
            r#"{"meeting_id":"m-9","meeting_name":"Standup","created_at":"2026-07-05T10:00:00Z","status":"recording"}"#,
            true,
        );
        assert!(discard_interrupted_recording(&root, "m-9").unwrap());
        assert!(
            scan_interrupted_recordings(&root).is_empty(),
            "discarded recording is no longer offered"
        );
        assert!(
            folder.join(".checkpoints").is_dir(),
            "captured audio is kept, not deleted"
        );
        assert!(
            !discard_interrupted_recording(&root, "no-such-id").unwrap(),
            "unknown id → false"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
