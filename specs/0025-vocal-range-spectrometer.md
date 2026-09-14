# 0025 — Vocal-Range Spectrometer Visualization

- **Status:** Implemented on `fix/0024-1.1-feedback` (pending GUI verification). Backend FFT band
  extraction + `recording-spectrum` event and the frontend hook rework landed with tests; all
  three surfaces become spectrometers via the shared hook. See **Implementation notes** below.
- **Owner agent(s):** audio-engineer + frontend-engineer
- **Roadmap phase:** Post-1.1 hardening (graduated from `specs/0024` WS7.1)

## Context / Problem

Graduated from `specs/0024` (1.1 feedback note 4). The recording-surface waveform currently reads
as a timeline of audio "bumps" scrolling right-to-left. The user wants a **spectrometer**: bars
representing low→high frequency that move up/down with the audio, scoped to the vocal speech range
rather than the full spectrum.

This is broken out because it adds a **new backend audio path** (per-chunk FFT magnitude
extraction + a new Tauri event) on the hot recording loop and reworks the shared frontend
waveform hook — larger and more independent than the rest of the 1.1 batch.

## Goals

- Show vocal-range frequency bars on all three recording surfaces (record header, the floating
  `RecordingControls` pill, and `GlobalRecordingBar`) that move up/down with audio and ease to
  flat on silence.
- Scope the displayed bands to the vocal range (proposed ~80 Hz–4 kHz, log-spaced) so the bars
  track speech, not the whole spectrum.

## Non-goals

- A full spectrogram / waterfall display. Just instantaneous per-band magnitude bars.
- Browser-side audio analysis. Audio is captured in Rust (system tap), so `AudioContext`/
  `AnalyserNode` cannot see it — the FFT must happen backend-side.
- Touching the capture/mix/VAD pipeline behavior (kept per `/CLAUDE.md`); we only *read* the
  already-mixed chunk.

## Approach

Reuse the existing `realfft` planner pattern already in the codebase for noise reduction
(`audio/audio_processing.rs:405` `spectral_subtraction`) to compute an FFT magnitude spectrum of
each mixed chunk, bucket the magnitudes into a small number of log-spaced bands across the vocal
range, and emit them on a **new throttled event** (`recording-spectrum { bands: number[] }`)
alongside the existing `recording-level`. On the frontend, replace the scrolling RMS buffer in the
shared `useRecordingWaveform` hook with a band-indexed bar model (each bar = one band, height =
its magnitude, eased toward 0 on silence), so all three surfaces become spectrometers via the one
hook.

*Why backend FFT over a second analysis path:* the mixed PCM only exists in Rust; emitting bands
from where `recording-level` is already computed keeps one throttle and one source of truth.

## Design

### Backend (Rust)

- **FFT band extraction** near `frontend/src-tauri/src/audio/transcription/worker.rs:341-358`
  (where `recording-level` `{rms, peak}` is already computed per mixed chunk). Compute a windowed
  real FFT of the chunk, take magnitudes, sum into `N` log-spaced bands over [~80 Hz, ~4 kHz],
  normalize to 0..1. Reuse the `RealFftPlanner` import/pattern from
  `audio/audio_processing.rs:4-5,405`.
- **Throttle** identically to `recording-level` (the existing `LEVEL_EMIT_INTERVAL` 80 ms,
  `worker.rs:341`) so the new path adds no meaningful cost to the capture loop.
- **New event** `recording-spectrum` with payload `{ bands: Vec<f32> }` (length `N`). Register/emit
  alongside the existing level emit.

### Frontend

- **Rework `frontend/src/hooks/useRecordingWaveform.ts:20-46`**: subscribe to `recording-spectrum`,
  hold a `number[]` of band magnitudes (not a scrolling history), decay toward 0 on each tick when
  no new frame arrives so silence flattens. Keep the active/paused gating.
- **Renderers** (each maps the band array to bars; no scrolling):
  - `frontend/src/app/record/page.tsx:54-65` (HeaderWaveform) and `:294-298` (pill bars input).
  - `frontend/src/components/RecordingControls.tsx:470-482`.
  - `frontend/src/components/GlobalRecordingBar.tsx:213-223` (drop the unused legacy `grb-wave`
    keyframe at `:117`).

## Tasks

1. [x] (audio) `spectrum_bands(samples, sample_rate)` in `audio/audio_processing.rs` — 2048-sample
   Hann-windowed real FFT, **32** log-spaced bands over **80 Hz–4 kHz**, peak-per-band, window-size
   normalization + `GAIN` + sqrt compression. Emitted as `recording-spectrum { bands }` from
   `transcription/worker.rs` under the existing 80 ms throttle, alongside `recording-level`.
2. [x] (frontend) `useRecordingWaveform` reworked to subscribe to `recording-spectrum`, resample N
   bands → the surface's bar count (`resampleBands`), and ease/decay (rise fast, fall slower;
   staleness decay so pause/stop settles flat).
3. [x] (frontend) The three renderers are unchanged structurally (they already map `levels[i]` →
   bar height) so they became spectrometers via the shared hook; comments updated; the dead
   `grb-wave` keyframe removed.
4. [x] Tests — Rust: a 1 kHz tone peaks in the band that contains it; silence/empty are all-zero.
   Frontend: `resampleBands` (identity / averaging / low→high order) + hook easing & decay.

## Implementation notes

- **Why 32 bands:** an upper bound on any surface's bar count (header 28, global bar 14, pill 3),
  so the hook always **downsamples** (averages slices) and never interpolates up. The renderers
  keep their existing bar counts.
- **Renderers needed almost no change:** they already rendered `levels[i]` as `scaleY` bars, so
  swapping the hook's data source from a scrolling RMS buffer to frequency bands turns them into
  spectrometers (bars now move up/down per band instead of scrolling R→L).
- **`GAIN` (=6.0) is the one tuning knob** for bar liveliness; peak-per-band (not average) keeps
  it punchy. Cost: one 2048-pt FFT ~12×/s on the existing throttle — negligible.
- **Decode/back-pressure:** uses the most-recent samples of each already-mixed chunk; the capture
  pipeline is untouched.

## Acceptance criteria

- Tie back to the Definition of Done in `/CLAUDE.md`.
- All three recording surfaces show vocal-range frequency bars that move up/down with speech and
  flatten on silence (not a right-to-left timeline).
- The new FFT path is throttled and does not regress capture/transcription (smoke path intact).

## Risks / open questions

- **Cost on the hot loop:** per-chunk FFT must stay cheap; reuse a single planner, window in place.
  Confirm no audio-thread stalls.
- **Band count & cutoffs:** proposed ~80 Hz–4 kHz log-spaced, 12–24 bands — tune visually.
- **Normalization:** fixed vs. AGC. Start with a fixed reference; a slow auto-gain may read better.

## Verification

- Rust: `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal && cargo
  clippy`; unit test the banding on a synthesized sine.
- Frontend: `cd frontend && pnpm lint && pnpm test`.
- Manual: record and speak; confirm bars track speech and flatten on silence on all three surfaces.
