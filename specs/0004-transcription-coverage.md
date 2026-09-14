# 0004 — Transcription coverage (missing speech)

- **Status:** VAD over-gating fixed + guarded by tests (2026-06-24). Pending a real-meeting smoke test for final sign-off.
- **Owner agent(s):** audio-engineer
- **Roadmap phase:** Phase 1/3 (quality)

## Context / Problem
First live test (Phase 0, 2026-06-23) on Apple Silicon: recording + live transcription worked,
but the transcript **dropped whole phrases** — gaps/silence where the user was clearly
speaking (a *coverage* failure, not garbled words). Transcriber in use was **Parakeet
0.6b v3 int8** (onboarding default; downloaded to
`~/Library/Application Support/com.vinyl.dev/models/parakeet/`), with meetily's strict Silero
VAD upstream of it.

## Hypotheses (in priority order)
1. **VAD over-gating** — `audio/vad.rs` strict config dropped quieter/short utterances before
   they reached the model. **CONFIRMED — this was the bug.**
2. **Mic level / mixing** — checked; not the primary cause (see notes).
3. **Dropped chunks** — worker/queue backpressure. **Ruled out** — all channels are unbounded.
4. (Lower) model recognition — not investigated; symptom was whole-phrase loss, not garbling.

## Root cause (confirmed)
The Silero state machine (silero-rs) only emits a `SpeechStart` once an utterance has
accumulated `min_speech_time` of frames above the *negative* threshold. If the probability
dips below `negative_speech_threshold` **before** that timer elapses, the whole nascent
utterance is silently discarded — no segment is ever produced. The shipped "strict" config:

| param | old (strict) | Silero default |
|---|---|---|
| `positive_speech_threshold` | 0.50 | 0.50 |
| `negative_speech_threshold` | 0.35 | 0.35 |
| `min_speech_time` | **250 ms** | 90 ms |
| `redemption_time` (live) | 400 ms | 600 ms |
| `pre_speech_pad` / `post_speech_pad` | 300 / 400 ms | 600 / 0 ms |

The 250 ms `min_speech_time` is the dominant cause: a 1-2 word reply (~350-450 ms of audio)
often never clears 250 ms of continuous above-threshold frames (TTS/real speech has internal
dips), so the utterance vanishes. Soft/quiet speech compounds it — lower probabilities dip
below 0.35 more often, resetting the timer.

## Repro (mic-free harness, specs/0009)
`tests/vad_filter.rs` now synthesizes adversarial `say` fixtures and runs them through the
VAD streamed in 512-sample chunks (the way the live pipeline feeds it). Retained-fraction of
voiced audio, **old strict vs new tuned** config:

| fixture | OLD strict | tuned |
|---|---|---|
| normal sentence (full level) | 69% | 87% |
| quiet sentence, peak −35 dBFS | 69% | 74% |
| short one-word reply "okay" (~448 ms) | **0% (dropped)** | 27% |
| "no" / "right" / "got it" (~400 ms each) | **0% (dropped)** | 41-49% |
| quiet + noise, −38 dBFS peak | **0% (dropped)** | 36% |
| quiet + noise, −45 dBFS peak | 0% | 0% (below noise floor — not recoverable, OK) |

So the OLD config dropped **short single-word utterances entirely** and dropped quiet+noisy
speech below ~−38 dBFS entirely — exactly the "whole phrases missing" symptom. (See the
`#[ignore]`d `probe_old_config_drops` test for the raw numbers.)

## Fix — VAD tuning (favour recall without flooding silence)
New named constants in `audio/vad.rs` (top of module), applied via `ContinuousVadProcessor::new`:

| param | new value | rationale |
|---|---|---|
| `POSITIVE_SPEECH_THRESHOLD` | **0.40** | lower onset so softer speech crosses the start threshold |
| `NEGATIVE_SPEECH_THRESHOLD` | **0.25** | brief intra-word dips don't reset a nascent utterance before it clears min-speech |
| `MIN_SPEECH_MS` | **120** | short (1-2 word) utterances survive; still rejects single-frame blips |
| `PRE_SPEECH_PAD_MS` / `POST_SPEECH_PAD_MS` | 300 / 400 | unchanged (STT context) |
| redemption (live) | 400 ms | unchanged; bridges natural pauses |

`new_with_thresholds(...)` was added so tests can compare configs; `new()` delegates to it with
the tuned constants. Silence still yields **zero** segments (the pure-silence test still
passes), so we did not loosen into transcribing noise.

## Instrumentation
Added `debug!`-level VAD decision logging gated behind `RUST_LOG` (no spam at info level):
each completed segment logs duration, sample count, and **RMS/peak in dBFS** (`level_dbfs`
helper). Enable with e.g. `RUST_LOG=app_lib::audio::vad=debug` to watch onset/offset
transitions and segment levels live.

## Other-hypothesis notes
- **Dropped chunks (hyp. 3):** `audio/transcription/worker.rs` uses `tokio::mpsc::unbounded`
  for both the dispatcher and per-worker channels — `send()` never blocks or drops. No
  backpressure drop path exists; ruled out.
- **Mic level / mixing (hyp. 2):** `audio/pipeline.rs` does NOT do RMS ducking — the mixer is
  `ProfessionalAudioMixer`, a plain additive mix with proportional soft-clip (no per-stream
  gain reduction). Mic is EBU R128-normalized to −23 LUFS at capture, so mic gain is unlikely
  to push speech below VAD threshold. The ring-buffer can drop samples on overflow but logs
  loudly when it does; not observed as the cause. **Follow-up (low priority):** the pipeline
  filters VAD segments shorter than 800 samples (50 ms) before sending to STT — fine for now,
  but worth revisiting if very short words still slip through in live tests.

## Tests added (mic-free, `tests/vad_filter.rs`)
- `quiet_speech_is_not_dropped` — −35 dBFS sentence retains > 20%.
- `short_utterance_is_not_dropped` — one-word "okay" retains > 20% (was 0% under old config).
- `paused_speech_keeps_both_phrases` — two quiet phrases + 700 ms gap, both recovered.
- `tuned_config_beats_old_strict_config_on_adversarial_fixtures` — asserts tuned ≥ old on
  every fixture AND that ≥1 fully-dropped (old=0%) case is recovered. This is the regression guard.
- `silence_yields_no_speech_segments` (existing) — still passes; no noise flooding.
- New helpers in `tests/common/mod.rs`: `speech_samples_16k_text`, `scale_to_peak_dbfs`,
  `join_with_pause`, `peak_dbfs`, `rms_dbfs`.

## Acceptance criteria
- [x] Adversarial quiet/short/paused fixtures are NOT dropped by the VAD (guarded by tests).
- [x] Pure silence still yields no segments (no regression into transcribing noise).
- [x] VAD/segment levels observable via `RUST_LOG` debug logging.
- [ ] A scripted read passage transcribes with no whole-phrase dropouts at normal volume in a
      **real live recording** (cannot be validated headlessly — needs a mic + screen-recording
      smoke test the user runs).

## Verification
```
cd frontend/src-tauri && source ~/.cargo/env
cargo test --features metal --test vad_filter -- --nocapture   # all pass; comparison test prints repro
cargo test --features metal --lib vad::                        # existing VAD unit tests still pass
cargo check --features metal && cargo clippy --features metal   # clean (no new warnings)
```
Final sign-off still needs a real meeting: record a fixed passage, diff transcript vs script
for omissions, and inspect `RUST_LOG=app_lib::audio::vad=debug` VAD metrics.
