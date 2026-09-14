# ADR-0011 — Diarization embedding-model upgrade & overlap findings (specs/0043)

- **Status:** Accepted (2026-07-12) — TitaNet-L chosen from the W2.1 benchmark
- **Date:** 2026-07-10 (drafted) / 2026-07-12 (decided)
- **Relates to:** ADR-0005 (diarization engine), ADR-0007 (voiceprint storage), specs/0043

## Context

specs/0043 targets macro DER ≤ 25% on the ground-truth eval harness (baseline ~47%
AtMost / ~55% Auto). The 0041 measurements showed raw cluster impurity — not
post-processing — dominates the error, pointing at the embedding model
(3D-Speaker CAM++ VoxCeleb EN 16k, ~28 MB, `EMBEDDING_MODEL_ID` in
`src/diarization/embedding.rs`) as the main lever.

## Decision 1 — embedding model: **NVIDIA NeMo TitaNet-L** (`nemo_en_titanet_large.onnx`)

Benchmarked 2026-07-11/12 through `eval_ground_truth_sweep` (each candidate swept over
raw thresholds 0.40–0.80 in its own embedding space; best macro DER shown):

| Candidate | Size | Best macro DER | Notes |
|---|---|---|---|
| **`nemo_en_titanet_large.onnx`** | ~101 MB | **8.1%** | winner; threshold-robust (8.1–8.7% across the whole sweep, no cliff) |
| `wespeaker_en_voxceleb_resnet34_LM.onnx` | ~27 MB | 25.2% | 20-speaker file stuck at 37–44% |
| `wespeaker_en_voxceleb_resnet293_LM.onnx` | ~114 MB | 27.4% | worse than its smaller sibling here |
| (baseline) CAM++ VoxCeleb, wave-1 tuned | ~28 MB | 32.8% | per-file 15.3/54.8/28.3% |

TitaNet-L per-file at shipped defaults (threshold 0.80, `MERGE_FLOOR` 0.45,
`CONSOLIDATE_FLOOR` 0.50): long-1-1 5.7%, long-many-speakers (20 spk) 8.4%,
short-4-speakers 10.1% — macro **8.07% AtMost / 8.19% Auto**, versus the pre-0043
baseline of 47.1%/55.0%. The 20-speaker far-field file — immovable under CAM++ at any
tuning — dropped 54.8% → 8.4%, confirming raw-cluster impurity was an embedding-space
limitation. License: CC-BY-4.0 (NVIDIA NeMo), served from the sherpa-onnx model zoo.

**Cosine-scale consequences (measured, not assumed):** TitaNet's within-meeting scale is
compressed — same-voice drift-splits sit ~cos 0.50–0.55 (CONSOLIDATE_FLOOR 0.50, was
0.70/0.65 for CAM++), distinct voices reach above 0.30 (MERGE_FLOOR 0.45, was 0.30
originally). Cross-meeting same-person centroids measured ~0.70–0.75 (the owner appears
in all three eval recordings), so the `identity.rs` suggestion tiers (`TAU_MATCH` 0.5,
`TAU_AUTO_LABEL` 0.7, margin 0.06) remain valid **unchanged** in TitaNet space.

Known caveat: Auto mode (no calendar attendee count) over-splits cluster counts
(e.g. 25 predicted for a 2-speaker call; sub-10s slivers, DER-cheap). AtMost counts are
essentially exact. Next lever if label sprawl bothers real no-calendar meetings:
multi-pass consolidation with cross-pass snowball-guard state.

## Decision 2 — voiceprint migration on model change

Voiceprint storage is already keyed per `(person_id, embedding_model)` with
same-model-only reads (ADR-0007 §4), so a model change cannot corrupt matching — old
CAM++ rows simply stop being read. We adopt **dual-keyed organic re-enrollment**: keep
old rows, let galleries repopulate under the new model id via the existing
enroll-on-confirm path. No bulk re-embedding of historical audio, no schema change.
User-visible consequence (needs a release-notes line): cross-meeting speaker
suggestions go quiet until people speak in a new meeting under the new model.

## Decision 3 — overlap-aware segmentation: rely on existing pass-through (W2.3)

The 0043 spec assumed sherpa-onnx collapses pyannote segmentation-3.0's overlap
detection to single-speaker turns. **Investigation (2026-07-10, against the vendored
sherpa-onnx source in sherpa-rs-sys 0.6.8) showed both halves of that premise false:**

- `offline-speaker-diarization-pyannote-impl.h` decodes the powerset per frame
  (`ToMultiLabel`, pairs allowed), computes per-frame speaker *counts* before overlap
  exclusion, excludes overlapped frames only from *embedding extraction* (hygiene, not
  flattening), marks top-k clusters active per frame in `FinalizeLabels`, and emits
  segments per speaker independently — so **time-overlapping (start, end, speaker)
  segments come out of the C API**, and sherpa-rs 0.6.8 returns them 1:1.
- Our `sherpa.rs` maps them 1:1 into `SpeakerTurn`s; consolidation only relabels.
  Overlap collapses only at transcript attribution (`align.rs` picks one speaker per
  transcript row) — an STT-bounded product constraint (Whisper emits one text stream),
  not a diarization limitation. The eval harness scores turns pre-alignment, so
  overlapping hypothesis turns already score at overlapped reference frames.

Therefore: **no separate raw-ONNX segmentation pass** (it would replicate the vendored
implementation and risks a second onnxruntime linkage alongside the sherpa-bundled
dylib), and no upstream PR (nothing missing at interval granularity). Overlapped
speech is plausibly ~5–15% of meeting speech time — single-digit DER points against a
~50-point problem — so the embedding swap carries the DER goal. Optional follow-up if
wave 2 plateaus: half-day harness instrumentation to report reference-vs-hypothesis
overlap share and quantify the residual.

## Consequences

- First diarization after the upgrade downloads a ~100 MB model (vs ~28 MB) via the
  existing on-demand `models.rs` path (ADR-0005 decision 4); progress UI should cope.
- English-trained candidate models may underperform for multilingual users vs the
  zh-en CAM++ variants; accepted for now (eval set is English).
