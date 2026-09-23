//! specs/0072 W1 — the audio lifecycle: the `audio_state` migration, the shared backlog
//! predicate, state transitions, the sweep executor (delete + compress under the folder
//! lease), the dry-run preview's parity with it, and the diarization hook in `launch.rs`.
//!
//! Every folder lives in a tempdir. The compression tests use the bundled ffmpeg under
//! `binaries/` (or one on PATH) and skip when neither exists.
//!
//! Run with: cargo test --features metal --test audio_lifecycle

use std::path::{Path, PathBuf};
use std::process::Command;

use app_lib::audio::folder_lease::{self, LeaseHolder};
use app_lib::audio::lifecycle::compress::{
    compress_channels, compress_channels_with, CompressOptions,
};
use app_lib::audio::lifecycle::policy::{AudioRetention, AudioState};
use app_lib::audio::lifecycle::state::{self, DiarizationGate};
use app_lib::audio::lifecycle::sweep::{
    self, apply_one, purge_media_files, ApplyOutcome, SweepOptions,
};
use app_lib::database::manager::DatabaseManager;
use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::state::AppState;
use chrono::{Duration, Utc};
use sqlx::SqlitePool;
use tauri::{Listener, Manager};

const AFTER: AudioRetention = AudioRetention::AfterProcessing;
const OFF: DiarizationGate = DiarizationGate {
    enabled: false,
    models_present: false,
};
const ON: DiarizationGate = DiarizationGate {
    enabled: true,
    models_present: true,
};

async fn pool_with_schema() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

/// A meeting row. `transcripts` rows are added so the backlog predicate can be steered.
async fn meeting(
    pool: &SqlitePool,
    id: &str,
    age_days: i64,
    folder: Option<&Path>,
    audio_state: Option<&str>,
    transcripts: usize,
) {
    let created = (Utc::now() - Duration::days(age_days) - Duration::minutes(1)).to_rfc3339();
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path, audio_state) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(format!("Lifecycle {id}"))
    .bind(&created)
    .bind(&created)
    .bind(folder.map(|f| f.to_string_lossy().to_string()))
    .bind(audio_state)
    .execute(pool)
    .await
    .unwrap();
    for n in 0..transcripts {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, ?, ?)",
        )
        .bind(format!("{id}-t{n}"))
        .bind(id)
        .bind("words")
        .bind(&created)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn audio_state(pool: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT audio_state FROM meetings WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn set_mode(pool: &SqlitePool, id: &str, mode: Option<&str>) {
    MeetingsRepository::set_processing_mode(pool, id, mode)
        .await
        .unwrap();
}

/// A meeting folder with `metadata.json` (`status`) and the named files (dummy bytes).
fn folder(root: &Path, name: &str, status: &str, files: &[&str]) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("metadata.json"),
        format!(r#"{{"meeting_id":"{name}","status":"{status}"}}"#),
    )
    .unwrap();
    for f in files {
        let path = dir.join(f);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; 1000]).unwrap();
    }
    dir
}

fn ffmpeg() -> Option<PathBuf> {
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    std::fs::read_dir(bundled)
        .ok()
        .and_then(|d| {
            d.flatten().map(|e| e.path()).find(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("ffmpeg-"))
            })
        })
        .or_else(app_lib::audio::ffmpeg::find_ffmpeg_path)
}

/// A 16 kHz mono s16 WAV of a tone, like the channel writer produces.
fn tone_wav(ffmpeg: &Path, path: &Path, secs: f64, freq: u32) {
    let ok = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!(
            "sine=frequency={freq}:duration={secs}:sample_rate=16000"
        ))
        .args(["-ac", "1", "-c:a", "pcm_s16le"])
        .arg(path)
        .status()
        .unwrap()
        .success();
    assert!(ok, "fixture WAV");
}

fn compressing(ffmpeg: &Path) -> SweepOptions {
    SweepOptions {
        compress: true,
        compress_limit: 10,
        recording_active: false,
        ffmpeg: Some(ffmpeg.to_path_buf()),
    }
}

// -- task 5: the migration backfill ---------------------------------------------------------

