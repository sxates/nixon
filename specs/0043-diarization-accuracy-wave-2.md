# 0043 — Diarization accuracy wave 2: DER ≤ 25%

- **Status:** Implemented — goal exceeded: macro DER 8.07% AtMost / 8.19% Auto
  (target ≤ 25%; baseline 47.1%/55.0%). TitaNet-L chosen (ADR-0011).
- **Owner agent(s):** audio-engineer (waves 1–2), rust-core-engineer (gallery filter)
- **Roadmap phase:** Identity & Organization (follows 0039/0041 WS1)

## Context / Problem

The ground-truth eval harness (`frontend/src-tauri/tests/diarization_tuning.rs`, samples in
`../zoom-samples/`: short-4-speakers, long-1-1, long-many-speakers) measures **macro DER
~55% Auto / ~47% AtMost(true-n)** at shipped defaults (pinned 2026-07-08 in
`EVAL_PINNED_DER`, diarization_tuning.rs:1293). Owner-visible symptoms: **distinct speakers
merged into one**, and **"You" appearing on other speakers' speech**.

Root causes, from the completed investigation + specs/0041 WS1:

1. **Cap-path over-merging.** `merge_clusters_to_at_most` (`diarization/sherpa.rs:849`) is
   bounded only by `MERGE_FLOOR = 0.30` (sherpa.rs:188). Measured distinct-voice centroid
   pairs reach cos 0.62 on our samples, so real speakers fuse whenever the raw pass
   over-produces clusters. The consolidation floor (0.70, sherpa.rs:201/229) is second-order;
   raw cluster impurity upstream dominates (dominant raw cluster only ~55% pure on the
   4-speaker file).
2. **"Silence means you" heuristic.** In `align_turns_to_segments`
   (`diarization/align.rs:95-129`), Rule 3 sends no-overlap **Mixed**/untagged segments to
   `LOCAL_SPEAKER_KEY`; the live path `align_system_turns_to_segments` (align.rs:153-173)
   *always* falls back to local. Any remote speech the system-channel diarizer missed
   becomes "You".
3. **Speaker bleed, no AEC.** Channel attribution is pre-mix RMS dominance
   (`audio/pipeline.rs:56-160`; `CHANNEL_ACTIVE_RMS = 0.01`,
   `CHANNEL_DOMINANCE_RMS_RATIO = 3.0` ≈ 9.5 dB). Laptop-speaker playback picked up by the
   mic can tag remote speech as Microphone → hard "You" via align Rule 1.
4. **Weak embeddings.** CAM++ VoxCeleb EN 16k (`diarization/embedding.rs:39`,
   ~28 MB) is the smallest usable model in the sherpa zoo; same-meeting distinct voices
   land too close in its embedding space for any threshold to separate cleanly.
5. **Owner suggested as others.** The suggestion gallery
   (`diarization/pipeline.rs:966-1035`) includes the owner's voiceprint centroid in
   `all_centroids` candidates (pipeline.rs:1005-1022) — a residual "label a remote speaker
   as You" path. (Owner IS already excluded from roster sizing,
   `meeting_participant.rs`.)

## Goals

- **Macro DER ≤ 25%** across the three ground-truth samples in AtMost(true-n) mode, with
  Auto mode materially improved and both recorded.
- Stop defaulting ambiguity to "You": ambiguous segments become "Unknown speaker", never
  a wrong hard label. Mic-channel → You (Rule 1) stays untouched.
- Owner voiceprint never appears as a suggestion candidate for another speaker.
- Each change independently measured via the harness; `EVAL_PINNED_DER` ratcheted down as
  improvements land.

## Non-goals

- **Full acoustic echo cancellation (AEC).** Wave 1 ships only cheap RMS-level bleed
  mitigation; a real AEC (e.g. speex/WebRTC APM on the mic track) is a future spec.
- Overlap-aware *attribution* UX (two speakers on one transcript segment) — investigate
  the segmentation capability only (W2.3).
- New correction UI. Span-level speaker correction is specs/0039 WS2 and is the recovery
  path for the extra "Unknown speaker" labels this spec introduces.
