//! Shared helpers for the audio/transcription test harness (specs/0009).
//!
//! These tests exercise the REAL recording+transcription code paths *below* the
//! hardware capture layer (Core Audio tap / mic) by feeding fixture audio
//! buffers in at the mix / VAD / transcription interfaces. No microphone,
//! screen-recording permission, or running app is required.
//!
//! Fixture audio is generated at test time with macOS `say` (deterministic,
//! license-clean) and decoded with the app's own `decode_audio_file`. Synthetic
//! silence/tone buffers are generated in-process.

#![allow(dead_code)] // helpers are shared across several test binaries; not all use all of them

use std::path::{Path, PathBuf};
#[cfg(not(debug_assertions))]
use std::process::Command;

use app_lib::audio::pipeline::TranscriptionChunk;
use app_lib::audio::recording_state::{AudioChunk, DeviceType};

/// The sentence we synthesize for speech fixtures. Lowercase, no punctuation that
/// the STT engine would not produce, so substring assertions are stable.
pub const FIXTURE_SENTENCE: &str = "the quick brown fox jumps over the lazy dog";

/// Words we expect any reasonable STT engine to recover from `FIXTURE_SENTENCE`.
/// We assert on a subset (not the full sentence) to stay robust to minor
/// engine/casing/spacing differences.
pub const EXPECTED_WORDS: &[&str] = &["quick", "brown", "fox", "lazy", "dog"];

/// Default Parakeet model the app ships with (mirrors `config::DEFAULT_PARAKEET_MODEL`).
pub const DEFAULT_PARAKEET_MODEL: &str = "parakeet-tdt-0.6b-v3-int8";

// ---------------------------------------------------------------------------
// Database fixtures (db_lifecycle / diarization_persistence / fts_search)
// ---------------------------------------------------------------------------

/// Bring up a fresh, migrated SQLite database in a temp dir, through the app's
/// REAL migration path (`DatabaseManager::new` runs every file in
/// `migrations/`). No Tauri runtime required. Keep the `TempDir` alive for the
/// duration of the test — dropping it deletes the database file.
pub async fn fresh_db() -> (
    tempfile::TempDir,
    app_lib::database::manager::DatabaseManager,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("meeting_minutes.sqlite");
    let backend_path = dir.path().join("does-not-exist.db"); // legacy DB absent -> fresh create
    let manager = app_lib::database::manager::DatabaseManager::new(
        db_path.to_str().unwrap(),
        backend_path.to_str().unwrap(),
    )
    .await
    .expect("DatabaseManager::new ran migrations");
    (dir, manager)
}

/// Minimal transcript-segment fixture. The repository generates the real id.
/// The timestamp is derived from `start` so `ORDER BY audio_start_time, timestamp`
/// tiebreakers sort the same way as the audio timeline.
pub fn segment(text: &str, start: f64, end: f64) -> app_lib::transcripts::TranscriptSegment {
    app_lib::transcripts::TranscriptSegment {
        id: String::new(), // repository generates the real id
        text: text.to_string(),
        timestamp: format!("2026-06-24T00:00:{:02}Z", start as i64 % 60),
        audio_start_time: Some(start),
        audio_end_time: Some(end),
        duration: Some(end - start),
        speaker: None,
        channel: None,
        word_timestamps: None,
    }
}

// ---------------------------------------------------------------------------
// Speech fixtures (macOS `say`)
// ---------------------------------------------------------------------------

/// Generate a 16 kHz mono WAV of `text` using macOS `say` into `out`.
/// Returns `false` (with a logged reason) when `say` is unavailable or fails, so
/// callers can skip rather than hard-fail on non-macOS / CI machines.
#[cfg(debug_assertions)]
pub fn synth_say_wav(text: &str, out: &Path) -> bool {
    app_lib::dev_fixtures::wav::say_to_wav(text, None, out)
}

#[cfg(not(debug_assertions))]
pub fn synth_say_wav(text: &str, out: &Path) -> bool {
    let status = Command::new("say")
        .arg("-o")
        .arg(out)
        // 16-bit little-endian PCM, 16 kHz, mono -> exactly what the STT/VAD want.
        .arg("--data-format=LEI16@16000")
        .arg("--channels=1")
        .arg(text)
        .status();

    match status {
        Ok(s) if s.success() && out.exists() => true,
        Ok(s) => {
            eprintln!("`say` exited unsuccessfully ({s}); skipping speech-fixture test");
            false
        }
        Err(e) => {
            eprintln!("`say` not available ({e}); skipping speech-fixture test");
            false
        }
    }
}

