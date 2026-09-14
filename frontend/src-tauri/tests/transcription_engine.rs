//! Transcription engine integration test (specs/0009, priority #1).
//!
//! Feeds a known-speech fixture through the REAL Parakeet engine
//! (`ParakeetEngine::transcribe_audio`) and asserts the transcript recovers the
//! expected words. This is the highest-value guard against
//! "transcribing produces nothing" regressions, because it runs the actual ONNX
//! inference path on real (synthesized) speech.
//!
//! MODEL DEPENDENCY: the Parakeet model lives in the app-data dir and may not be
//! downloaded in every environment. The test SKIPS (with a clear message) when
//! the model is absent and RUNS when it is present. See `tests/common/mod.rs`
//! for how the model is located (and the `NIXON_TEST_MODELS_DIR` override).
//!
//! Run with:
//!   cargo test --features metal --test transcription_engine -- --nocapture

mod common;

use app_lib::parakeet_engine::ParakeetEngine;

#[tokio::test]
async fn parakeet_transcribes_known_speech() {
    let test_name = "parakeet_transcribes_known_speech";

    // 1. Model gate: skip cleanly if the model isn't downloaded.
    let Some(models_dir) = common::find_parakeet_models_dir() else {
        common::skip_no_model(test_name);
        return;
    };

    // 2. Speech fixture (skip if `say` is unavailable, e.g. non-macOS CI).
    let Some(samples) = common::speech_samples_16k() else {
        eprintln!("SKIP {test_name}: `say` unavailable, cannot synthesize speech fixture");
        return;
    };

    // 3. Real engine: load the default model from disk.
    let engine =
        ParakeetEngine::new_with_models_dir(Some(models_dir)).expect("construct ParakeetEngine");

    // discover_models populates internal metadata used by load_model.
    let models = engine.discover_models().await.expect("discover_models");
    assert!(
        models
            .iter()
            .any(|m| m.name == common::DEFAULT_PARAKEET_MODEL),
        "default model not discovered despite files on disk: {:?}",
        models.iter().map(|m| &m.name).collect::<Vec<_>>()
    );

    engine
        .load_model(common::DEFAULT_PARAKEET_MODEL)
        .await
        .expect("load_model");
    assert!(engine.is_model_loaded().await, "model failed to load");

    // 4. Real transcription on real speech samples (16 kHz mono f32).
    let transcript = engine
        .transcribe_audio(samples)
        .await
        .expect("transcribe_audio");

    eprintln!("Parakeet transcript: {transcript:?}");
    assert!(
        !transcript.trim().is_empty(),
        "Parakeet produced an EMPTY transcript for known speech \
         (this is the 'transcribing produces nothing' regression)"
    );
    // Require a majority of expected words to be recovered.
    common::assert_transcript_recovers_words(&transcript, 3);
}
