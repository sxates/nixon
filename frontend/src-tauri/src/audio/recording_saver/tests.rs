use super::*;

mod compressed_resume; // specs/0072 W2

fn sample_metadata(segments: Vec<RecordingSegment>) -> MeetingMetadata {
    MeetingMetadata {
        version: "1.0".to_string(),
        meeting_id: Some("meeting-abc".to_string()),
        meeting_name: Some("Test".to_string()),
        created_at: "2026-07-05T10:00:00+00:00".to_string(),
        completed_at: None,
        duration_seconds: None,
        devices: DeviceInfo {
            microphone: None,
            system_audio: None,
        },
        audio_file: MEETING_AUDIO_FILENAME.to_string(),
        transcript_file: "transcripts.json".to_string(),
        sample_rate: 48000,
        status: "recording".to_string(),
        segments,
    }
}

fn seg(index: u32, duration: Option<f64>) -> RecordingSegment {
    RecordingSegment {
        index,
        started_at: "2026-07-05T10:00:00+00:00".to_string(),
        completed_at: Some("2026-07-05T10:01:00+00:00".to_string()),
        duration_seconds: duration,
        audio_file: format!("audio_seg{:02}.mp4", index),
        system_wav: format!("system_seg{:02}.wav", index),
        mic_wav: format!("mic_seg{:02}.wav", index),
    }
}

#[test]
fn recording_segment_serde_round_trip() {
    let s = seg(1, Some(42.5));
    let json = serde_json::to_string(&s).unwrap();
    let back: RecordingSegment = serde_json::from_str(&json).unwrap();
    assert_eq!(back.index, 1);
    assert_eq!(back.duration_seconds, Some(42.5));
    assert_eq!(back.audio_file, "audio_seg01.mp4");
    assert_eq!(back.system_wav, "system_seg01.wav");
    assert_eq!(back.mic_wav, "mic_seg01.wav");
}

#[test]
fn metadata_round_trip_preserves_segments() {
    let meta = sample_metadata(vec![seg(0, Some(10.0)), seg(1, Some(20.0))]);
    let json = serde_json::to_string_pretty(&meta).unwrap();
    let back: MeetingMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(back.segments.len(), 2);
    assert_eq!(back.segments[1].index, 1);
}

#[test]
fn old_metadata_without_segments_field_defaults_to_empty() {
    // A pre-0037 metadata.json has no `segments` key at all.
    let old_json = r#"{
        "version": "1.0",
        "meeting_id": "meeting-legacy",
        "meeting_name": "Legacy",
        "created_at": "2026-01-01T00:00:00+00:00",
        "completed_at": "2026-01-01T00:10:00+00:00",
        "duration_seconds": 600.0,
        "devices": { "microphone": null, "system_audio": null },
        "audio_file": "audio.mp4",
        "transcript_file": "transcripts.json",
        "sample_rate": 48000,
        "status": "completed"
    }"#;
    let meta: MeetingMetadata = serde_json::from_str(old_json).unwrap();
    assert!(
        meta.segments.is_empty(),
        "missing segments field must default to empty vec"
    );
    assert_eq!(meta.duration_seconds, Some(600.0));
}

/// A minimal transcript segment for prior/current transcript tests.
fn tseg(sequence_id: u64, text: &str) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("seg_{}", sequence_id),
        text: text.to_string(),
        audio_start_time: sequence_id as f64,
        audio_end_time: sequence_id as f64 + 1.0,
        duration: 1.0,
        display_time: "[00:00]".to_string(),
        confidence: 1.0,
        sequence_id,
        channel: Some("microphone".to_string()),
    }
}