/// Decode a fixture WAV into mono 16 kHz f32 samples using the app's own decoder.
/// Returns `(samples, sample_rate)`. The `say` fixtures are already 16 kHz mono,
/// so no resampling/downmixing is needed here.
pub fn decode_wav_16k_mono(path: &Path) -> (Vec<f32>, u32) {
    let decoded =
        app_lib::audio::decoder::decode_audio_file(path).expect("decode fixture wav failed");
    assert_eq!(
        decoded.channels, 1,
        "fixture should be mono (got {} channels)",
        decoded.channels
    );
    (decoded.samples, decoded.sample_rate)
}

/// Convenience: synthesize `FIXTURE_SENTENCE` and return its 16 kHz mono samples.
/// Returns `None` when `say` is unavailable (test should skip).
pub fn speech_samples_16k() -> Option<Vec<f32>> {
    speech_samples_16k_text(FIXTURE_SENTENCE)
}

/// Synthesize an arbitrary `text` with `say` and return its 16 kHz mono samples.
/// Returns `None` when `say` is unavailable (test should skip).
pub fn speech_samples_16k_text(text: &str) -> Option<Vec<f32>> {
    let dir = tempfile::tempdir().expect("tempdir");
    let wav = dir.path().join("speech.wav");
    if !synth_say_wav(text, &wav) {
        return None;
    }
    let (samples, rate) = decode_wav_16k_mono(&wav);
    assert_eq!(rate, 16_000, "say fixture should decode at 16 kHz");
    assert!(!samples.is_empty(), "say fixture decoded to zero samples");
    Some(samples)
}

// ---------------------------------------------------------------------------
// Level helpers (for the specs/0004 over-gating fixtures)
// ---------------------------------------------------------------------------

/// Peak level of `samples` in dBFS (0 dBFS = full scale). Returns -inf for silence.
pub fn peak_dbfs(samples: &[f32]) -> f32 {
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * peak.log10()
    }
}

/// RMS level of `samples` in dBFS. Returns -inf for silence.
pub fn rms_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return f32::NEG_INFINITY;
    }
    let mean_sq = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    let rms = mean_sq.sqrt();
    if rms <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * rms.log10()
    }
}

/// Scale `samples` so their PEAK sits at `target_peak_dbfs` (e.g. -35.0 for quiet
/// speech). Used to mimic a soft talker / low mic gain — the specs/0004 symptom.
pub fn scale_to_peak_dbfs(samples: &[f32], target_peak_dbfs: f32) -> Vec<f32> {
    let current_peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if current_peak <= 0.0 {
        return samples.to_vec();
    }
    let target_linear = 10f32.powf(target_peak_dbfs / 20.0);
    let gain = target_linear / current_peak;
    samples.iter().map(|s| s * gain).collect()
}

/// Concatenate two speech clips with `pause_ms` of silence between them, at
/// `sample_rate`. Used to exercise redemption/min-speech with a mid-utterance gap.
pub fn join_with_pause(a: &[f32], b: &[f32], pause_ms: u32, sample_rate: u32) -> Vec<f32> {
    let gap = (sample_rate as f32 * pause_ms as f32 / 1000.0) as usize;
    let mut out = Vec::with_capacity(a.len() + gap + b.len());
    out.extend_from_slice(a);
    out.extend(std::iter::repeat_n(0.0f32, gap));
    out.extend_from_slice(b);
    out
}

// ---------------------------------------------------------------------------
// Synthetic buffers (no external tools)
// ---------------------------------------------------------------------------

/// `seconds` of pure digital silence at `sample_rate`.
pub fn silence(seconds: f32, sample_rate: u32) -> Vec<f32> {
    vec![0.0f32; (seconds * sample_rate as f32) as usize]
}

/// Resample mono f32 from `from_rate` to `to_rate` via linear interpolation.
/// Used to turn a 16 kHz speech fixture into 48 kHz "capture" audio for the
/// pipeline test (the capture layer delivers 48 kHz).
pub fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = samples.get(idx).copied().unwrap_or(0.0);
        let b = samples.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

// ---------------------------------------------------------------------------
// Parakeet model presence (skip-gate)
// ---------------------------------------------------------------------------

/// Candidate app-data roots that may contain a `models/parakeet/<model>` tree.
/// We check the dev, production, and upstream-meetily identifiers so the test
/// runs whenever a model is downloaded under any of them.
fn parakeet_models_root_candidates() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    // Explicit override for CI / custom setups.
    if let Ok(dir) = std::env::var("NIXON_TEST_MODELS_DIR") {
        roots.push(PathBuf::from(dir));
    }

    if let Some(data) = dirs::data_dir() {
        for id in [
            "ai.vinyl.app.debug",
            "ai.vinyl.app",
            "com.vinyl.dev.debug",
            "com.vinyl.dev",
            "com.meetily.ai",
            "Meetily",
        ] {
            roots.push(data.join(id).join("models"));
        }
    }

    // Dev fallback used by ParakeetEngine when no dir is supplied in debug builds.
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("models"));
    }

    roots
}