#[tokio::test]
async fn the_upgrade_backfill_marks_only_finished_meetings_processed() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let full = sqlx::migrate!("./migrations");
    let before = sqlx::migrate::Migrator {
        migrations: full
            .migrations
            .iter()
            .filter(|m| m.version < 20260922000000)
            .cloned()
            .collect::<Vec<_>>()
            .into(),
        ..sqlx::migrate!("./migrations")
    };
    before.run(&pool).await.unwrap();

    let f = Some(Path::new("/r/m"));
    let insert = |id: &'static str,
                  folder: Option<&'static Path>,
                  mode: Option<&'static str>,
                  n: usize| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO meetings (id, title, created_at, updated_at, folder_path, processing_mode) \
                 VALUES (?, 't', '2026-09-01T10:00:00Z', '2026-09-01T11:00:00Z', ?, ?)",
            )
            .bind(id)
            .bind(folder.map(|p| p.to_string_lossy().to_string()))
            .bind(mode)
            .execute(&pool)
            .await
            .unwrap();
            for k in 0..n {
                sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, 'w', 'x')")
                    .bind(format!("{id}-{k}"))
                    .bind(id)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
    };
    insert("transcribed", f, None, 3).await;
    insert("sparse", f, None, 1).await;
    insert("sparse-summarized", f, None, 1).await;
    insert("deferred", f, Some("defer"), 9).await;
    insert("stranded-live", f, Some("live"), 9).await;
    insert("notes-only", None, None, 9).await;
    sqlx::query("INSERT INTO summary_processes (meeting_id, status, created_at, updated_at) VALUES ('sparse-summarized', 'completed', 'x', 'x')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO speakers (id, meeting_id, speaker_key, display_name, created_at, updated_at) VALUES ('s1', 'transcribed', 'spk_0', 'Speaker 1', 'x', 'x')")
        .execute(&pool)
        .await
        .unwrap();

    full.run(&pool).await.unwrap();

    for (id, want) in [
        ("transcribed", Some("processed")),
        ("sparse-summarized", Some("processed")),
        ("sparse", None),
        ("deferred", None),
        ("stranded-live", None),
        ("notes-only", None),
    ] {
        assert_eq!(audio_state(&pool, id).await.as_deref(), want, "{id}");
    }
    let identified: Option<String> =
        sqlx::query_scalar("SELECT speakers_identified_at FROM meetings WHERE id = 'transcribed'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(identified.as_deref(), Some("2026-09-01T11:00:00Z"));
}

// -- task 12: the shared backlog predicate --------------------------------------------------