#[test]
fn migrate_synthesizes_segment_zero_for_pre_0037_folder() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    // Simulate a completed pre-0037 meeting: plain files on disk, no segments.
    std::fs::write(folder.join("audio.mp4"), b"fake-mp4").unwrap();
    std::fs::write(folder.join("system.wav"), b"fake-wav").unwrap();
    std::fs::write(folder.join("mic.wav"), b"fake-wav").unwrap();
    let mut meta = sample_metadata(vec![]);
    meta.duration_seconds = Some(123.0);

    let mut journal = Vec::new();
    RecordingSaver::migrate_prior_segments(folder, &mut meta, &mut journal);

    assert_eq!(meta.segments.len(), 1);
    let s0 = &meta.segments[0];
    assert_eq!(s0.index, 0);
    assert_eq!(s0.duration_seconds, Some(123.0));
    // Files renamed to segment-scoped names, metadata updated to match.
    assert_eq!(s0.audio_file, "audio_seg00.mp4");
    assert_eq!(s0.system_wav, "system_seg00.wav");
    assert_eq!(s0.mic_wav, "mic_seg00.wav");
    assert!(folder.join("audio_seg00.mp4").exists());
    assert!(folder.join("system_seg00.wav").exists());
    assert!(folder.join("mic_seg00.wav").exists());
    assert!(!folder.join("audio.mp4").exists());
    // All three renames journaled for rollback.
    assert_eq!(journal.len(), 3);
    assert!(journal
        .iter()
        .any(|(src, dst)| src.ends_with("audio.mp4") && dst.ends_with("audio_seg00.mp4")));
}

#[test]
fn migrate_recovers_crashed_first_session_from_loose_checkpoints() {
    // A crash-interrupted first recording: status still "recording", finalize never
    // ran, so there is NO audio.mp4/system.wav/mic.wav — the captured audio survives
    // ONLY as loose `.checkpoints/audio_chunk_*.mp4`, and metadata has no segments and
    // no duration.
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let checkpoints = folder.join(".checkpoints");
    std::fs::create_dir_all(&checkpoints).unwrap();
    std::fs::write(checkpoints.join("audio_chunk_000.mp4"), b"chunk0").unwrap();
    std::fs::write(checkpoints.join("audio_chunk_001.mp4"), b"chunk1").unwrap();

    let mut meta = sample_metadata(vec![]);
    meta.duration_seconds = None; // crash: never finalized

    // Fake concat writes a non-empty merged file (can't run real ffmpeg on synthetic
    // bytes); fake probe returns None to force the chunk-count fallback.
    let seen_inputs = std::cell::RefCell::new(Vec::new());
    let concat = |inputs: &[PathBuf], out: &Path| -> anyhow::Result<()> {
        *seen_inputs.borrow_mut() = inputs.to_vec();
        std::fs::write(out, b"merged-audio").unwrap();
        Ok(())
    };
    let probe = |_: &Path| -> Option<f64> { None };

    RecordingSaver::migrate_prior_segments_with(
        folder,
        &mut meta,
        &mut Vec::new(),
        &concat,
        &probe,
    );

    // segment[0] now points at a REAL, non-empty audio_seg00.mp4.
    assert_eq!(meta.segments.len(), 1);
    let s0 = &meta.segments[0];
    assert_eq!(s0.index, 0);
    assert_eq!(s0.audio_file, "audio_seg00.mp4");
    let merged = folder.join("audio_seg00.mp4");
    assert!(merged.exists(), "merged seg00 audio must exist");
    assert!(
        std::fs::metadata(&merged).unwrap().len() > 0,
        "merged seg00 audio must be non-empty"
    );

    // Duration is a clearly non-None, positive value (2 chunks × 30s fallback).
    assert_eq!(s0.duration_seconds, Some(60.0));

    // Both chunks merged, then consumed.
    assert_eq!(seen_inputs.borrow().len(), 2);
    assert!(!checkpoints.join("audio_chunk_000.mp4").exists());
    assert!(!checkpoints.join("audio_chunk_001.mp4").exists());
}

