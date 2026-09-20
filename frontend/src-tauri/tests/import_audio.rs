//! Import-audio integration test (specs/0066 W3).
//!
//! Import has shipped behind a "beta" flag since the fork, on trust rather than evidence:
//! the only coverage was three unit tests over its pure helpers. Before graduating it to an
//! ordinary feature, this runs a real file through the whole path — validate, copy, decode,
//! resample, segment, transcribe, persist — and asserts a meeting with transcript segments
//! comes out the far end.
//!
//! MODEL DEPENDENCY: transcription needs a downloaded Whisper model. The test SKIPS (with a
//! clear message) when none is present, exactly like `transcription_engine`. So CI proves
//! nothing here; a developer machine with a model does.
//!
//! Run with:
//!   cargo test --features metal --test import_audio -- --nocapture

mod common;

use app_lib::audio::import::{start_import, validate_audio_file};
use app_lib::database::manager::DatabaseManager;
use app_lib::state::AppState;
use tauri::Manager;

/// The fixture sentence is long enough to produce speech, short enough that model load
/// dominates the runtime.
const TITLE: &str = "Imported staff meeting";

#[tokio::test]
async fn importing_a_wav_produces_a_meeting_with_transcript_segments() {
    let test_name = "importing_a_wav_produces_a_meeting_with_transcript_segments";

    // 1. Model gate.
    let Some((models_dir, model_name)) = common::find_whisper_model() else {
        eprintln!(
            "SKIP {test_name}: no Whisper ggml model found. Download one in the app \
             (Settings → Transcription) or set NIXON_TEST_MODELS_DIR to a dir containing \
             ggml-*.bin. This is a runtime skip, not a failure."
        );
        return;
    };
    eprintln!(
        "using whisper model {model_name} from {}",
        models_dir.display()
    );

    // 2. Source audio (skip if `say` is unavailable, e.g. non-macOS CI).
    let scratch = tempfile::tempdir().expect("tempdir");
    let source = scratch.path().join("meeting.wav");
    if !common::synth_say_wav(common::FIXTURE_SENTENCE, &source) {
        eprintln!("SKIP {test_name}: `say` unavailable, cannot synthesize an audio fixture");
        return;
    }

    // 3. Validation is the gate the picker uses before anything is copied.
    let info = validate_audio_file(&source).expect("a say-produced wav must validate");
    assert!(
        info.duration_seconds > 0.5,
        "the fixture should be at least half a second, got {:.2}s",
        info.duration_seconds
    );

    // 4. A real migrated DB and a recordings root of our own, so the import writes
    //    somewhere disposable instead of the developer's ~/Movies.
    let (db_dir, db_manager) = fresh_manager().await;
    let recordings = tempfile::tempdir().expect("tempdir");
    app_lib::audio::recording_preferences::set_recordings_root(recordings.path().to_path_buf());
    app_lib::whisper_engine::commands::set_models_directory_path(models_dir);

    let app = tauri::test::mock_app();
    app.handle().manage(AppState { db_manager });

    // 5. The import itself.
    let result = start_import(
        app.handle().clone(),
        source.to_string_lossy().to_string(),
        TITLE.to_string(),
        Some("en".to_string()),
        Some(model_name),
        None,
    )
    .await
    .expect("import must succeed with a model present and a valid wav");

    assert_eq!(result.title, TITLE);
    assert!(
        result.segments_count > 0,
        "the import reported no transcript segments"
    );

    // 6. What the user actually gets: a meeting row, its transcript, and the copied audio.
    let pool = app.state::<AppState>().db_manager.pool().clone();
    let transcript =
        app_lib::database::repositories::transcript::TranscriptsRepository::get_full_transcript(
            &pool,
            &result.meeting_id,
        )
        .await
        .expect("read back the transcript");
    assert!(
        !transcript.trim().is_empty(),
        "a meeting was created but its transcript is empty"
    );
    common::assert_transcript_recovers_words(&transcript, 2);

    let folder = std::fs::read_dir(recordings.path())
        .expect("recordings root")
        .flatten()
        .next()
        .expect("the import must create a meeting folder");
    assert!(
        folder.path().join("audio.wav").exists(),
        "the source audio should be copied into the meeting folder"
    );

    drop(db_dir);
}

/// `common::fresh_db` hands back the TempDir separately; this keeps the two together so the
/// database outlives the test body.
async fn fresh_manager() -> (tempfile::TempDir, DatabaseManager) {
    common::fresh_db().await
}