#[tokio::test]
async fn a_silent_meeting_leaves_the_backlog_once_it_has_been_transcribed() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = folder(tmp.path(), "silent", "completed", &["audio.mp4"]);
    meeting(&pool, "silent", 1, Some(&dir), None, 0).await;
    let listed = |pool: SqlitePool| async move {
        MeetingsRepository::list_deferred_candidates(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(listed(pool.clone()).await, vec!["silent".to_string()]);

    // A reevaluation that doesn't know transcription happened keeps it pending…
    assert_eq!(
        state::reevaluate(&pool, "silent", OFF, false)
            .await
            .unwrap(),
        None
    );
    // …the backlog clearing its marker does, and it leaves the backlog for good.
    assert_eq!(
        state::reevaluate(&pool, "silent", OFF, true).await.unwrap(),
        Some(AudioState::Processed)
    );
    assert!(listed(pool.clone()).await.is_empty());
}

// -- task 10: reevaluate waits for every condition ------------------------------------------

#[tokio::test]
async fn processed_is_written_only_when_every_condition_holds() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();

    // Speaker identification applies (system channel + setting + models) and hasn't run.
    let dir = folder(
        tmp.path(),
        "m",
        "completed",
        &["audio.mp4", "mic.wav", "system.wav"],
    );
    meeting(&pool, "m", 0, Some(&dir), None, 5).await;
    assert_eq!(
        state::reevaluate(&pool, "m", ON, false).await.unwrap(),
        None
    );
    state::mark_speakers_identified(&pool, "m").await.unwrap();
    assert_eq!(
        state::reevaluate(&pool, "m", ON, false).await.unwrap(),
        Some(AudioState::Processed)
    );
    assert_eq!(
        state::reevaluate(&pool, "m", ON, false).await.unwrap(),
        None,
        "idempotent"
    );

    // Diarization off: processed straight away.
    let dir = folder(tmp.path(), "off", "completed", &["audio.mp4", "system.wav"]);
    meeting(&pool, "off", 0, Some(&dir), None, 5).await;
    assert_eq!(
        state::reevaluate(&pool, "off", OFF, false).await.unwrap(),
        Some(AudioState::Processed)
    );

    // No system channel: diarization doesn't apply.
    let dir = folder(tmp.path(), "mix-only", "completed", &["audio.mp4"]);
    meeting(&pool, "mix-only", 0, Some(&dir), None, 5).await;
    assert!(state::reevaluate(&pool, "mix-only", ON, false)
        .await
        .unwrap()
        .is_some());

    // Still recording (or crash-interrupted): never.
    let dir = folder(tmp.path(), "rec", "recording", &["mic.wav"]);
    meeting(&pool, "rec", 0, Some(&dir), None, 5).await;
    assert_eq!(
        state::reevaluate(&pool, "rec", OFF, true).await.unwrap(),
        None
    );

    // Deferred: not even when told transcription happened, while the marker is there.
    let dir = folder(tmp.path(), "defer", "completed", &["audio.mp4"]);
    meeting(&pool, "defer", 0, Some(&dir), None, 5).await;
    set_mode(&pool, "defer", Some("defer")).await;
    assert_eq!(
        state::reevaluate(&pool, "defer", OFF, false).await.unwrap(),
        None
    );
    assert_eq!(
        state::reevaluate(&pool, "defer", OFF, true).await.unwrap(),
        None
    );
    set_mode(&pool, "defer", Some("live")).await;
    assert_eq!(
        state::reevaluate(&pool, "defer", OFF, false).await.unwrap(),
        None
    );
    set_mode(&pool, "defer", None).await;
    assert!(state::reevaluate(&pool, "defer", OFF, true)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn a_failed_identification_is_failed_only_once_transcription_is_done() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = folder(tmp.path(), "done", "completed", &["system.wav"]);
    meeting(&pool, "done", 0, Some(&dir), None, 5).await;
    assert!(state::mark_failed_if_pending(&pool, "done").await.unwrap());
    assert_eq!(audio_state(&pool, "done").await.as_deref(), Some("failed"));

    let dir = folder(tmp.path(), "deferred", "completed", &["system.wav"]);
    meeting(&pool, "deferred", 0, Some(&dir), None, 5).await;
    set_mode(&pool, "deferred", Some("defer")).await;
    assert!(!state::mark_failed_if_pending(&pool, "deferred")
        .await
        .unwrap());
    assert_eq!(
        audio_state(&pool, "deferred").await,
        None,
        "the backlog retries it"
    );

    // A successful retry moves failed → processed.
    state::mark_speakers_identified(&pool, "done")
        .await
        .unwrap();
    assert_eq!(
        state::reevaluate(&pool, "done", ON, false).await.unwrap(),
        Some(AudioState::Processed)
    );
}

#[tokio::test]
async fn a_resumed_recording_makes_the_meeting_pending_again() {
    let pool = pool_with_schema().await;
    meeting(&pool, "r", 0, Some(Path::new("/r")), Some("purged"), 5).await;
    state::mark_speakers_identified(&pool, "r").await.unwrap();
    assert!(state::reset_for_resume(&pool, "r").await.unwrap());
    let row = state::load_row(&pool, "r").await.unwrap().unwrap();
    assert_eq!(row.state(), AudioState::Pending);
    assert!(row.speakers_identified_at.is_none());
    assert!(
        !state::reset_for_resume(&pool, "r").await.unwrap(),
        "already pending"
    );
}

// -- task 8: the sweep executor ---------------------------------------------------------------

/// Sabotage target (spec #2): an interrupted session's `.checkpoints/` is its only audio.
#[tokio::test]
async fn an_interrupted_recordings_audio_is_never_swept() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    // Backfilled `processed` before the crash was noticed: the folder still says recording.
    let dir = folder(
        tmp.path(),
        "crashed",
        "recording",
        &["mic.wav", "system.wav", ".checkpoints/audio_chunk_000.mp4"],
    );
    meeting(&pool, "crashed", 400, Some(&dir), Some("processed"), 9).await;

    let report = sweep::run_sweep(&pool, AFTER, Utc::now(), &SweepOptions::default())
        .await
        .unwrap();
    assert_eq!(report.meetings_purged, 0);
    assert!(dir.join(".checkpoints/audio_chunk_000.mp4").exists());
    assert!(dir.join("mic.wav").exists() && dir.join("system.wav").exists());
    assert_eq!(
        audio_state(&pool, "crashed").await.as_deref(),
        Some("processed")
    );

    // The executor itself never removes checkpoints from a recording folder…
    purge_media_files(&dir);
    assert!(dir.join(".checkpoints/audio_chunk_000.mp4").exists());

    // …and once the folder is finalized, the policy covers it, checkpoints included.
    std::fs::write(
        dir.join("metadata.json"),
        r#"{"meeting_id":"crashed","status":"completed"}"#,
    )
    .unwrap();
    std::fs::write(dir.join("mic.wav"), b"again").unwrap();
    let report = sweep::run_sweep(&pool, AFTER, Utc::now(), &SweepOptions::default())
        .await
        .unwrap();
    assert_eq!(report.meetings_purged, 1);
    assert!(!dir.join(".checkpoints").exists());
    assert!(!dir.join("mic.wav").exists());
    assert!(dir.join("metadata.json").exists());
    assert_eq!(
        audio_state(&pool, "crashed").await.as_deref(),
        Some("purged")
    );
}