/// FIX 2 (specs/0037): a crash DURING a resumed session strands `seg{NN}_` chunks
/// with NO segment entry (segments are only pushed at stop). Migrate must synthesize
/// the segment, merge its chunks into `audio_seg{NN}.mp4`, claim the crashed
/// session's plain per-channel WAVs, and push the next index PAST it.
#[test]
fn migrate_recovers_crashed_resumed_session_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let checkpoints = folder.join(".checkpoints");
    std::fs::create_dir_all(&checkpoints).unwrap();

    // Session 0 completed cleanly at a previous resume (already scoped).
    std::fs::write(folder.join("audio_seg00.mp4"), b"seg0").unwrap();
    std::fs::write(folder.join("system_seg00.wav"), b"seg0").unwrap();
    std::fs::write(folder.join("mic_seg00.wav"), b"seg0").unwrap();
    // The crashed RESUMED session (would-be segment 1): prefixed chunks + the plain
    // per-channel WAVs the pipeline wrote for it (renamed only at stop → still plain).
    std::fs::write(checkpoints.join("seg01_audio_chunk_000.mp4"), b"c0").unwrap();
    std::fs::write(checkpoints.join("seg01_audio_chunk_001.mp4"), b"c1").unwrap();
    std::fs::write(folder.join("system.wav"), b"crashed-session").unwrap();
    std::fs::write(folder.join("mic.wav"), b"crashed-session").unwrap();

    let mut meta = sample_metadata(vec![seg(0, Some(30.0))]);

    let seen = std::cell::RefCell::new(Vec::new());
    let concat = |inputs: &[PathBuf], out: &Path| -> anyhow::Result<()> {
        seen.borrow_mut().push((inputs.to_vec(), out.to_path_buf()));
        std::fs::write(out, b"merged").unwrap();
        Ok(())
    };
    let probe = |_: &Path| -> Option<f64> { None };

    let mut journal = Vec::new();
    RecordingSaver::migrate_prior_segments_with(folder, &mut meta, &mut journal, &concat, &probe);

    // Segment 1 synthesized with scoped filenames and an estimated duration.
    assert_eq!(meta.segments.len(), 2);
    let s1 = &meta.segments[1];
    assert_eq!(s1.index, 1);
    assert_eq!(s1.audio_file, "audio_seg01.mp4");
    assert_eq!(s1.system_wav, "system_seg01.wav");
    assert_eq!(s1.mic_wav, "mic_seg01.wav");
    assert_eq!(s1.duration_seconds, Some(60.0), "2 chunks × 30s fallback");
    assert!(s1.completed_at.is_none(), "crashed session never completed");

    // Chunks merged into audio_seg01.mp4 and consumed; seg0 untouched (one concat).
    assert_eq!(seen.borrow().len(), 1);
    assert!(seen.borrow()[0].1.ends_with("audio_seg01.mp4"));
    assert!(folder.join("audio_seg01.mp4").exists());
    assert!(!checkpoints.join("seg01_audio_chunk_000.mp4").exists());
    assert!(!checkpoints.join("seg01_audio_chunk_001.mp4").exists());

    // Plain WAVs claimed for segment 1 (journaled), so the NEW session can't clobber them.
    assert!(folder.join("system_seg01.wav").exists());
    assert!(folder.join("mic_seg01.wav").exists());
    assert!(!folder.join("system.wav").exists());
    assert!(!folder.join("mic.wav").exists());
    assert_eq!(journal.len(), 2);

    // Index allocation can never reuse the crashed session's slot.
    assert_eq!(RecordingSaver::next_segment_index(&meta), 2);
}

