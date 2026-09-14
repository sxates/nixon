# 0009 — Audio + transcription test harness (no microphone required)

- **Status:** Done 2026-06-24
- **Owner agent(s):** audio-engineer
- **Roadmap phase:** Phase 1 (foundations / tech-debt hardening)

## Context / Problem

The recording + transcription pipeline has a recurring class of failures that are
**hard to catch by hand** and currently only surface when someone manually records a
meeting:

- silent recordings (mic/system capture returns silence),
- VAD over-gating that drops real speech (`specs/0004`),
- "transcribing produces nothing" (engine path broken / empty transcript),
- regressions in the persist-at-recording-START lifecycle (`specs/0007`).

The only truly hardware-dependent part is the **Core Audio process tap / mic**
(`audio/capture/core_audio.rs`, `capture/system.rs`). Everything below it — mixing,
resampling, VAD, the transcription worker, and the STT engines — operates on audio
**samples/buffers**. So we can feed fixture audio in at those layers and exercise the
real code paths without a microphone, screen-recording permission, or a running app.

## Goals

- Automated `cargo test` coverage of the recording/transcription pipeline **below** the
  hardware capture layer, with no mic / no permissions / no running app.
- Directly guard the four failure modes above.
- Keep the core suite fast and CI-friendly; gate the model-dependent test so it **skips
  with a clear message** when the Parakeet model isn't downloaded and **runs** when it is.

## Non-goals

- Testing the Core Audio tap / cpal capture itself (the genuinely hardware-dependent
  layer). That still requires a manual mic + screen-recording smoke test.
- Testing Whisper (we test the **default** engine, Parakeet). The same fixture pattern
  extends to Whisper later via `TranscriptionProvider`.
- Testing the Tauri command layer end-to-end (needs an `AppHandle`); we test the
  repository/engine/pipeline logic those commands wrap.

## Approach

Add **integration tests** under `frontend/src-tauri/tests/` that depend on the crate's
public library (`app_lib`, the `rlib` target). Integration tests live outside the binary,
exercise only the public API, and don't bloat the app. Fixture audio is generated at test
time with macOS `say` (deterministic, license-clean) and decoded with the app's own
`decode_audio_file`; synthetic silence is generated in-process.

### Injection points (verified)

| Layer | Injection point | Notes |
|---|---|---|
| STT engine | `ParakeetEngine::new_with_models_dir(Some(dir))` → `discover_models()` → `load_model()` → `transcribe_audio(Vec<f32>) -> Result<String>` | Takes 16 kHz mono f32; the real ONNX path. |
| VAD | `audio::vad::get_speech_chunks(&[f32], redemption_ms)` and `ContinuousVadProcessor::{new, process_audio, flush}` | 16 kHz mono in; emits `SpeechSegment`s. |
| Pipeline | `AudioPipeline::new(receiver, transcription_sender, RecordingState::new(), …)` then `run()` | We own both channel ends: feed `AudioChunk`s to the receiver, observe VAD-filtered 16 kHz segments on the sender. `run()` does **not** gate on `state.is_recording()`, so closing the input channel drains + flushes cleanly. The mixer ring buffer zero-pads the missing stream, so feeding only mic chunks works. |
| DB lifecycle | `DatabaseManager::new(temp_db_path, absent_backend_path)` (runs the real migrations), then `MeetingsRepository` / `TranscriptsRepository` | No `AppHandle` needed; temp-file SQLite. |

## Design

### Files added

```
frontend/src-tauri/tests/
  common/mod.rs            # shared fixture + skip-gate helpers
  transcription_engine.rs  # Parakeet integration (model-gated)
  vad_filter.rs            # silence vs speech VAD
  pipeline_integration.rs  # push AudioChunks through the real AudioPipeline
  db_lifecycle.rs          # persist-at-start DB lifecycle
  fixtures/audio/          # (reserved for committed fixtures; none needed today)
```

