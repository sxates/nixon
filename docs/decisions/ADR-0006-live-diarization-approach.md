# ADR-0006 — Live diarization: periodic re-diarization on the growing buffer, offline pass authoritative

- **Status:** Proposed (pending Brian's decisions in `specs/0011`)
- **Date:** 2026-06-25
- **Related:** `specs/0011-diarization-p3-live-and-cross-meeting-identity.md`,
  `docs/decisions/ADR-0005-diarization-engine.md` (sherpa-onnx; `Diarizer` trait), `specs/0010`

## Context

P1/P2 shipped **offline, post-meeting** speaker diarization: at stop, `diarization/pipeline.rs` runs
`SherpaDiarizer::diarize()` over the whole `system.wav` and aligns turns to transcript segments. P3
(Task 10) wants speaker labels to appear **during** recording. The decision is *how labels appear live*,
and it must satisfy two constraints that pull against each other:

1. **Accuracy.** Brian observed P2 diarization is "not entirely accurate." Live diarization inherits the
   offline engine's accuracy, and a naive live approach makes inaccuracy worse and *visible in real
   time* (labels flickering between speakers as the model changes its mind).
2. **Don't starve live STT.** P1 ran offline specifically so diarization wouldn't compete with
   Whisper/Parakeet. Live re-introduces CPU contention.

Two facts constrain the engine choice: sherpa-onnx's diarization API is **offline/batch only**
(`Diarize::compute(full_buffer)`); and the `speakrs` alternative is likewise a batch pyannote pipeline.
Neither offers true streaming clustering, and no mature Rust streaming-diarization binding exists.

## Decision

1. **Periodic re-diarization on the *growing* buffer.** During recording, accumulate the (already
   captured, pre-mix, 16 kHz mono) **system** channel and, every ~5–10 s, run the **existing offline
   `SherpaDiarizer`** over the audio captured *so far*, on a CPU `spawn_blocking` thread, single
   in-flight and debounced. We reuse the shipped, tested clustering path rather than inventing streaming
   or sliding-window clustering. Clustering the whole-so-far buffer is the most accurate mid-call result
   available — it *is* the offline algorithm, run earlier and repeatedly.
2. **The mic channel stays a zero-latency `You` short-circuit, live.** Only the system channel is
   diarized (per ADR-0005); mic-dominant speech is labeled `You` the instant it is transcribed,
   independent of the diarizer.
3. **Label stability via an embedding-anchored session id map + "stable-once-shown".** sherpa's cluster
   ids are per-run and arbitrary. A live-session registry maps each pass's raw clusters onto
   session-stable keys by matching cluster centroid embeddings (cosine ≥ τ_session) to known session
   speakers. A label, once shown for a segment, is **never silently changed** by a later live pass; live
   transitions are only `unlabeled → labeled`.
4. **The offline pass at stop is authoritative.** The shipped P1 offline pass still runs on the final
   `system.wav` and produces the source-of-truth labels. Because live used the *same engine on a prefix*
   of the same audio, the final relabel is usually a no-op or small and is applied as one quiet,
   atomic update — not continuous on-screen churn.
5. **Accuracy gate before building live.** Per `specs/0011`, an accuracy spike (benchmark + sherpa
   tuning, then `speakrs` evaluation behind the `Diarizer` trait if needed) must clear an agreed
   attribution bar before the live build proceeds. The `Diarizer` trait (ADR-0005) keeps the `speakrs`
   swap cheap.

## Alternatives rejected

- **True online/streaming diarization** (sample-by-sample turn emission, diart-style incremental
  clustering): no offline-only sherpa support, no mature Rust binding, highest accuracy risk, worst
  flicker. Rejected.
- **Low-latency sliding-window re-clustering** (cluster only the last N s): discards the whole-conversation
  context that makes clustering accurate, and re-introduces the hard window-stitching/re-numbering
  problem we want to avoid. Rejected as the *default*; retained only as a **fallback knob** (cap the live
  window to the last M minutes) if profiling shows full-buffer recompute is too heavy late in long calls
  — with the offline-at-stop pass still processing the full audio.

## Consequences

- **Zero new engine/model.** Live reuses `SherpaDiarizer` + the already-downloaded ONNX models; no
  streaming clustering to build or maintain.
- **CPU contention is bounded, not eliminated.** Diarizer is CPU-only (never the GPU/Metal path STT
  uses), single in-flight, debounced, `spawn_blocking`. `specs/0011` makes "live STT latency within an
  agreed bound of OFF" an acceptance criterion, plus a concurrent-`compute()`-with-STT smoke (a new
  stress point beyond ADR-0005's at-init coexistence proof).
- **Recompute grows with call length.** Each late pass re-clusters more audio. Mitigated by debounce +
  single-flight; the sliding-window cap is the documented fallback.
- **Labels are provisional but stable.** Users see labels that may be *coarsely* corrected exactly once,
  at stop — never flickering live. The UI must signal "provisional, finalized when you stop."
- **An embedding centroid per cluster is needed live** (for the stable id map) — the same per-cluster
  embedding `specs/0011` Task 11 extracts for cross-meeting identity, so the two P3 efforts share
  `diarization/embedding.rs`.