/// FIX 2 failure path: when the crashed resumed session's merge fails, its chunks
/// are PRESERVED (only copy), the segment is still synthesized (fallback duration,
/// index reserved), and a later migrate retries the merge successfully.
#[test]
fn migrate_preserves_chunks_and_retries_when_resumed_merge_fails() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let checkpoints = folder.join(".checkpoints");
    std::fs::create_dir_all(&checkpoints).unwrap();
    std::fs::write(folder.join("audio_seg00.mp4"), b"seg0").unwrap();
    std::fs::write(checkpoints.join("seg01_audio_chunk_000.mp4"), b"c0").unwrap();
    std::fs::write(checkpoints.join("seg01_audio_chunk_001.mp4"), b"c1").unwrap();

    let mut meta = sample_metadata(vec![seg(0, Some(30.0))]);

    let failing = |_: &[PathBuf], _: &Path| -> anyhow::Result<()> {
        Err(anyhow::anyhow!("simulated ffmpeg failure"))
    };
    let probe = |_: &Path| -> Option<f64> { None };
    RecordingSaver::migrate_prior_segments_with(
        folder,
        &mut meta,
        &mut Vec::new(),
        &failing,
        &probe,
    );

    // Chunks preserved; segment synthesized with fallback duration; index reserved.
    assert!(checkpoints.join("seg01_audio_chunk_000.mp4").exists());
    assert!(checkpoints.join("seg01_audio_chunk_001.mp4").exists());
    assert_eq!(meta.segments.len(), 2);
    assert_eq!(meta.segments[1].duration_seconds, Some(60.0));
    assert_eq!(RecordingSaver::next_segment_index(&meta), 2);

    // A later resume retries the merge (audio still missing + chunks survive).
    let succeeding = |_: &[PathBuf], out: &Path| -> anyhow::Result<()> {
        std::fs::write(out, b"merged").unwrap();
        Ok(())
    };
    let probe2 = |_: &Path| -> Option<f64> { Some(58.0) };
    RecordingSaver::migrate_prior_segments_with(
        folder,
        &mut meta,
        &mut Vec::new(),
        &succeeding,
        &probe2,
    );
    assert!(folder.join("audio_seg01.mp4").exists());
    assert!(!checkpoints.join("seg01_audio_chunk_000.mp4").exists());
    assert_eq!(
        meta.segments.len(),
        2,
        "retry must not duplicate the segment"
    );
    assert_eq!(meta.segments[1].duration_seconds, Some(58.0));
}

/// FIX 2 defence-in-depth: a merged `audio_seg{NN}.mp4` with no metadata entry (crash
/// or rolled-back start between the recovery merge and the metadata write) is adopted
/// so its index can't be reused and its duration counts toward the append offset.
#[test]
fn migrate_adopts_orphan_segment_audio_files() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    std::fs::write(folder.join("audio_seg00.mp4"), b"seg0").unwrap();
    std::fs::write(folder.join("audio_seg01.mp4"), b"orphan").unwrap();

    let mut meta = sample_metadata(vec![seg(0, Some(30.0))]);

    let concat = |_: &[PathBuf], _: &Path| -> anyhow::Result<()> {
        panic!("no chunks — concat must not run")
    };
    let probe = |_: &Path| -> Option<f64> { Some(33.0) };
    RecordingSaver::migrate_prior_segments_with(
        folder,
        &mut meta,
        &mut Vec::new(),
        &concat,
        &probe,
    );

    assert_eq!(meta.segments.len(), 2);
    assert_eq!(meta.segments[1].index, 1);
    assert_eq!(meta.segments[1].audio_file, "audio_seg01.mp4");
    assert_eq!(meta.segments[1].duration_seconds, Some(33.0));
    assert_eq!(RecordingSaver::next_segment_index(&meta), 2);
}

/// FIX 4 self-heal: a segment whose audio exists but whose duration was never
/// persisted (e.g. its recovery-merge bookkeeping was rolled back by a failed start)
/// gets its duration re-probed so the append offset isn't zero.
#[test]
fn migrate_fills_missing_duration_by_probing_existing_audio() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    std::fs::write(folder.join("audio_seg00.mp4"), b"merged-earlier").unwrap();

    let mut meta = sample_metadata(vec![seg(0, None)]);

    let concat = |_: &[PathBuf], _: &Path| -> anyhow::Result<()> {
        panic!("audio exists — concat must not run")
    };
    let probe = |_: &Path| -> Option<f64> { Some(12.25) };
    RecordingSaver::migrate_prior_segments_with(
        folder,
        &mut meta,
        &mut Vec::new(),
        &concat,
        &probe,
    );

    assert_eq!(meta.segments[0].duration_seconds, Some(12.25));
}