- Diarizing the mic channel / multi-local-speaker support (ADR-0005 #3 stands).

## Approach

Two waves, measured between every task. **Wave 1** exhausts the current architecture:
re-tune the three clustering constants against ground truth (the sweep harness + env knobs
already exist), remove the two "default to You" fallbacks, filter the owner from the
suggestion gallery, and cheaply demote bleed windows to Mixed. **Wave 2** swaps the
embedding model — the biggest expected lever given raw-cluster impurity — benchmarking 2–3
stronger sherpa-zoo models through the same harness, plus an investigate-then-decide on
overlap-aware segmentation. Alternative considered: jumping straight to a different
diarization stack (pyannote-full via ONNX, or offline re-clustering with our own AHC) —
rejected for now; sherpa's pipeline is model-pluggable and the harness will tell us if we
plateau.

## Design

### Wave 1 — quick fixes in the current architecture

**W1.1 Raise the cap-path merge floor.** `MERGE_FLOOR` (sherpa.rs:188) 0.30 → ~0.55–0.60,
picked by sweep. When `AtMost(n)` can't be honored above the floor, the existing behavior
(top-n by speech duration stay, overflow → Unknown bucket, sherpa.rs:190-199) is the
*correct* failure mode: an "Unknown speaker" label beats fusing two real people. Keep the
`CONSOLIDATE_FLOOR > MERGE_FLOOR` clamp invariant (sherpa.rs:366-376) valid.

**W1.2 Re-tune `DEFAULT_AUTO_THRESHOLD`.** sherpa.rs:161, currently 0.8. The knob already
exists: `VINYL_EVAL_RAW_THRESHOLD` (diarization_tuning.rs:585) feeds the raw pass in
`eval_ground_truth_sweep` (:1043). Sweep 0.5–0.9, pick the macro-DER minimum that does not
regress long-1-1 (the 0041 lesson: floor 0.75+ regressed the 2-speaker file).

**W1.3 Ambiguity → Unknown, not You.**
- Offline (`align_turns_to_segments`, align.rs:95-129): Rule 3 for `Channel::Mixed`
  changes from `LOCAL_SPEAKER_KEY` to `SYSTEM_FALLBACK_KEY` ("Unknown speaker") — or, if
  the harness shows it helps, nearest-turn attribution within a small window (~1.0 s)
  before falling back to Unknown. `Channel::System` fallback already goes to Unknown.
- Live (`align_system_turns_to_segments`, align.rs:153-173): the unconditional
  local fallback likewise becomes nearest-turn-within-window, else Unknown.
- **Rule 1 (Microphone → `LOCAL_SPEAKER_KEY`) is untouched.**
- UX consequence: more "Unknown speaker" labels in transcripts. Accepted — a wrong "You"
  is worse than an honest Unknown, and 0039 WS2 span-correction is the recovery path.
  Note in align.rs docs that legacy all-null-channel meetings now map Mixed→Unknown too
  (behavior change on re-runs; acceptable).

**W1.4 Owner out of the suggestion gallery.** In `compute_suggestions_with_emails`
(`diarization/pipeline.rs:966-1035`), skip `person_id == OWNER_PERSON_ID`
(`people/enroll.rs:34`) when pushing gallery centroids (pipeline.rs:1007), and add the same
guard to the 1a prior-speaker candidates (pipeline.rs:989-1000) if any carry the owner
person. Unit test: a candidate set containing the owner centroid produces no owner-labeled
suggestion.

**W1.5 Cheap bleed mitigation.** In `audio/pipeline.rs:90-160`: (a) sweep
`CHANNEL_ACTIVE_RMS` / `CHANNEL_DOMINANCE_RMS_RATIO` against the harness (the zoom samples
carry real bleed); (b) add a correlated-energy heuristic — when a window is mic-dominant
*and* system RMS is simultaneously above a high activity bar, demote to `Mixed` (which,
post-W1.3, resolves via diarizer turns instead of hard "You"). Pure function change; extend
the existing channel-classification unit tests. If the harness shows no measurable DER
gain, keep constants and document the negative result.

### Wave 2 — embedding/backend upgrade

**W2.1 Benchmark stronger embedding models.** Zoo survey (sherpa-onnx
`speaker-recongition-models` release, verified 2026-07-10) gives these English 16k
candidates, all loadable by the same `DiarizeConfig` path (`sherpa.rs:15`; sherpa-rs 0.6.8
takes an arbitrary embedding model path):

| Candidate | Size | Why |
|---|---|---|
| `wespeaker_en_voxceleb_resnet293_LM.onnx` | ~114 MB | strongest wespeaker VoxCeleb line (LM = large-margin) |
| `nemo_en_titanet_large.onnx` | ~101 MB | NeMo TitaNet-L, strong EN verification baseline |
| `wespeaker_en_voxceleb_resnet34_LM.onnx` | ~27 MB | same footprint as CAM++; cheap fallback if big models are too slow |

(ERes2NetV2 exists in the zoo only as zh-cn — not a fit for the EN eval set.) Benchmark
via a harness knob (`VINYL_EVAL_EMBEDDING_MODEL` env → model path) added to
diarization_tuning.rs; re-run the full sweep per model since the cosine scale shifts —
thresholds/floors from W1.1–W1.2 must be re-picked per embedding space. Pick the best
DER/latency trade-off; measure the offline pass wall-clock on the long files.

**W2.2 Ship the chosen model.** Extend `diarization/models.rs` (the on-demand
download+cache: const URL + min-bytes guard + `ensure_models`, models.rs:23-141) with the
new `EMBEDDING_MODEL_FILE`/`EMBEDDING_URL`; bump `EMBEDDING_MODEL_ID`
(`embedding.rs:39`) and the re-embed `ClusterEmbedder` to match.
**Voiceprint migration:** storage is already keyed per `embedding_model`
(`database/repositories/voiceprints.rs` — best-N per `(person_id, embedding_model)`,
same-model-only reads; ADR-0007 §4). So old CAM++ voiceprints simply stop matching —
no corruption, no schema change. Decision: **dual-keyed re-enrollment** — keep old rows
(harmless, pruned per-model), let the gallery repopulate organically under the new model
id via the existing auto-enroll path; no bulk re-embed of old meetings' audio. People
directory shows "no voiceprint yet" under the new model until the owner/others speak in a
new meeting. Write a short ADR (ADR-0011) recording the model choice + migration stance.

**W2.3 Overlap-aware segmentation (investigate-then-decide).** pyannote segmentation-3.0
(our `SEGMENTATION_MODEL_FILE`) is powerset/overlap-capable, but sherpa-onnx's offline
diarization collapses it to single-speaker turns and sherpa-rs 0.6.8 exposes no overlap
output. Timebox: confirm against sherpa-onnx C API; if unexposed, record findings +
options (upstream PR, raw-onnx segmentation pass of our own) in the ADR and stop.

### Data model
No schema changes. `voiceprints` rows are per-`embedding_model` already; new-model rows
coexist with CAM++ rows.

### Tauri IPC
None. All changes are inside the diarization/audio pipeline and test harness.

### UI
No new components. Expect more "Unknown speaker" rows in `frontend/src/` transcript views
(already rendered today for `SYSTEM_FALLBACK_KEY`); verify styling copes in smoke.

## Tasks

Measure with `eval_ground_truth_sweep` + `eval_regression_gate` after **every** task;
update `EVAL_PINNED_DER` (diarization_tuning.rs:1293) whenever a task improves a pin.

1. [x] **W1.4** Owner filter in `compute_suggestions_with_emails` + unit test.
       *(`2f42a10`)*
2. [x] **W1.1** `MERGE_FLOOR` 0.30 → 0.60 for CAM++ (`3afed73`), re-swept → **0.45**
       for TitaNet space (0.30 scores 20.3% there — the scale is compressed).
3. [x] **W1.2** `DEFAULT_AUTO_THRESHOLD` 0.8 → 0.55 for CAM++ (32.9% macro,
       `3afed73`), re-swept → **0.80** for TitaNet (threshold-robust, no cliff).
4. [x] **W1.3** Ambiguity → nearest-turn (≤1.0 s edge gap) → Unknown, offline + live;
       mic hard-constraint untouched. *(`66360e4`)*
5. [x] **W1.5** Bleed guard: mic-dominant windows with system ≥ 5×ACTIVE_RMS demote
       to Mixed (R128 normalization makes echo look mic-dominant). *(`40f1e84`)*
6. [x] **W2.1** Harness knob (`5c900f1`) + benchmark: TitaNet-L 8.1%,
       ResNet34-LM 25.2%, ResNet293-LM 27.4%, CAM++ tuned 32.8%. Winner: TitaNet-L.
7. [x] **W2.2** TitaNet wired (`models.rs`, `EMBEDDING_MODEL_ID`), constants re-set
       for its space, pins ratcheted, ADR-0011 accepted; identity.rs TAUs measured
       still-valid (same-person cross-meeting ≈ 0.70–0.75) and left unchanged.
8. [x] **W2.3** Overlap investigation: sherpa already emits overlapping turns
       end-to-end; no extra segmentation pass needed (ADR-0011 Decision 3). *(`56f75c6`)*
9. [x] **Final** Target exceeded: macro 8.07% AtMost / 8.19% Auto at shipped
       defaults — no plateau handling needed. Residual (non-blocking): Auto-mode
       cluster-count over-split (25/98/29 pred vs 2/20/4 true; sub-10s slivers).
       Next lever if it bothers real no-calendar meetings: multi-pass consolidation
       with cross-pass snowball-guard state.

## Acceptance criteria

1. `eval_ground_truth_sweep` macro DER ≤ 25% in AtMost(true-n) mode at shipped defaults;
   Auto-mode macro DER recorded and materially below the 2026-07-08 ~55% baseline.
2. `eval_regression_gate` passes with pins ratcheted to the new measured values (+~3 pp
   headroom); **long-1-1 DER not worse than its 2026-07-08 pin** at any point.
3. align.rs unit tests: Mixed/no-overlap → Unknown (offline), live no-overlap →
   nearest-turn/Unknown; existing mic-hard-constraint test unchanged and green.
4. Owner voiceprint filter covered by a unit test; no suggestion ever carries
   `OWNER_PERSON_ID` for a non-mic speaker.
5. ADR-0011 exists (embedding model choice, voiceprint re-enrollment, overlap findings).
6. Definition of Done (`/CLAUDE.md`): cargo check/clippy/test, pnpm lint/test, file-size
   ratchet, `/check`, and the record → live transcript → summary smoke on a real call
   (verify "You" only on actual owner speech; Unknown labels render sanely).

## Risks / open questions

- **The 20-speaker far-field file (long-many-speakers) may dominate the macro** and cap
  achievable DER; if so, report per-file DER and let the owner judge whether ≤ 25% macro
  needs a harder lever (task 9's stop condition).
- **Embedding swap invalidates voiceprints** — mitigated by per-model keying, but users
  lose cross-meeting matching until galleries re-fill; a release-notes line is needed.
- **English-only eval samples**; the chosen EN model may underperform for multilingual
  users vs the zh-en CAM++ variants. Out of scope to fix; note in ADR.
- **Eval tests are `#[ignore]`d and need `--features metal` + local sample files** — CI
  cannot gate DER; all measurement is local-only (pins are the only CI-visible artifact,
  and even those run only when invoked with `--ignored`).
- Model size grows ~28 MB → ~100 MB download on first diarization; acceptable (on-demand,
  ADR-0005 decision 4) but worth a progress-UI sanity check.

## Verification

```bash
cd frontend/src-tauri && source ~/.cargo/env
# Full sweep + gate (local samples required):
cargo test --features metal --test diarization_tuning eval_ground_truth_sweep -- --ignored --nocapture
cargo test --features metal --test diarization_tuning eval_regression_gate -- --ignored --nocapture
# Unit layers:
cargo test --features metal          # align.rs, sherpa.rs, audio/pipeline.rs, suggestion filter
cd ../ && pnpm lint && pnpm test && ../scripts/check-file-size.sh
```

Manual smoke: `./dev-vinyl.sh`, record a Zoom call with ≥ 2 remote speakers + laptop
speakers (no headset) → confirm live labels don't show "You" on remote speech, offline
pass separates the remote speakers, and suggestions never propose the owner for a remote
speaker.