/// Q3: an "Immediately" user's leftover channel WAVs go on the first sweep after the
/// upgrade, once processed; the purge is idempotent.
#[tokio::test]
async fn the_upgrade_purge_removes_leftover_channels_once() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = folder(
        tmp.path(),
        "kept",
        "completed",
        &[
            "mic.wav",
            "system.wav",
            ".nixon_decode_1.wav",
            "transcripts.json",
        ],
    );
    meeting(&pool, "kept", 30, Some(&dir), Some("processed"), 9).await;
    let pending = folder(
        tmp.path(),
        "pending",
        "completed",
        &["mic.wav", "system.wav"],
    );
    meeting(&pool, "pending", 30, Some(&pending), None, 9).await;

    let first = sweep::run_sweep(&pool, AFTER, Utc::now(), &SweepOptions::default())
        .await
        .unwrap();
    assert_eq!(first.meetings_purged, 1);
    assert_eq!(first.bytes_freed, 3000);
    assert_eq!(first.purged_ids, vec!["kept".to_string()]);
    for gone in ["mic.wav", "system.wav", ".nixon_decode_1.wav"] {
        assert!(!dir.join(gone).exists(), "{gone}");
    }
    assert!(dir.join("transcripts.json").exists());
    assert!(
        pending.join("mic.wav").exists(),
        "a pending meeting is never deleted"
    );

    let second = sweep::run_sweep(&pool, AFTER, Utc::now(), &SweepOptions::default())
        .await
        .unwrap();
    assert_eq!(second.meetings_purged, 0);
    assert!(second.purged_ids.is_empty());
}

// -- acceptance 5: the preview and the sweep agree --------------------------------------------