#[test]
fn migrate_uses_probed_duration_when_available() {
    // When the merged file's real duration can be probed, it wins over the estimate.
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let checkpoints = folder.join(".checkpoints");
    std::fs::create_dir_all(&checkpoints).unwrap();
    std::fs::write(checkpoints.join("audio_chunk_000.mp4"), b"chunk0").unwrap();

    let mut meta = sample_metadata(vec![]);
    meta.duration_seconds = None;

    let concat = |_: &[PathBuf], out: &Path| -> anyhow::Result<()> {
        std::fs::write(out, b"merged").unwrap();
        Ok(())
    };
    let probe = |_: &Path| -> Option<f64> { Some(27.5) };

    RecordingSaver::migrate_prior_segments_with(
        folder,
        &mut meta,
        &mut Vec::new(),
        &concat,
        &probe,
    );

    assert_eq!(meta.segments[0].duration_seconds, Some(27.5));
}

#[test]
fn migrate_leaves_chunks_and_sets_fallback_duration_when_concat_fails() {
    // If the merge itself fails, we must NOT delete the chunks (they're the only copy)
    // and still hand segment 0 a non-zero duration so the append offset isn't 0.
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let checkpoints = folder.join(".checkpoints");
    std::fs::create_dir_all(&checkpoints).unwrap();
    std::fs::write(checkpoints.join("audio_chunk_000.mp4"), b"chunk0").unwrap();
    std::fs::write(checkpoints.join("audio_chunk_001.mp4"), b"chunk1").unwrap();

    let mut meta = sample_metadata(vec![]);
    meta.duration_seconds = None;

    let concat = |_: &[PathBuf], _: &Path| -> anyhow::Result<()> {
        Err(anyhow::anyhow!("simulated ffmpeg failure"))
    };
    let probe = |_: &Path| -> Option<f64> { None };

    RecordingSaver::migrate_prior_segments_with(
        folder,
        &mut meta,
        &mut Vec::new(),
        &concat,
        &probe,
    );

    // Chunks preserved (unrecovered audio must not be thrown away).
    assert!(checkpoints.join("audio_chunk_000.mp4").exists());
    assert!(checkpoints.join("audio_chunk_001.mp4").exists());
    // Non-None, positive fallback duration.
    assert_eq!(meta.segments[0].duration_seconds, Some(60.0));
}

#[test]
fn migrate_is_idempotent_for_already_scoped_segments() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    // Already-resumed meeting: segment names are scoped; a derived audio.mp4 (concat
    // output) also sits in the folder but is NOT referenced by any segment.
    std::fs::write(folder.join("audio_seg00.mp4"), b"a").unwrap();
    std::fs::write(folder.join("audio_seg01.mp4"), b"b").unwrap();
    std::fs::write(folder.join("audio.mp4"), b"derived").unwrap();
    let mut meta = sample_metadata(vec![seg(0, Some(10.0)), seg(1, Some(5.0))]);

    let mut journal = Vec::new();
    RecordingSaver::migrate_prior_segments(folder, &mut meta, &mut journal);

    // No plain-named segments → nothing renamed; the derived audio.mp4 is untouched.
    assert_eq!(meta.segments.len(), 2);
    assert_eq!(meta.segments[0].audio_file, "audio_seg00.mp4");
    assert!(folder.join("audio.mp4").exists());
    assert!(
        journal.is_empty(),
        "idempotent migrate must journal no renames"
    );
}

#[test]
fn next_segment_index_skips_past_recovered_indices() {
    // Plain sequential history: next = len.
    assert_eq!(
        RecordingSaver::next_segment_index(&sample_metadata(vec![])),
        0
    );
    assert_eq!(
        RecordingSaver::next_segment_index(&sample_metadata(vec![seg(0, Some(1.0))])),
        1
    );
    assert_eq!(
        RecordingSaver::next_segment_index(&sample_metadata(vec![
            seg(0, Some(1.0)),
            seg(1, Some(1.0)),
        ])),
        2
    );
    // Gapped history (a recovered crashed segment can leave gaps): next must go PAST
    // the highest index — never reuse it, which would overwrite recovered audio.
    assert_eq!(
        RecordingSaver::next_segment_index(&sample_metadata(vec![
            seg(0, Some(1.0)),
            seg(2, Some(1.0)),
        ])),
        3
    );
}

