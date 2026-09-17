//! Writes a recording folder in exactly the shape `RecordingSaver` produces (specs/0059).
use super::dataset::FixtureMeeting;
use crate::audio::recording_saver::{
    DeviceInfo, MeetingMetadata, TranscriptSegment as DiskSegment,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

pub const FOLDER_PREFIX: &str = "demo-";

pub fn write_folder(m: &FixtureMeeting, root: &Path, start: DateTime<Utc>) -> Result<PathBuf> {
    let name = format!(
        "{FOLDER_PREFIX}{}_{}",
        crate::audio::audio_processing::sanitize_filename(&m.id),
        start.format("%Y-%m-%d_%H-%M")
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

pub fn remove_demo_folders(root: &Path) -> Result<u32> {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && e.file_name().to_string_lossy().starts_with(FOLDER_PREFIX)
            {
                std::fs::remove_dir_all(e.path())?;
                n += 1;
            }
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
        assert!(folder
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("demo-demo-04_"));
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
        assert_eq!(remove_demo_folders(dir.path()).unwrap(), 1);
        assert!(!folder.exists());
    }
}