/// Sabotage target (spec #4): every rule the sweep applies, the preview applies too.
#[tokio::test]
async fn the_preview_counts_exactly_what_applying_deletes() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let files = &["audio.mp4", "mic.wav", "system.wav"];
    let mut add = Vec::new();
    for (id, age, st, n) in [
        ("processed-old", 40, Some("processed"), 9),
        ("processed-new", 1, Some("processed"), 9),
        ("failed-past-grace", 10, Some("failed"), 9),
        ("failed-in-grace", 2, Some("failed"), 9),
        ("pending", 40, None, 9),
        ("busy", 40, Some("processed"), 9),
        ("purged", 40, Some("purged"), 9),
    ] {
        let dir = folder(tmp.path(), id, "completed", files);
        meeting(&pool, id, age, Some(&dir), st, n).await;
        add.push(dir);
    }
    let deferred = folder(tmp.path(), "deferred", "completed", files);
    meeting(&pool, "deferred", 40, Some(&deferred), Some("processed"), 9).await;
    set_mode(&pool, "deferred", Some("defer")).await;

    let _held = folder_lease::acquire("busy", LeaseHolder::Mover).await;
    let now = Utc::now();
    let preview = sweep::preview(&pool, AFTER, now).await.unwrap();
    assert_eq!(preview.meetings, 4, "{preview:?}");
    assert_eq!(preview.bytes, 4 * 3000);
    assert_eq!(preview.busy, 1);
    assert_eq!(preview.kept_pending, 2, "pending + deferred");
    assert_eq!(preview.kept_failed, 1);

    let report = sweep::run_sweep(&pool, AFTER, now, &SweepOptions::default())
        .await
        .unwrap();
    assert_eq!(report.skipped_busy, 1);
    assert_eq!(
        report.meetings_purged,
        preview.meetings - report.skipped_busy
    );
    assert_eq!(report.bytes_freed, preview.bytes - 3000);
    assert!(
        add[5].join("audio.mp4").exists(),
        "the busy meeting is untouched"
    );

    // Days(30): the same parity on a different policy.
    drop(_held);
    let days = AudioRetention::Days { days: 30 };
    let preview = sweep::preview(&pool, days, now).await.unwrap();
    let report = sweep::run_sweep(&pool, days, now, &SweepOptions::default())
        .await
        .unwrap();
    assert_eq!(
        preview.meetings, 1,
        "only the busy one is left past 30 days"
    );
    assert_eq!(
        report.meetings_purged,
        preview.meetings - report.skipped_busy
    );
}

// -- task 9: the compressor ---------------------------------------------------------------------

#[test]
fn channels_compress_to_opus_and_the_wavs_go() {
    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = folder(tmp.path(), "c", "completed", &["audio.mp4"]);
    tone_wav(&ff, &dir.join("mic.wav"), 3.0, 300);
    tone_wav(&ff, &dir.join("system.wav"), 3.0, 500);
    tone_wav(&ff, &dir.join("system_seg00.wav"), 1.0, 700);

    let out = compress_channels(&ff, &dir).unwrap();
    assert_eq!(out.files, 3);
    assert!(out.bytes_after * 5 < out.bytes_before, "{out:?}");
    for stem in ["mic", "system", "system_seg00"] {
        assert!(
            !dir.join(format!("{stem}.wav")).exists(),
            "{stem}.wav removed"
        );
        assert!(
            dir.join(format!("{stem}.opus")).exists(),
            "{stem}.opus written"
        );
    }
    let (samples, peak) =
        app_lib::audio::lifecycle::compress::decode_stats(&ff, &dir.join("mic.opus")).unwrap();
    assert!(samples.abs_diff(48_000) <= 480, "{samples}");
    assert!(peak > 1000);
    assert!(
        dir.join("audio.mp4").exists(),
        "the mix is never re-encoded"
    );
    assert_eq!(
        compress_channels(&ff, &dir).unwrap().files,
        0,
        "nothing left to do"
    );
}

/// Sabotage target (spec #3): a truncated encode must never replace the WAV.
#[test]
fn a_truncated_encode_leaves_the_wavs_untouched() {
    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = folder(tmp.path(), "t", "completed", &[]);
    tone_wav(&ff, &dir.join("mic.wav"), 3.0, 300);
    tone_wav(&ff, &dir.join("system.wav"), 3.0, 500);
    let before = std::fs::read(dir.join("mic.wav")).unwrap();

    let opts = CompressOptions {
        truncate_encode_secs: Some(1.5),
        ..CompressOptions::default()
    };
    let err = compress_channels_with(&ff, &dir, &opts).unwrap_err();
    assert!(format!("{err:#}").contains("decodes to"), "{err:#}");
    assert_eq!(std::fs::read(dir.join("mic.wav")).unwrap(), before);
    assert!(dir.join("system.wav").exists());
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains("opus"))
        .collect();
    assert!(leftovers.is_empty(), "no .opus or temp left: {leftovers:?}");
}

