// Abandoned-recording discard-safety guard (specs/0019 WS6.1), moved from
// api/api.rs (specs/0042 WS3).

use tauri::{AppHandle, Runtime};

use crate::{database::repositories::meeting::MeetingsRepository, state::AppState};

/// specs/0019 WS6.1 — guards the stop handler's "abandoned recording" auto-cleanup
/// against destroying a meeting that actually holds durable content.
///
/// The stop handler treats a recording that ends with zero transcripts (and no typed
/// notes) as abandoned and deletes the row created at start. That false signal — a
/// recording whose transcripts simply hadn't flushed, or a Join & Record meeting that
/// Zoom auto-stopped early — was silently deleting real meetings (and orphaning their
/// audio on disk, since `folder_path` is `NULL` at start). This command answers the
/// only safe question: *is there nothing worth keeping?*
///
/// Returns `true` (safe to auto-discard) ONLY when ALL hold:
/// - the meeting is **not** calendar-linked (`calendar_event_id` is NULL), and
/// - it has **no** persisted transcripts, and
/// - there is **no** audio on disk in its recording folder.
///
/// Any of those being present returns `false` — keep the meeting. This is a *read-only*
/// safety check; it never deletes. Explicit user-initiated deletes go through
/// `api_delete_meeting` and are unaffected (a calendar-linked meeting can still be
/// deleted on purpose).
#[tauri::command]
pub async fn api_recording_is_safe_to_discard<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    folder_path: Option<String>,
) -> Result<bool, String> {
    let pool = state.db_manager.pool();
    recording_is_safe_to_discard(pool, &meeting_id, folder_path.as_deref())
        .await
        .map_err(|e| {
            format!(
                "Failed to evaluate discard safety for {}: {}",
                meeting_id, e
            )
        })
}

/// specs/0019 WS6.1 — the durability decision behind `api_recording_is_safe_to_discard`,
/// factored out of the Tauri command so it can be integration-tested over a real pool
/// (the command itself needs an `AppHandle`/`State` that tests can't construct).
///
/// Returns `true` (safe to auto-discard) ONLY when ALL hold: the meeting is **not**
/// calendar-linked, has **no** persisted transcripts, and has **no** on-disk audio.
/// A missing row is safe (nothing to protect). `folder_path` (from the stop event)
/// takes precedence over the row's stored folder when checking for audio.
pub async fn recording_is_safe_to_discard(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    folder_path: Option<&str>,
) -> Result<bool, sqlx::Error> {
    // 1) Calendar-linked meetings represent explicit user intent — never auto-discard.
    let meeting = MeetingsRepository::get_meeting_metadata(pool, meeting_id).await?;
    let (calendar_event_id, stored_folder) = match meeting {
        Some(m) => (m.calendar_event_id, m.folder_path),
        None => return Ok(true), // no such row (already gone / never created)
    };
    if calendar_event_id
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        return Ok(false);
    }

    // 2) Any persisted transcripts → keep.
    let transcript_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_one(pool)
            .await?;
    if transcript_count > 0 {
        return Ok(false);
    }

    // 3) Any audio on disk → keep (transcripts may simply not have flushed). The
    //    passed-in folder (from the stop event) wins over the row's stored folder.
    let folder: Option<String> = folder_path
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.to_string())
        .or(stored_folder);
    Ok(!recording_folder_has_audio(folder.as_deref()))
}

