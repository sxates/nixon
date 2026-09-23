//! specs/0072 W2: resuming (0037) a meeting whose kept channels the lifecycle compressed to
//! Opus, and one whose audio the retention setting deleted.
//!
//! Real audio: `say` speech as 16 kHz mono WAVs, compressed by the production compressor and
//! joined by the production channel concat, both with the bundled ffmpeg sidecar. Skips
//! cleanly without `say` or the sidecar.

use super::*;
use crate::audio::channel_files::concat_segment_channels_with;
use crate::audio::lifecycle::compress::{compress_channels, decode_stats};

fn sidecar() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("ffmpeg-"))
        })
}

/// Speak `text` into `folder/name` (16 kHz mono PCM, the capture format).
fn say(folder: &Path, name: &str, text: &str) -> bool {
    crate::dev_fixtures::wav::say_to_wav(text, None, &folder.join(name))
}

fn secs(ff: &Path, path: &Path) -> f64 {
    decode_stats(ff, path).unwrap().0 as f64 / 16_000.0
}

/// A lone recorded session as the saver leaves it at stop: plain names in segment 0.
fn plain_session_zero() -> RecordingSegment {
    RecordingSegment {
        audio_file: MEETING_AUDIO_FILENAME.to_string(),
        system_wav: SYSTEM_CHANNEL_FILENAME.to_string(),
        mic_wav: MIC_CHANNEL_FILENAME.to_string(),
        ..seg(0, Some(3.0))
    }
}