#[test]
fn both_channels_are_compressed_or_neither() {
    let Some(ff) = ffmpeg() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = folder(tmp.path(), "n", "completed", &[]);
    tone_wav(&ff, &dir.join("mic.wav"), 2.0, 300);
    tone_wav(&ff, &dir.join("system.wav"), 2.0, 500);

    let opts = CompressOptions {
        fail_encode_of: Some("system.wav".into()),
        ..CompressOptions::default()
    };
    assert!(compress_channels_with(&ff, &dir, &opts).is_err());
    assert!(dir.join("mic.wav").exists() && dir.join("system.wav").exists());
    assert!(
        !dir.join("mic.opus").exists(),
        "the first channel's .opus is removed"
    );
    assert!(!dir.join(".mic.opus.tmp").exists());
}

#[tokio::test]
async fn a_kept_meeting_is_compressed_by_the_sweep_but_never_while_recording() {
    let Some(ff) = ffmpeg() else { return };
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = folder(tmp.path(), "keep", "completed", &["audio.mp4"]);
    tone_wav(&ff, &dir.join("mic.wav"), 2.0, 300);
    tone_wav(&ff, &dir.join("system.wav"), 2.0, 500);
    meeting(&pool, "keep", 3, Some(&dir), Some("processed"), 9).await;
    let days = AudioRetention::Days { days: 30 };

    let recording = SweepOptions {
        recording_active: true,
        ..compressing(&ff)
    };
    assert_eq!(
        apply_one(&pool, "keep", days, Utc::now(), &recording)
            .await
            .unwrap(),
        ApplyOutcome::Nothing
    );
    assert!(dir.join("mic.wav").exists());
    // Production default today: compression off (consumers read .opus from W2).
    assert_eq!(
        apply_one(&pool, "keep", days, Utc::now(), &SweepOptions::default())
            .await
            .unwrap(),
        ApplyOutcome::Nothing
    );

    let report = sweep::run_sweep(&pool, days, Utc::now(), &compressing(&ff))
        .await
        .unwrap();
    assert_eq!(report.meetings_compressed, 1);
    assert!(dir.join("mic.opus").exists() && dir.join("system.opus").exists());
    assert!(!dir.join("mic.wav").exists());
    assert!(dir.join("audio.mp4").exists());
    assert_eq!(
        audio_state(&pool, "keep").await.as_deref(),
        Some("processed")
    );
}

/// 0073 task 25: a compressor tick during a move of meeting X skips X and does the rest.
#[tokio::test]
async fn a_compressor_tick_during_a_move_skips_that_meeting_only() {
    let Some(ff) = ffmpeg() else { return };
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut dirs = Vec::new();
    for id in ["moving", "resting"] {
        let dir = folder(tmp.path(), id, "completed", &[]);
        tone_wav(&ff, &dir.join("mic.wav"), 1.0, 300);
        tone_wav(&ff, &dir.join("system.wav"), 1.0, 500);
        meeting(&pool, id, 1, Some(&dir), Some("processed"), 9).await;
        dirs.push(dir);
    }
    let mover = folder_lease::acquire("moving", LeaseHolder::Mover).await;
    let report = sweep::run_sweep(
        &pool,
        AudioRetention::Forever,
        Utc::now(),
        &compressing(&ff),
    )
    .await
    .unwrap();
    assert_eq!(report.meetings_compressed, 1);
    assert!(
        dirs[0].join("mic.wav").exists(),
        "the moving meeting is untouched"
    );
    assert!(!dirs[0].join("mic.opus").exists());
    assert!(dirs[1].join("mic.opus").exists());

    // And the mover never runs under a compressor: the lease excludes it both ways.
    drop(mover);
    let compressor = folder_lease::acquire("moving", LeaseHolder::Compression).await;
    assert!(folder_lease::try_acquire("moving", LeaseHolder::Mover).is_none());
    drop(compressor);
}