/// Files Parakeet needs for an int8 model to be considered "downloaded".
const PARAKEET_INT8_FILES: &[&str] = &[
    "encoder-model.int8.onnx",
    "decoder_joint-model.int8.onnx",
    "nemo128.onnx",
    "vocab.txt",
];

/// Returns the `models` directory (the one you pass to
/// `ParakeetEngine::new_with_models_dir(Some(..))`, which appends `parakeet/`)
/// that contains a fully-downloaded `DEFAULT_PARAKEET_MODEL`, or `None`.
pub fn find_parakeet_models_dir() -> Option<PathBuf> {
    for root in parakeet_models_root_candidates() {
        let model_dir = root.join("parakeet").join(DEFAULT_PARAKEET_MODEL);
        let complete = PARAKEET_INT8_FILES
            .iter()
            .all(|f| model_dir.join(f).exists());
        if complete {
            return Some(root);
        }
    }
    None
}

/// Print a uniform skip message and return. Use at the top of a model-gated test.
pub fn skip_no_model(test_name: &str) {
    eprintln!(
        "SKIP {test_name}: Parakeet model '{DEFAULT_PARAKEET_MODEL}' not found. \
         Download it in the app (Settings -> Transcription) or set \
         NIXON_TEST_MODELS_DIR to a dir containing models/parakeet/{DEFAULT_PARAKEET_MODEL}/. \
         This test is a runtime skip, not a failure."
    );
}

/// Assert that `transcript` (lowercased) contains at least `min_hits` of
/// `EXPECTED_WORDS`. STT is not byte-exact, so we require a majority rather than
/// the full sentence.
pub fn assert_transcript_recovers_words(transcript: &str, min_hits: usize) {
    let lower = transcript.to_lowercase();
    let hits: Vec<&str> = EXPECTED_WORDS
        .iter()
        .copied()
        .filter(|w| lower.contains(w))
        .collect();
    assert!(
        hits.len() >= min_hits,
        "transcript did not recover enough expected words (got {:?} of {:?}, need >= {}). \
         Full transcript: {:?}",
        hits,
        EXPECTED_WORDS,
        min_hits,
        transcript
    );
}

// ---------------------------------------------------------------------------
// Pipeline streaming helpers (pipeline_integration: mid-recording toggle tests)
// ---------------------------------------------------------------------------

/// Stream `samples` into a running pipeline as ~20ms microphone chunks at `capture_rate`.
pub async fn feed(
    audio_tx: &tokio::sync::mpsc::Sender<AudioChunk>,
    samples: &[f32],
    chunk_len: usize,
    capture_rate: u32,
) {
    feed_device(
        audio_tx,
        samples,
        chunk_len,
        capture_rate,
        DeviceType::Microphone,
    )
    .await;
}

/// As `feed`, but on a chosen capture device — so a test can drive DISTINCT mic and
/// system content and assert the channel attribution that rides the transcription
/// chunks (specs/0029 WS3.4, spec 0051 WS1 across the live/deferred toggle).
pub async fn feed_device(
    audio_tx: &tokio::sync::mpsc::Sender<AudioChunk>,
    samples: &[f32],
    chunk_len: usize,
    capture_rate: u32,
    device_type: DeviceType,
) {
    for (chunk_id, chunk) in samples.chunks(chunk_len).enumerate() {
        audio_tx
            .send(AudioChunk {
                data: chunk.to_vec(),
                sample_rate: capture_rate,
                timestamp: chunk_id as f64 * chunk_len as f64 / capture_rate as f64,
                chunk_id: chunk_id as u64,
                device_type: device_type.clone(),
            })
            .await
            .expect("send chunk into pipeline");
    }
}

/// Drain every chunk the pipeline currently produces, returning once it has been quiet
/// for ~1s (the pipeline runs faster than real time, but asynchronously).
pub async fn drain_settled(
    rx: &mut tokio::sync::mpsc::Receiver<TranscriptionChunk>,
) -> Vec<TranscriptionChunk> {
    let mut out = Vec::new();
    let mut idle = 0;
    while idle < 20 {
        match rx.try_recv() {
            Ok(chunk) => {
                out.push(chunk);
                idle = 0;
            }
            Err(_) => {
                idle += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
    out
}
