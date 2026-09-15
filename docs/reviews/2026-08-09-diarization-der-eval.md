# Diarization DER eval — baseline, sweep, and the "noisy separation" verdict

**Date:** 2026-08-09 · **Trigger:** owner report that remote speaker *separation* is still noisy
(one real person split across many speaker labels), plus a standing worry that past diarization
changes made things worse with no measurable test. This documents (a) the true build-state of
spec 0039, (b) the existing DER eval harness, (c) a full baseline + parameter sweep, and (d) the
evidence-based decision **not** to change clustering — with the lever that *would* help.

## 1. Spec 0039 is shipped (header was stale)

All three workstreams of `specs/0039` shipped in **v1.7.0 (2026-07-06)**: WS1 same-voice
consolidation (`consolidate_clusters` + `CONSOLIDATE_FLOOR`), WS2 span-level correction
(`api_set_segment_speakers`, `api_create_meeting_speaker`, span-select UI), WS3 voiceprint
pollution guard (quarantine migration, `EnrollConfidence` gate, per-sample purge, retraction).
Follow-on **0041** bounded the consolidation cascade (anti-snowball) and **0043 W2** swapped the
embedding to **TitaNet-L**, taking macro DER ~33% → ~8%. The spec's `Draft (not built)` header was
simply never flipped; corrected in this change.

WS1's one deliberate deviation: quiet-cluster *suppression* was dropped in code review ("merge,
never delete a real speaker").

## 2. The eval harness already exists — this is the regression gate

`frontend/src-tauri/tests/diarization_tuning.rs` computes real **DER** (10 ms frames, ±0.25 s
collar, greedy overlap mapping, miss/false-alarm/confusion) against the `zoom-samples/` Zoom VTTs
(real speaker names + timings as ground truth). Two `#[ignore]` entry points:

- `eval_ground_truth_sweep` — floors 0.50–0.80 × bounds{on,off} × mode{Auto, AtMost(n)}; prints
  DER + predicted-count per file + a macro ranking + worst confusion pairs.
- `eval_regression_gate` — asserts per-file DER stays under pinned ceilings (short ≤14%,
  long-1-1 ≤9%, long-many ≤12%). **This is the safety net that was missing.**

Run (CPU provider avoids a CoreML teardown-abort in the multi-pass loop):
```bash
cd frontend/src-tauri && source ~/.cargo/env
VINYL_DIARIZATION_PROVIDER=cpu cargo test --features metal --test diarization_tuning \
    eval_regression_gate -- --ignored --nocapture      # pass/fail gate
    # or eval_ground_truth_sweep for the full table
```
The scorer itself has fast, non-ignored unit tests (perfect=0 DER, collapsed-cluster=confusion,
miss/FA), so the metric is CI-protected even though the real-audio runs are gated.

## 3. Baseline (shipped defaults: TitaNet-L, floor 0.50, bounds on)

| sample | true n | Auto DER / pred | AtMost(n) DER / pred |
|---|---|---|---|
| long-1-1 | 2 | 5.98% / **25** | 5.73% / **3** |
| long-many-speakers | 20 | 8.33% / **98** | 8.39% / **21** |
| short-4-speakers | 4 | 10.25% / **29** | 10.10% / **5** |

**DER is already good (~8% macro).** The owner-visible "noise" is the **Auto predicted-count
sprawl** (25/98/29 vs true 2/20/4): one real voice is split across many clusters. DER stays low
because the extra clusters are tiny slivers (DER-cheap) — but the transcript *looks* like a crowd.
`AtMost(n)` (given the true count) fixes the count to near-exact at equal-or-better DER.

## 4. Sweep verdict — clustering can't safely fix the sprawl

Best config over all three files: **`floor 0.50 | bounds off | Auto`** (aggressive/unbounded
consolidation): DER 5.87 / 8.14 / 10.21 (macro **8.07%**, vs shipped **8.19%**), counts 23 / 92 /
28. So it's *marginally* better on DER and slightly fewer clusters, and notably **more robust**:
bounds-*on* is fragile to the floor constant — at floor 0.55 it **collapses long-1-1 to 29% DER**
(fragments left unmerged map to the wrong speaker), whereas bounds-*off* at 0.55 stays 5.98%.

**But it does not meaningfully reduce the sprawl** (25→23, 98→92, 29→28). The surviving slivers
sit *below* the safe merge floor: on short segments the TitaNet centroid is noisy, so a sliver's
centroid isn't similar enough to its parent to merge without dropping the floor into the **cliff**
where genuinely distinct speakers fuse and DER explodes (e.g. long-1-1 jumps 5.98% → 29% once the
floor stops merging the big fragments). There is **no floor that collapses the slivers without
fusing real people.** Only `AtMost(n)` — which *knows* when to stop — reaches the true count.

**Worst confusion at the shipped config** is small and sensible (e.g. long-many: Franco↔Arno 12 s,
short-4: Scott↔the owner ~10 s) — i.e. genuinely similar/overlapping voices, not a systematic bug.

## 5. Decision: do NOT change clustering now

- The only clustering lever (aggressive consolidation) buys ~0.1 pp DER and barely touches the
  sprawl. Not worth touching core clustering for.
- Shipping `bounds:off` would remove the **0041 anti-snowball budget**, which was added because an
  open-ended pass once "snowballed a whole meeting into the dominant speaker." That motivating
  meeting is **not in the 3-file eval set**, so the harness cannot prove `bounds:off` is safe on
  it — precisely the "3 files can't prove it" risk. Prudent: leave clustering alone.

## 6. The lever that actually helps: the speaker-count seed

`AtMost(n)` is the whole game (count → near-exact, DER equal-or-better, floor-independent). The
improvement path is **getting a good `n`**, not tuning clustering:

1. **Populate the seed we already honor.** `resolve_speaker_count` already produces `AtMost(n)`
   from calendar attendees or the participant roster; the gap is meetings with neither. `specs/0048`
   added a default `AtMost(10)` cap for that case — good for small/medium calls, but it caps a
   genuine 20-person ad-hoc meeting too low. Improving roster/calendar capture is the low-risk win.
2. **(Proposal, must be validated) duration-based `n` estimation for Auto.** The real speakers are
   the *dominant* clusters; slivers are tiny. Estimate `n` = clusters whose pooled speech exceeds a
   threshold, then apply `AtMost(n_est)` — potentially reaching the oracle without a calendar seed.
   This is a **new** heuristic: it must be built behind the DER gate and, ideally, validated on more
   labelled files (including a snowball-prone one) before shipping. Not done here.

## 7. How to safely tune diarization in future

Always: capture baseline (`eval_regression_gate`), make the change, **clear `zoom-samples/*/.eval-cache/`
if you touch the raw clustering or embedding model** (the cache keys on threshold + model), re-run
`eval_ground_truth_sweep` (full picture) + `eval_regression_gate` (hard pass/fail), and re-pin
`EVAL_PINNED_DER` only with fresh measured values. Add more labelled meetings to `zoom-samples/`
(one `.mp4` + one `.transcript.vtt`) to widen coverage — especially a meeting that previously
regressed, so the gate protects it.