#[test]
fn prior_audio_duration_sums_existing_segment_durations() {
    // Fresh recording: no resume context → offset is 0.
    let saver = RecordingSaver::new();
    assert_eq!(saver.prior_audio_duration_seconds(), 0.0);

    // The resume offset is the sum of existing segment durations (some may be None).
    let meta = sample_metadata(vec![seg(0, Some(30.0)), seg(1, Some(15.5)), seg(2, None)]);
    let sum: f64 = meta
        .segments
        .iter()
        .filter_map(|s| s.duration_seconds)
        .sum();
    assert_eq!(sum, 45.5);
}

#[test]
fn set_resume_context_enables_resume_and_sets_meeting_id() {
    let mut saver = RecordingSaver::new();
    assert!(saver.resume_folder.is_none());
    saver.set_resume_context("meeting-xyz".to_string(), PathBuf::from("/tmp/folder"));
    assert_eq!(saver.resume_folder, Some(PathBuf::from("/tmp/folder")));
    assert_eq!(saver.meeting_id, Some("meeting-xyz".to_string()));
}

/// Write a transcripts.json in the exact shape `write_transcripts_json` produces.
fn write_transcripts_file(folder: &Path, segments: &[TranscriptSegment]) {
    let json = serde_json::json!({
        "version": "1.0",
        "segments": segments,
        "last_updated": "2026-07-05T10:00:00+00:00",
        "total_segments": segments.len(),
    });
    std::fs::write(
        folder.join("transcripts.json"),
        serde_json::to_string_pretty(&json).unwrap(),
    )
    .unwrap();
}

/// FIX 3 (specs/0037): the resumed session's first incremental transcript write must
/// PREPEND the prior sessions' segments — not atomically overwrite the crashed
/// session's only on-disk transcript copy.
#[tokio::test]
async fn resume_preserves_prior_transcripts_json_segments() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().to_path_buf();
    let meta = sample_metadata(vec![seg(0, Some(30.0))]);
    std::fs::write(
        folder.join("metadata.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();
    std::fs::write(folder.join("audio_seg00.mp4"), b"seg0").unwrap();
    let prior = vec![tseg(1, "hello from the crashed session")];
    write_transcripts_file(&folder, &prior);

    let mut saver = RecordingSaver::new();
    saver.set_resume_context("meeting-abc".to_string(), folder.clone());
    // auto_save=false keeps the test free of ffmpeg/checkpoints; transcript
    // preservation is independent of audio saving.
    let _sender = saver.start_accumulation(false).unwrap();

    assert_eq!(saver.prior_transcript_segments.len(), 1);

    // First live segment of the resumed session → triggers the incremental write.
    saver.add_transcript_segment(tseg(2, "hello from the resumed session"));

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("transcripts.json")).unwrap())
            .unwrap();
    let segments = written["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 2, "prior + current");
    assert_eq!(
        segments[0]["text"], "hello from the crashed session",
        "prior segments must come first"
    );
    assert_eq!(segments[1]["text"], "hello from the resumed session");
    assert_eq!(written["total_segments"], 2);

    // The reload-sync path must still see ONLY the current session.
    let live = saver.get_transcript_segments();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].sequence_id, 2);
}

/// FIX 4a (specs/0037): a resume-init failure must PROPAGATE out of
/// `start_accumulation` — never a live-looking session that records nothing.
#[tokio::test]
async fn resume_init_failure_propagates_from_start_accumulation() {
    // Missing folder.
    let mut saver = RecordingSaver::new();
    saver.set_resume_context(
        "meeting-abc".to_string(),
        PathBuf::from("/nonexistent/nixon-resume-folder"),
    );
    assert!(saver.start_accumulation(true).is_err());

    // Folder exists but metadata.json is missing/unreadable.
    let dir = tempfile::tempdir().unwrap();
    let mut saver = RecordingSaver::new();
    saver.set_resume_context("meeting-abc".to_string(), dir.path().to_path_buf());
    assert!(saver.start_accumulation(true).is_err());

    // Fresh sessions stay best-effort: no meeting name → no folder, but Ok.
    let mut saver = RecordingSaver::new();
    assert!(saver.start_accumulation(false).is_ok());
}