// -- task 11: the launch.rs hook (sabotage #9) --------------------------------------------------

/// Point this test process's app-data directory at a temp dir, so nothing reads the real
/// profile: diarization settings default (off), and placeholder model files of the minimum
/// size make `ensure_models` a no-op (no download). Nothing ever loads them — a pass fails
/// at decode first.
fn isolated_app_data() -> &'static Path {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("nixon-lifecycle-appdata-{}", std::process::id()));
        let models = dir.join("models").join("diarization");
        std::fs::create_dir_all(&models).unwrap();
        for (name, size) in [
            ("segmentation.onnx", 4u64 << 20),
            ("nemo_en_titanet_large.onnx", 90u64 << 20),
        ] {
            let f = std::fs::File::create(models.join(name)).unwrap();
            f.set_len(size).unwrap(); // sparse: no real disk use
        }
        app_lib::app_paths::init(dir.clone());
        dir
    })
}

async fn app_with_db() -> (
    tauri::App<tauri::test::MockRuntime>,
    SqlitePool,
    tempfile::TempDir,
) {
    isolated_app_data();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t.sqlite").to_string_lossy().to_string();
    let db_manager = DatabaseManager::new(&db_path, &db_path).await.unwrap();
    let pool = db_manager.pool().clone();
    let app = tauri::test::mock_app();
    app.handle().manage(AppState { db_manager });
    (app, pool, dir)
}

