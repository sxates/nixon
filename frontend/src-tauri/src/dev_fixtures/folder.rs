//! Writes a recording folder in exactly the shape `RecordingSaver` produces (specs/0059).
use super::dataset::FixtureMeeting;
use crate::audio::recording_saver::{
    DeviceInfo, MeetingMetadata, TranscriptSegment as DiskSegment,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// specs/0059 fix round 2 (Important 1): distinctive and STABLE across seed re-runs — no
/// date component — so the folder name is the same every time a given meeting id is seeded.
/// That lets `render_meeting_audio`'s `.fixture-hash` cache actually hit on re-runs, and
/// makes cleanup unambiguous: a real user folder can never start with this prefix.
pub const FOLDER_PREFIX: &str = "nixon-demo-";

/// Writes (or rewrites) `<root>/<FOLDER_PREFIX><id-without-its-own-"demo-"-prefix>/`.
/// Idempotent: `create_dir_all` is a no-op when the folder already exists, and
/// `metadata.json`/`transcripts.json` are overwritten in place. Any existing
/// `mic.wav`/`system.wav`/`audio.mp4`/`.fixture-hash` is left untouched so a cached
/// render from a previous run survives (see `audio::render_meeting_audio`).
pub fn write_folder(m: &FixtureMeeting, root: &Path, start: DateTime<Utc>) -> Result<PathBuf> {
    let stripped = m.id.strip_prefix("demo-").unwrap_or(&m.id);
    let name = format!(
        "{FOLDER_PREFIX}{}",
        crate::audio::audio_processing::sanitize_filename(stripped)
    );
    let folder = root.join(name);
    std::fs::create_dir_all(&folder).with_context(|| format!("create {}", folder.display()))?;
    let completed = start + chrono::Duration::seconds(m.duration_seconds as i64);
    let meta = MeetingMetadata {
        version: "1.0".into(),
        meeting_id: Some(m.id.clone()),
        meeting_name: Some(m.title.clone()),
        created_at: start.to_rfc3339(),
        completed_at: Some(completed.to_rfc3339()),
        duration_seconds: Some(m.duration_seconds as f64),
        devices: DeviceInfo {
            microphone: Some("Demo Microphone".into()),
            system_audio: Some("Demo System Audio".into()),
        },
        audio_file: "audio.mp4".into(),
        transcript_file: "transcripts.json".into(),
        sample_rate: 48_000,
        status: "completed".into(),
        segments: Vec::new(),
    };
    std::fs::write(
        folder.join("metadata.json"),
        serde_json::to_string_pretty(&meta)?,
    )?;
    let segments: Vec<DiskSegment> = m
        .segments
        .iter()
        .enumerate()
        .map(|(i, s)| DiskSegment {
            id: s.id.clone(),
            text: s.text.clone(),
            audio_start_time: s.start,
            audio_end_time: s.end,
            duration: s.end - s.start,
            display_time: format!("{:02}:{:02}", (s.start as u64) / 60, (s.start as u64) % 60),
            confidence: 0.95,
            sequence_id: i as u64,
            channel: Some(s.channel.clone()),
        })
        .collect();
    let tj = serde_json::json!({
        "version": "1.0",
        "segments": segments,
        "last_updated": Utc::now().to_rfc3339(),
        "total_segments": segments.len()
    });
    std::fs::write(
        folder.join("transcripts.json"),
        serde_json::to_string_pretty(&tj)?,
    )?;
    Ok(folder)
}

/// Deletes only folders that are unambiguously ours: named with [`FOLDER_PREFIX`] AND
/// whose `metadata.json` parses as a [`MeetingMetadata`] with a `meeting_id` that starts
/// with `"demo-"` and `devices.microphone == Some("Demo Microphone")` (specs/0059 fix
/// round 2, Important 2 — cleanup must never be able to touch a real recording folder,
/// even one a user happened to name with our prefix). `keep_ids` are meeting ids from the
/// dataset that is about to be re-seeded; a stale folder whose id isn't in that set is
/// removed, everything else is skipped with a `log::warn!` naming the folder and why.
pub fn remove_stale_demo_folders(root: &Path, keep_ids: &[&str]) -> Result<u32> {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with(FOLDER_PREFIX) {
                continue;
            }
            let path = e.path();
            let meta: Option<MeetingMetadata> = std::fs::read_to_string(path.join("metadata.json"))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok());
            let meeting_id = meta.as_ref().and_then(|m| {
                let is_demo_meta = m
                    .meeting_id
                    .as_deref()
                    .map(|id| id.starts_with("demo-"))
                    .unwrap_or(false)
                    && m.devices.microphone.as_deref() == Some("Demo Microphone");
                is_demo_meta.then(|| m.meeting_id.clone().unwrap())
            });
            let Some(meeting_id) = meeting_id else {
                log::warn!(
                    "[dev] skipping cleanup of {}: metadata.json is missing/invalid or doesn't \
                     look like a demo folder (meeting_id/devices mismatch)",
                    path.display()
                );
                continue;
            };
            if keep_ids.contains(&meeting_id.as_str()) {
                continue;
            }
            std::fs::remove_dir_all(&path)?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_fixtures::dataset::load_embedded;
    #[test]
    fn writes_metadata_and_transcripts_json_in_app_shape() {
        let dir = tempfile::tempdir().unwrap();
        let ds = load_embedded().unwrap();
        let start = chrono::Utc::now();
        let folder = write_folder(&ds.meetings[3], dir.path(), start).unwrap();
        assert_eq!(
            folder.file_name().unwrap().to_string_lossy(),
            "nixon-demo-04"
        );
        let meta: crate::audio::recording_saver::MeetingMetadata =
            serde_json::from_str(&std::fs::read_to_string(folder.join("metadata.json")).unwrap())
                .unwrap();
        assert_eq!(meta.status, "completed");
        assert_eq!(meta.meeting_id.as_deref(), Some("demo-04"));
        let tj: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(folder.join("transcripts.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(tj["version"], "1.0");
        assert_eq!(
            tj["total_segments"].as_u64().unwrap() as usize,
            ds.meetings[3].segments.len()
        );
        assert_eq!(
            tj["segments"][0]["channel"],
            ds.meetings[3].segments[0].channel
        );
        assert_eq!(
            remove_stale_demo_folders(dir.path(), &[]).unwrap(),
            1,
            "not in keep_ids, so it is stale and removed"
        );
        assert!(!folder.exists());
    }

    #[test]
    fn write_folder_is_idempotent_and_preserves_existing_audio() {
        let dir = tempfile::tempdir().unwrap();
        let ds = load_embedded().unwrap();
        let start = chrono::Utc::now();
        let folder = write_folder(&ds.meetings[0], dir.path(), start).unwrap();
        // Simulate a previously-rendered, cached audio artifact.
        std::fs::write(folder.join("audio.mp4"), b"fake mp4 bytes").unwrap();
        std::fs::write(folder.join(".fixture-hash"), "some-hash").unwrap();

        // Re-running write_folder (as a second `--demo` does) must not fail, must land at
        // the SAME path (no date component), and must leave the cached audio untouched.
        let folder2 = write_folder(&ds.meetings[0], dir.path(), start).unwrap();
        assert_eq!(folder, folder2);
        assert_eq!(
            std::fs::read(folder.join("audio.mp4")).unwrap(),
            b"fake mp4 bytes"
        );
        assert_eq!(
            std::fs::read_to_string(folder.join(".fixture-hash")).unwrap(),
            "some-hash"
        );
    }

    #[test]
    fn remove_stale_demo_folders_keeps_ids_in_the_keep_list() {
        let dir = tempfile::tempdir().unwrap();
        let ds = load_embedded().unwrap();
        let start = chrono::Utc::now();
        let kept = write_folder(&ds.meetings[0], dir.path(), start).unwrap();
        let stale = write_folder(&ds.meetings[1], dir.path(), start).unwrap();
        let keep_ids: Vec<&str> = vec![ds.meetings[0].id.as_str()];
        assert_eq!(remove_stale_demo_folders(dir.path(), &keep_ids).unwrap(), 1);
        assert!(kept.exists(), "kept id must survive cleanup");
        assert!(!stale.exists(), "id not in keep_ids must be removed");
    }

    #[test]
    fn remove_stale_demo_folders_never_touches_a_folder_with_real_looking_metadata() {
        // A folder that happens to be named with our prefix but whose metadata.json
        // does not look like one of ours (real meeting_id, real device names) must
        // never be deleted, no matter what keep_ids says (specs/0059 Important 2).
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join(format!("{FOLDER_PREFIX}not-actually-demo"));
        std::fs::create_dir_all(&folder).unwrap();
        let real_meta = serde_json::json!({
            "version": "1.0",
            "meeting_id": "a-real-meeting-id",
            "meeting_name": "Weekly 1:1",
            "created_at": chrono::Utc::now().to_rfc3339(),
            "completed_at": null,
            "duration_seconds": null,
            "devices": { "microphone": "Built-in Microphone", "system_audio": null },
            "audio_file": "audio.mp4",
            "transcript_file": "transcripts.json",
            "sample_rate": 48000,
            "status": "completed",
            "segments": []
        });
        std::fs::write(
            folder.join("metadata.json"),
            serde_json::to_string_pretty(&real_meta).unwrap(),
        )
        .unwrap();
        assert_eq!(remove_stale_demo_folders(dir.path(), &[]).unwrap(), 0);
        assert!(
            folder.exists(),
            "real-looking metadata must never be deleted"
        );
    }

    #[test]
    fn remove_stale_demo_folders_skips_a_folder_with_no_metadata_json() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join(format!("{FOLDER_PREFIX}no-metadata"));
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(remove_stale_demo_folders(dir.path(), &[]).unwrap(), 0);
        assert!(folder.exists());
    }
}