/// FIX 4b (specs/0037): when the start fails AFTER resume init, the rollback must
/// leave a cleanly-completed folder exactly as it was — status "completed" (scans
/// as NOT interrupted), canonical plain filenames restored, no leftover
/// `.checkpoints/` phantom-offering recovery at every launch.
#[tokio::test]
async fn rollback_failed_start_restores_completed_folder() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().to_path_buf();
    let mut meta = sample_metadata(vec![]);
    meta.status = "completed".to_string();
    meta.completed_at = Some("2026-07-05T11:00:00+00:00".to_string());
    meta.duration_seconds = Some(600.0);
    let original_metadata = serde_json::to_string_pretty(&meta).unwrap();
    std::fs::write(folder.join("metadata.json"), &original_metadata).unwrap();
    std::fs::write(folder.join("audio.mp4"), b"audio").unwrap();
    std::fs::write(folder.join("system.wav"), b"system").unwrap();
    std::fs::write(folder.join("mic.wav"), b"mic").unwrap();
    write_transcripts_file(&folder, &[tseg(1, "prior")]);

    let mut saver = RecordingSaver::new();
    saver.set_resume_context("meeting-abc".to_string(), folder.clone());
    let _sender = saver.start_accumulation(true).unwrap();

    // Init really was destructive (the thing rollback exists to undo).
    assert!(folder.join("audio_seg00.mp4").exists());
    assert!(!folder.join("audio.mp4").exists());
    assert!(folder.join(".checkpoints").is_dir());
    let mutated: MeetingMetadata =
        serde_json::from_str(&std::fs::read_to_string(folder.join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(mutated.status, "recording");

    // Simulate the pipeline/stream start failing → rollback.
    saver.rollback_failed_start().await;

    // Canonical filenames restored.
    assert!(folder.join("audio.mp4").exists());
    assert!(folder.join("system.wav").exists());
    assert!(folder.join("mic.wav").exists());
    assert!(!folder.join("audio_seg00.mp4").exists());
    assert!(!folder.join("system_seg00.wav").exists());
    assert!(!folder.join("mic_seg00.wav").exists());

    // Original metadata restored VERBATIM (status "completed" → scans as NOT
    // interrupted) and the phantom .checkpoints dir removed.
    assert_eq!(
        std::fs::read_to_string(folder.join("metadata.json")).unwrap(),
        original_metadata
    );
    assert!(!folder.join(".checkpoints").exists());

    // Prior transcripts untouched on disk; saver state cleared.
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("transcripts.json")).unwrap())
            .unwrap();
    assert_eq!(written["segments"].as_array().unwrap().len(), 1);
    assert!(saver.meeting_folder.is_none());
    assert!(saver.resume_init_journal.is_none());

    // A successful start instead COMMITS: the journal is dropped so nothing can
    // unwind a live session later.
    let mut saver = RecordingSaver::new();
    saver.set_resume_context("meeting-abc".to_string(), folder.clone());
    let _sender = saver.start_accumulation(true).unwrap();
    assert!(saver.resume_init_journal.is_some());
    saver.commit_start();
    assert!(saver.resume_init_journal.is_none());
}