/// A diarization pass that errors (here: an undecodable system channel) must leave the meeting
/// `failed`, which only the hook in `diarization/launch.rs` writes.
#[tokio::test]
async fn a_failed_identification_run_is_recorded_by_the_launch_hook() {
    let (app, pool, db_dir) = app_with_db().await;
    // A system channel that isn't audio: the pass resolves it from the row (never scanning a
    // recordings root) and fails decoding it.
    let dir = folder(
        db_dir.path(),
        "bad-channel",
        "completed",
        &["audio.mp4", "system.wav"],
    );
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) \
         VALUES ('launch-hook', 'lifecycle-launch-hook-fixture', '2001-01-01T00:00:00Z', \
                 '2001-01-01T00:00:00Z', ?)",
    )
    .bind(dir.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .unwrap();
    for n in 0..3 {
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, 'launch-hook', 'w', 'x')")
            .bind(format!("lh-{n}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = events.clone();
    app.handle().listen(
        app_lib::audio::lifecycle::EVENT_AUDIO_STATE_CHANGED,
        move |e| {
            sink.lock().unwrap().push(e.payload().to_string());
        },
    );

    assert!(app_lib::diarization::launch::diarize_meeting(
        app.handle().clone(),
        "launch-hook".into()
    ));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while audio_state(&pool, "launch-hook").await.is_none() && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        audio_state(&pool, "launch-hook").await.as_deref(),
        Some("failed")
    );
    let seen = events.lock().unwrap().clone();
    assert!(
        seen.iter().any(
            |p| p.contains(r#""meetingId":"launch-hook""#) && p.contains(r#""state":"failed""#)
        ),
        "the state change is announced: {seen:?}"
    );
}

#[tokio::test]
async fn a_successful_identification_marks_the_meeting_processed() {
    let (app, pool, db_dir) = app_with_db().await;
    let dir = folder(
        db_dir.path(),
        "ok",
        "completed",
        &["audio.mp4", "system.wav"],
    );
    meeting(&pool, "ok", 0, Some(&dir), None, 5).await;
    app_lib::audio::lifecycle::on_diarization_outcome(app.handle(), "ok", true).await;
    let row = state::load_row(&pool, "ok").await.unwrap().unwrap();
    assert!(row.speakers_identified_at.is_some());
    assert_eq!(row.state(), AudioState::Processed);
}

// -- review fix round 1 ------------------------------------------------------------------------

/// A delete that leaves files behind is not a purge: no `purged` state, no purged id (the
/// meeting page would otherwise say the audio is gone).
#[tokio::test]
async fn a_delete_that_leaves_files_behind_is_not_reported_as_purged() {
    use std::os::unix::fs::PermissionsExt;
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = folder(tmp.path(), "stuck", "completed", &["audio.mp4", "mic.wav"]);
    meeting(&pool, "stuck", 3, Some(&dir), Some("processed"), 9).await;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let outcome = apply_one(&pool, "stuck", AFTER, Utc::now(), &SweepOptions::default()).await;
    let report = sweep::run_sweep(&pool, AFTER, Utc::now(), &SweepOptions::default()).await;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        matches!(outcome.unwrap(), ApplyOutcome::PurgeIncomplete(_)),
        "a folder that kept its files is an incomplete purge"
    );
    let report = report.unwrap();
    assert!(
        report.purged_ids.is_empty(),
        "nothing to announce as purged"
    );
    assert_eq!(report.meetings_purged, 0);
    assert_eq!(
        audio_state(&pool, "stuck").await.as_deref(),
        Some("processed")
    );
    assert!(dir.join("audio.mp4").exists());
}

/// A pending meeting with a finalized folder, a mix only (so speaker identification never
/// applies), and `rows` transcript rows.
async fn pending_mix_only(app_pool: &SqlitePool, root: &Path, id: &str, rows: usize) {
    let dir = folder(root, id, "completed", &["audio.mp4"]);
    meeting(app_pool, id, 0, Some(&dir), None, rows).await;
}

/// Review fix 3: a retranscription that "succeeds" with a sparse result (an engine misfire)
/// is not proof the meeting is transcribed — unless it was the deliberate deferred run.
#[tokio::test]
async fn a_sparse_retranscription_outside_deferred_processing_keeps_the_meeting_pending() {
    let (app, pool, db_dir) = app_with_db().await;
    pending_mix_only(&pool, db_dir.path(), "misfire", 1).await;
    app_lib::audio::lifecycle::on_transcript_replaced(app.handle(), "misfire", 1).await;
    assert_eq!(audio_state(&pool, "misfire").await, None, "audio kept");

    // The same sparse result from the deferred run (the meeting carried `defer`) is final.
    pending_mix_only(&pool, db_dir.path(), "deferred-run", 1).await;
    set_mode(&pool, "deferred-run", Some("defer")).await;
    app_lib::audio::lifecycle::on_transcript_replaced(app.handle(), "deferred-run", 1).await;
    assert_eq!(
        audio_state(&pool, "deferred-run").await.as_deref(),
        Some("processed")
    );
    let mode: Option<String> =
        sqlx::query_scalar("SELECT processing_mode FROM meetings WHERE id = 'deferred-run'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(mode, None, "the pass clears the deferred marker");

    // A full transcript is final whatever surface ran it.
    pending_mix_only(&pool, db_dir.path(), "enhanced", 5).await;
    app_lib::audio::lifecycle::on_transcript_replaced(app.handle(), "enhanced", 5).await;
    assert_eq!(
        audio_state(&pool, "enhanced").await.as_deref(),
        Some("processed")
    );
}

/// Review fix 4: clearing the processing marker is not, by itself, evidence of a transcript.
#[tokio::test]
async fn clearing_the_marker_needs_a_completed_transcript_pass_for_a_sparse_meeting() {
    let (app, pool, db_dir) = app_with_db().await;
    pending_mix_only(&pool, db_dir.path(), "no-pass", 0).await;
    app_lib::audio::lifecycle::reevaluate_after_backlog(app.handle(), "no-pass").await;
    assert_eq!(
        audio_state(&pool, "no-pass").await,
        None,
        "no pass ran: still pending"
    );

    // The backlog's own pass (sparse, no marker) then its final clear: processed.
    pending_mix_only(&pool, db_dir.path(), "silent", 0).await;
    app_lib::audio::lifecycle::on_transcript_replaced(app.handle(), "silent", 0).await;
    assert_eq!(audio_state(&pool, "silent").await, None);
    app_lib::audio::lifecycle::reevaluate_after_backlog(app.handle(), "silent").await;
    assert_eq!(
        audio_state(&pool, "silent").await.as_deref(),
        Some("processed")
    );
}
