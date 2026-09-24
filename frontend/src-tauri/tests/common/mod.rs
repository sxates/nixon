//! Shared helpers for the audio/transcription test harness (specs/0009).
//!
//! These tests exercise the REAL recording+transcription code paths *below* the
//! hardware capture layer (Core Audio tap / mic) by feeding fixture audio
//! buffers in at the mix / VAD / transcription interfaces. No microphone,
//! audio-capture permission, or running app is required.
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

/// Returns `(models_dir, model_name)` for a downloaded Whisper ggml model, or `None`.
///
/// The name is the CATALOG key (`config::WHISPER_MODEL_CATALOG`), which is the filename
/// without its `ggml-` prefix and `.bin` suffix — `load_model` takes that, not a path.
///
/// Whisper resolves models from one flat dir (`<app-data>/models/ggml-*.bin`), so unlike
/// Parakeet there is no per-model folder to check for completeness — any `ggml-*.bin` in a
/// candidate root will do. Prefers a smaller model when several are present, since these
/// tests transcribe a couple of seconds of speech and model load dominates.
pub fn find_whisper_model() -> Option<(PathBuf, String)> {
    for root in parakeet_models_root_candidates() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut found: Vec<(u64, String)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                if !name.starts_with("ggml-") || !name.ends_with(".bin") {
                    return None;
                }
                let catalog_name = name
                    .trim_start_matches("ggml-")
                    .trim_end_matches(".bin")
                    .to_string();
                Some((e.metadata().ok()?.len(), catalog_name))
            })
            .collect();
        found.sort_by_key(|(size, _)| *size);
        if let Some((_, name)) = found.into_iter().next() {
            return Some((root, name));
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

// ---------------------------------------------------------------------------
// Room-recording fixtures (specs/0078 W0 task 4)
// ---------------------------------------------------------------------------

/// Which synthetic meeting [`synth_room_meeting`] builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomFixture {
    /// Two voices taking turns on the mic; the system track is silent but for two dings.
    Room,
    /// One voice on the mic (dictation, solo notes); the same dinged system track.
    Solo,
    /// A call: voice A on the mic, voice B on the system track.
    Call,
}

/// Lines per voice. Each renders to roughly 3–5 s, so each voice pools well over
/// `N_AUDIO_MIN_SECS` (10 s) and 25 s across its turns.
const ROOM_VOICE_A_LINES: &[&str] = &[
    "Good morning, thanks for coming in today to go over the plan. I know everyone is busy this week.",
    "The first thing I want to cover is the delivery schedule for next month. It has changed a little.",
    "We moved the design review up by a week, so the team has more time to test. That was the right call.",
    "I also want to talk about the budget, because a few numbers changed. Most of them went in our favor.",
    "Overall I think we are in good shape, but there are two risks to watch. Staffing is the bigger one.",
    "Let's plan to meet again on Thursday and check where things stand. Same room, same time works for me.",
    "Thanks, that covers everything on my list for this morning. I appreciate you making the time.",
];
const ROOM_VOICE_B_LINES: &[&str] = &[
    "Sure, happy to be here, I brought the latest numbers with me. They came in late last night.",
    "That works for us, the vendor confirmed the shipping dates yesterday. Nothing has slipped so far.",
    "Testing will need at least two full weeks, so the extra time really helps. We were worried about it.",
    "The hardware costs went up a little, but the licensing came in lower. So the total is about even.",
    "The main risk on our side is staffing during the holiday period. Two people are out for a week.",
    "Thursday is fine, I will send an updated spreadsheet before then. It will have the new totals.",
    "Great, thank you, I will follow up with the team this afternoon. Have a good rest of the day.",
];

/// Pause between turns (seconds).
const ROOM_TURN_GAP_SECS: f32 = 0.5;
/// Where the two synthetic notification dings sit on the system track (seconds). Both are
/// multiples of the classifier's 600 ms window, so each ding fills exactly one window.
pub const ROOM_DING_AT_SECS: [f32; 2] = [12.0, 60.0];
/// Ding length (seconds): a 1 kHz tone at −20 dBFS.
pub const ROOM_DING_SECS: f32 = 0.3;