/// FIX 4b counterpart: rolling back a failed resume onto a CRASHED folder must keep
/// it offerable (status "recording", `.checkpoints/` preserved) and keep the merged
/// recovery audio so the next resume self-heals via probe/orphan adoption.
#[tokio::test]
async fn rollback_failed_start_keeps_crashed_folder_offerable() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().to_path_buf();
    let mut meta = sample_metadata(vec![]);
    meta.status = "recording".to_string(); // crashed: never completed
    meta.duration_seconds = None;
    let original_metadata = serde_json::to_string_pretty(&meta).unwrap();
    std::fs::write(folder.join("metadata.json"), &original_metadata).unwrap();
    let checkpoints = folder.join(".checkpoints");
    std::fs::create_dir_all(&checkpoints).unwrap();
    // Unmergeable chunk (fake bytes → real ffmpeg concat fails) so the chunk is
    // PRESERVED through init and must still be there after rollback.
    std::fs::write(checkpoints.join("audio_chunk_000.mp4"), b"chunk").unwrap();

    let mut saver = RecordingSaver::new();
    saver.set_resume_context("meeting-abc".to_string(), folder.clone());
    let _sender = saver.start_accumulation(true).unwrap();

    saver.rollback_failed_start().await;

    // Still detectable as interrupted: status "recording" + .checkpoints intact.
    assert_eq!(
        std::fs::read_to_string(folder.join("metadata.json")).unwrap(),
        original_metadata
    );
    assert!(folder.join(".checkpoints").is_dir());
    assert!(checkpoints.join("audio_chunk_000.mp4").exists());
}

/// specs/0060 review finding: a FRESH (non-resume) start has no journal to replay —
/// `initialize_resume_folder` is the only path that arms one — so `rollback_failed_start`
/// must instead remove the whole folder `initialize_fresh_folder` just created, or a
/// `start_recording` that fails after folder init (e.g. no audio streams available, as
/// hit by the specs/0060 screenshot driver in a sandboxed environment) orphans a
/// meeting-less folder on disk forever. `initialize_fresh_folder` itself resolves its
/// base folder from the process-wide `recordings_root()` cache, which this crate's own
/// tests deliberately never touch (see `recording_preferences::tests`, which build a
/// throwaway `RecordingsRootCache` instead) to keep tests order-independent — so this
/// test reproduces the exact state a fresh init leaves behind (`meeting_folder`/`metadata`
/// set, no resume journal armed) against a temp dir instead of calling
/// `initialize_fresh_folder` through the global.
#[tokio::test]
async fn rollback_failed_start_removes_a_fresh_folder_with_no_journal() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Screenshot take_2026-01-01_00-00");
    std::fs::create_dir_all(folder.join(".checkpoints")).unwrap();
    std::fs::write(
        folder.join("metadata.json"),
        serde_json::to_string_pretty(&sample_metadata(vec![])).unwrap(),
    )
    .unwrap();

    let mut saver = RecordingSaver::new();
    saver.meeting_folder = Some(folder.clone());
    saver.metadata = Some(sample_metadata(vec![]));
    assert!(saver.resume_init_journal.is_none());

    saver.rollback_failed_start().await;

    assert!(
        !folder.exists(),
        "fresh folder must be removed on a failed start"
    );
    assert!(saver.meeting_folder.is_none());
    assert!(saver.metadata.is_none());
}

/// specs/0073 W1: the saver holds its meeting's folder lease from start through
/// finalization, and gives it up when a start is rolled back (or the saver is dropped), so
/// a failed start never leaves the folder locked against every other job.
#[tokio::test]
async fn the_folder_lease_is_released_by_a_rolled_back_start_and_by_drop() {
    use crate::audio::folder_lease::{acquire, current_holder, LeaseHolder};

    let mut saver = RecordingSaver::new();
    saver.set_folder_lease(Some(
        acquire("saver-lease-rollback", LeaseHolder::Recording).await,
    ));
    assert_eq!(
        current_holder("saver-lease-rollback"),
        Some(LeaseHolder::Recording)
    );
    saver.rollback_failed_start().await;
    assert_eq!(current_holder("saver-lease-rollback"), None);

    let mut saver = RecordingSaver::new();
    saver.set_folder_lease(Some(
        acquire("saver-lease-drop", LeaseHolder::Recording).await,
    ));
    drop(saver);
    assert_eq!(current_holder("saver-lease-drop"), None);

    // Finalization (`stop_and_save`) ends the hold even though the saver lives on.
    let app = tauri::test::mock_app();
    let mut saver = RecordingSaver::new();
    saver.set_folder_lease(Some(
        acquire("saver-lease-stop", LeaseHolder::Recording).await,
    ));
    let _ = saver.stop_and_save(app.handle(), None).await;
    assert_eq!(current_holder("saver-lease-stop"), None);
}