/// True if `folder_path` resolves to a directory containing at least one non-empty
/// audio file. Used by `api_recording_is_safe_to_discard` to avoid deleting a meeting
/// whose audio was captured even when no transcripts landed. Best-effort: any IO error
/// (or absent folder) reads as "no audio" so it never blocks the caller.
fn recording_folder_has_audio(folder_path: Option<&str>) -> bool {
    const AUDIO_EXTS: [&str; 5] = ["mp4", "m4a", "wav", "webm", "mp3"];
    let folder = match folder_path {
        Some(p) if !p.trim().is_empty() => p.trim(),
        _ => return false,
    };
    let entries = match std::fs::read_dir(folder) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_audio = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false);
        if is_audio {
            // A non-empty audio file counts; a 0-byte stub does not.
            if entry.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                return true;
            }
        }
    }
    // specs/0037: a crash-interrupted recording has NO finalized top-level audio — its
    // captured audio lives only in `.checkpoints/*audio_chunk_*.mp4`. Count those too, so
    // the abandoned-recording cleanup never deletes a recoverable recording. Filter to the
    // actual chunk files (same predicate the recovery merge uses) — a stray .DS_Store or
    // leftover concat list must NOT make an audio-less folder permanently uncleanable.
    if let Ok(checkpoints) = std::fs::read_dir(std::path::Path::new(folder).join(".checkpoints")) {
        for entry in checkpoints.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_chunk = name.contains("audio_chunk_") && name.ends_with(".mp4");
            if is_chunk && entry.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

/// specs/0019 WS6.1 — the on-disk-audio guard that keeps a captured-but-untranscribed
/// recording from being auto-discarded by the stop handler.
#[cfg(test)]
mod discard_guard_tests {
    use super::recording_folder_has_audio;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    /// A throwaway dir under the OS temp dir; removed on drop.
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let mut p = std::env::temp_dir();
            p.push(format!("nixon_discard_test_{}_{}", tag, std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
        fn file(&self, name: &str, bytes: &[u8]) {
            let mut f = fs::File::create(self.0.join(name)).unwrap();
            f.write_all(bytes).unwrap();
        }
        fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn none_or_empty_or_missing_path_has_no_audio() {
        assert!(!recording_folder_has_audio(None));
        assert!(!recording_folder_has_audio(Some("")));
        assert!(!recording_folder_has_audio(Some("   ")));
        assert!(!recording_folder_has_audio(Some("/no/such/folder/xyz123")));
    }

    #[test]
    fn detects_non_empty_audio_file() {
        let dir = TmpDir::new("audio");
        dir.file("audio.mp4", b"\x00\x01\x02non-empty");
        assert!(recording_folder_has_audio(Some(dir.path())));
    }

    #[test]
    fn detects_checkpoint_audio_when_no_finalized_top_level_file() {
        // specs/0037: a crash-interrupted recording has audio ONLY in .checkpoints/;
        // the guard must see it so abandoned-cleanup never deletes a recoverable folder.
        let dir = TmpDir::new("checkpoints");
        let cp = dir.0.join(".checkpoints");
        fs::create_dir_all(&cp).unwrap();
        let mut f = fs::File::create(cp.join("audio_chunk_000.mp4")).unwrap();
        f.write_all(b"\x00\x01captured").unwrap();
        assert!(
            recording_folder_has_audio(Some(dir.path())),
            "loose checkpoint audio must count as captured audio"
        );
    }

    #[test]
    fn stray_non_chunk_files_in_checkpoints_do_not_count_as_audio() {
        // A .DS_Store or leftover concat list must not make an audio-less folder
        // permanently ineligible for cleanup (review-2 finding).
        let dir = TmpDir::new("checkpoints_stray");
        let cp = dir.0.join(".checkpoints");
        fs::create_dir_all(&cp).unwrap();
        for name in [".DS_Store", "concat_list.txt", ".metadata.json.tmp"] {
            let mut f = fs::File::create(cp.join(name)).unwrap();
            f.write_all(b"not audio").unwrap();
        }
        assert!(
            !recording_folder_has_audio(Some(dir.path())),
            "stray non-chunk files must not read as captured audio"
        );
        // A seg-prefixed chunk (resumed session) still counts.
        let mut f = fs::File::create(cp.join("seg01_audio_chunk_000.mp4")).unwrap();
        f.write_all(b"\x00\x01captured").unwrap();
        assert!(recording_folder_has_audio(Some(dir.path())));
    }

    #[test]
    fn detects_audio_regardless_of_extension_case() {
        let dir = TmpDir::new("case");
        dir.file("mix.WAV", b"riff-ish bytes");
        assert!(recording_folder_has_audio(Some(dir.path())));
    }

    #[test]
    fn ignores_zero_byte_audio_stub() {
        let dir = TmpDir::new("stub");
        dir.file("audio.mp4", b"");
        assert!(!recording_folder_has_audio(Some(dir.path())));
    }

    #[test]
    fn ignores_non_audio_files() {
        let dir = TmpDir::new("nonaudio");
        dir.file("notes.txt", b"hello");
        dir.file("transcript.json", b"[]");
        assert!(!recording_folder_has_audio(Some(dir.path())));
    }
}