One pre-existing source nit was fixed so test-aware clippy is clean:
`audio/system_audio_commands.rs` had `assert!(device_list.len() >= 0)`
(always-true comparison) in an existing `#[cfg(test)]` block.

### What each test does

1. **`transcription_engine::parakeet_transcribes_known_speech`** (priority #1) —
   `say` → 16 kHz WAV → decode → `ParakeetEngine::transcribe_audio` → assert the transcript
   recovers ≥3 of the expected words. Guards "transcribing produces nothing".
   **Model-gated** (see below).
2. **`vad_filter`** (priority #2) — `silence_yields_no_speech_segments` (pure zeros → no
   segments), `speech_fixture_is_detected` (real speech → ≥1 segment retaining >20% of the
   clip; guards `specs/0004` over-gating), `speech_detected_when_streamed_in_small_chunks`
   (state survives 512-sample chunk boundaries, like live capture). No model needed.
3. **`pipeline_integration`** (priority #3, "recording works, minus the mic") —
   `pipeline_emits_vad_segments_for_speech` upsamples the 16 kHz fixture to 48 kHz, pushes
   it as mic `AudioChunk`s through the real `AudioPipeline`, and asserts VAD-filtered
   16 kHz segments reach the transcription stage; when the model is present it also
   transcribes them and asserts non-empty text. `pipeline_emits_nothing_for_silence`
   asserts silence is fully gated out.
4. **`db_lifecycle`** (priority #4) — `persist_at_start_lifecycle`:
   `create_meeting` (START) → `save_transcripts_for_meeting` (STOP) →
   `get_meetings_enriched` → assert ONE meeting with transcripts attached, correct
   `duration_seconds` (= `MAX(audio_end_time)`) and gist (`first_transcript`) →
   `delete_meeting` cleans it up (no orphan transcripts). `save_to_missing_meeting_returns_false`
   guards the fallback contract.

### Model dependency (skip-gate)

`tests/common::find_parakeet_models_dir()` looks for a fully-downloaded
`parakeet-tdt-0.6b-v3-int8` (all int8 ONNX files + `vocab.txt`) under, in order:
`$VINYL_TEST_MODELS_DIR`, then `~/Library/Application Support/{com.vinyl.dev.debug,
com.vinyl.dev, com.meetily.ai, Meetily}/models`, then `./models` (the dev fallback). If
none is found, the model-dependent assertions **print a SKIP message and return** (not a
failure). `say` unavailability is handled the same way. This dev environment has the model,
so the test RUNS here.

## Acceptance criteria

- `cargo test --features metal` passes with no mic / permissions / running app.
- Model-dependent test runs when the model is present, skips cleanly otherwise.
- `cargo check` and `cargo clippy --features metal --tests` clean (DoD, `/CLAUDE.md`).

## Risks / open questions

- `say` is macOS-only; on a non-mac CI the speech fixtures skip (silence/DB tests still
  run). If we add Linux CI, commit a small WAV under `tests/fixtures/audio/` and have the
  helper fall back to it.
- STT is not byte-exact; assertions require a majority of expected words, not the full
  sentence, to avoid flakiness.

## Verification

```bash
cd frontend/src-tauri && source ~/.cargo/env
# fast, model-free:
cargo test --features metal --test vad_filter --test db_lifecycle
# full harness (model-dependent ones run if the Parakeet model is downloaded):
cargo test --features metal \
  --test transcription_engine --test vad_filter \
  --test pipeline_integration --test db_lifecycle -- --nocapture
```

Observed on the dev machine (model present): all 8 tests pass; Parakeet returned
`"The quick brown fox jumps over the lazy dog."` and the pipeline-minus-mic path returned
`"Fox jumps over the lazy dog."`.

## Still NOT covered (the hardware layer)

The Core Audio process tap and mic/cpal capture (`audio/capture/`) are untested here — they
need real devices + screen-recording permission. Continue to smoke-test those manually
(record → live transcript → summary) per the Definition of Done.