fn completed_folder(folder: &Path) {
    let mut meta = sample_metadata(vec![plain_session_zero()]);
    meta.status = "completed".to_string();
    std::fs::write(
        folder.join("metadata.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();
    write_transcripts_file(folder, &[tseg(1, "prior")]);
}

/// Start a resume, then do what the resumed session's stop does to the channels: its new
/// `system_seg01.wav` / `mic_seg01.wav` are joined after every prior segment.
async fn resume_and_stop(ff: &Path, folder: &Path) -> Vec<RecordingSegment> {
    let mut saver = RecordingSaver::new();
    saver.set_resume_context("meeting-abc".to_string(), folder.to_path_buf());
    let _sender = saver.start_accumulation(true).unwrap();
    saver.commit_start();
    let mut segments = saver.metadata.clone().unwrap().segments;
    segments.push(seg(1, Some(2.0)));
    concat_segment_channels_with(ff, folder, &segments);
    segments
}

/// Sabotage #7: with `.wav` hardcoded in `migrate_prior_segments_with`, segment 0's
/// `system.opus` is never scoped and the resumed meeting's channels lose it.
#[tokio::test]
async fn resuming_a_compressed_meeting_keeps_the_first_sessions_channels() {
    let Some(ff) = sidecar() else {
        eprintln!("SKIP resuming_a_compressed_meeting: no ffmpeg sidecar");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    completed_folder(folder);
    std::fs::write(folder.join("audio.mp4"), b"mix").unwrap();
    if !say(
        folder,
        "system.wav",
        "the first session, from the remote side",
    ) || !say(folder, "mic.wav", "the first session, from the owner")
        || !say(folder, "system_seg01.wav", "and the resumed session")
        || !say(folder, "mic_seg01.wav", "resumed owner")
    {
        eprintln!("SKIP resuming_a_compressed_meeting: `say` unavailable");
        return;
    }
    let expected: Vec<f64> = ["system.wav", "system_seg01.wav", "mic.wav", "mic_seg01.wav"]
        .iter()
        .map(|n| secs(&ff, &folder.join(n)))
        .collect();
    // The processed meeting was compressed; the resumed session's files stay WAV (they are
    // written after the compression, while the folder is leased for recording).
    for n in ["system_seg01.wav", "mic_seg01.wav"] {
        std::fs::rename(folder.join(n), folder.join(format!("{n}.later"))).unwrap();
    }
    compress_channels(&ff, folder).unwrap();
    for n in ["system_seg01.wav", "mic_seg01.wav"] {
        std::fs::rename(folder.join(format!("{n}.later")), folder.join(n)).unwrap();
    }
    assert!(folder.join("system.opus").exists() && !folder.join("system.wav").exists());

    let segments = resume_and_stop(&ff, folder).await;

    assert_eq!(segments[0].system_wav, "system_seg00.opus");
    assert_eq!(segments[0].mic_wav, "mic_seg00.opus");
    assert!(folder.join("system_seg00.opus").exists());
    assert!(
        !folder.join("system.opus").exists(),
        "no stale canonical channel"
    );
    let system = secs(&ff, &folder.join("system.wav"));
    let mic = secs(&ff, &folder.join("mic.wav"));
    assert!(
        (system - expected[0] - expected[1]).abs() < 0.1,
        "system {system:.2}s"
    );
    assert!(
        (mic - expected[2] - expected[3]).abs() < 0.1,
        "mic {mic:.2}s"
    );
}

/// A meeting resumed twice, compressed in between: its scoped segments are Opus now and its
/// previous canonical `system.opus` is superseded by the new join.
#[tokio::test]
async fn resuming_again_joins_compressed_segments_and_drops_the_old_canonical_channel() {
    let Some(ff) = sidecar() else {
        eprintln!("SKIP resuming_again: no ffmpeg sidecar");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    for (n, text) in [
        ("system_seg00.wav", "zero"),
        ("system_seg01.wav", "one one"),
        ("system_seg02.wav", "two two two"),
    ] {
        if !say(folder, n, text) {
            eprintln!("SKIP resuming_again: `say` unavailable");
            return;
        }
    }
    let want: f64 = (0..3)
        .map(|i| secs(&ff, &folder.join(format!("system_seg{i:02}.wav"))))
        .sum();
    std::fs::rename(folder.join("system_seg02.wav"), folder.join("new.tmp")).unwrap();
    compress_channels(&ff, folder).unwrap();
    std::fs::rename(folder.join("new.tmp"), folder.join("system_seg02.wav")).unwrap();
    std::fs::write(folder.join("system.opus"), b"the previous join").unwrap();

    concat_segment_channels_with(&ff, folder, &[seg(0, None), seg(1, None), seg(2, None)]);

    let got = secs(&ff, &folder.join("system.wav"));
    assert!(
        (got - want).abs() < 0.1,
        "joined {got:.2}s, segments {want:.2}s"
    );
    assert!(!folder.join("system.opus").exists());
}

/// A meeting whose audio the retention setting deleted still resumes: the new session's
/// channels become the meeting's, and the prior transcript is kept.
#[tokio::test]
async fn resuming_a_purged_meeting_records_the_new_session() {
    let Some(ff) = sidecar() else {
        eprintln!("SKIP resuming_a_purged_meeting: no ffmpeg sidecar");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    completed_folder(folder);
    if !say(folder, "system_seg01.wav", "after the audio was deleted")
        || !say(folder, "mic_seg01.wav", "still here")
    {
        eprintln!("SKIP resuming_a_purged_meeting: `say` unavailable");
        return;
    }
    let want = secs(&ff, &folder.join("system_seg01.wav"));

    let segments = resume_and_stop(&ff, folder).await;

    assert_eq!(segments.len(), 2);
    assert_eq!(
        segments[0].system_wav, "system_seg00.wav",
        "nothing to rename"
    );
    assert_eq!(
        segments[0].duration_seconds,
        Some(3.0),
        "the offset is kept"
    );
    let got = secs(&ff, &folder.join("system.wav"));
    assert!((got - want).abs() < 0.1, "system {got:.2}s");
    assert!(folder.join("mic.wav").exists());
    let transcripts = std::fs::read_to_string(folder.join("transcripts.json")).unwrap();
    assert!(transcripts.contains("prior"));
}