/// `say -v voice` → 16 kHz mono f32. `None` when `say` or the voice is unavailable.
pub fn say_voice_16k(text: &str, voice: &str) -> Option<Vec<f32>> {
    let dir = tempfile::tempdir().ok()?;
    let out = dir.path().join("line.wav");
    let ok = std::process::Command::new("say")
        .args(["-v", voice, "-o"])
        .arg(&out)
        .args(["--data-format=LEI16@16000", "--channels=1", "--", text])
        .status()
        .map(|s| s.success() && out.exists())
        .unwrap_or(false);
    ok.then(|| decode_wav_16k_mono(&out).0)
}

/// Write 16 kHz mono f32 samples as a 16-bit PCM WAV (the recorder's channel format).
pub fn write_wav_16k(path: &Path, samples: &[f32]) {
    let data_bytes = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&16_000u32.to_le_bytes());
    bytes.extend_from_slice(&32_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write fixture wav");
}

/// One voice turn on the meeting timeline: `(voice: 0 = A, 1 = B; start sample; samples)`.
type PlacedTurn = (usize, usize, Vec<f32>);

/// Build a synthetic meeting in `dir` and return `(mic.wav, system.wav)`.
///
/// Voice A (*Samantha*) and voice B (*Daniel*) alternate, seven turns each with
/// [`ROOM_TURN_GAP_SECS`] gaps (about 90 s in all). The system track is all zeros except
/// two [`ROOM_DING_SECS`] 1 kHz dings at [`ROOM_DING_AT_SECS`], except in the call variant,
/// where voice B's turns are on it instead of on the mic. The solo variant keeps voice B's
/// slots silent. `None` when `say` or either voice is unavailable (callers skip).
pub fn synth_room_meeting(dir: &Path, variant: RoomFixture) -> Option<(PathBuf, PathBuf)> {
    const SR: usize = 16_000;
    let gap = (ROOM_TURN_GAP_SECS * SR as f32) as usize;
    let mut placed: Vec<PlacedTurn> = Vec::new();
    let mut at = 0usize;
    for (a, b) in ROOM_VOICE_A_LINES.iter().zip(ROOM_VOICE_B_LINES) {
        for (voice, (name, line)) in [("Samantha", a), ("Daniel", b)].into_iter().enumerate() {
            let samples = say_voice_16k(line, name)?;
            let len = samples.len();
            placed.push((voice, at, samples));
            at += len + gap;
        }
    }
    let total = at;
    let mut mic = vec![0.0f32; total];
    let mut system = vec![0.0f32; total];
    for (voice, start, samples) in &placed {
        let target = match (variant, voice) {
            (RoomFixture::Room, _) | (_, 0) => Some(&mut mic),
            (RoomFixture::Call, _) => Some(&mut system),
            (RoomFixture::Solo, _) => None,
        };
        if let Some(track) = target {
            track[*start..*start + samples.len()].copy_from_slice(samples);
        }
    }
    if variant != RoomFixture::Call {
        let amp = 10f32.powf(-20.0 / 20.0);
        for at_secs in ROOM_DING_AT_SECS {
            let start = (at_secs * SR as f32) as usize;
            let len = (ROOM_DING_SECS * SR as f32) as usize;
            for i in 0..len.min(total.saturating_sub(start)) {
                let t = i as f32 / SR as f32;
                system[start + i] = amp * (2.0 * std::f32::consts::PI * 1_000.0 * t).sin();
            }
        }
    }
    let mic_path = dir.join("mic.wav");
    let system_path = dir.join("system.wav");
    write_wav_16k(&mic_path, &mic);
    write_wav_16k(&system_path, &system);
    Some((mic_path, system_path))
}

/// The on-disk diarization model dir (`models/diarization/` under any of the app's data
/// dirs, or `NIXON_TEST_DIARIZATION_DIR`), gated on the SHIPPED model file names
/// (`models::SEGMENTATION_MODEL_FILE` / `EMBEDDING_MODEL_FILE`). `None` → skip.
pub fn diarization_models_dir() -> Option<PathBuf> {
    use app_lib::diarization::models::{EMBEDDING_MODEL_FILE, SEGMENTATION_MODEL_FILE};
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("NIXON_TEST_DIARIZATION_DIR") {
        roots.push(PathBuf::from(dir));
    }
    if let Some(data) = dirs::data_dir() {
        for id in [
            "ai.vinyl.app.debug",
            "ai.vinyl.app",
            "com.meetily.ai",
            "Nixon",
        ] {
            roots.push(data.join(id).join("models").join("diarization"));
        }
    }
    roots.into_iter().find(|dir| {
        dir.join(SEGMENTATION_MODEL_FILE).exists() && dir.join(EMBEDDING_MODEL_FILE).exists()
    })
}
