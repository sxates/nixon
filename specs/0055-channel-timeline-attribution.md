# 0055 — Channel timeline attribution (row granularity)

- **Status:** Stage 1 implemented (no human smoke yet); Stage 2 not started
- **Owner agent(s):** audio-engineer (backend) + frontend-engineer (live render)
- **Roadmap phase:** Feedback / diarization accuracy

## Context / Problem

Owner report (2026-08-26, v1.18.0): *"attribution is off by 1 or 2 speaking blocks.
Speaker A is talking, then I start talking. The first block or two that I say gets
attributed to Speaker A. Then I also get attribution for the first block or two of
whatever the next speaker says."* Seen **both** live and in the final transcript;
the owner's audio-out setup **varies** (speakers some meetings, headphones others).

This is the fourth report of the same symptom (0044 W1.2, 0046, 0047). Each previous
fix adjusted which *turns* may claim a *row*; none changed the fact that the row is
the unit of attribution. That is the actual defect.

### Evidence (real capture, measured — not hypothesised)

Measured on `zoom-samples/Ad-Hoc Steerco_2026-08-07_22-31` (real 51-min two-channel
capture: `mic.wav` + `system.wav` + `transcripts.json`, 346 rows / 3068 s). A Python
replication of `classify_window_channel` + `dominant_channel_for_span` reproduces the
stored `transcripts.channel` values **346/346 = 100%**, so these are the shipped
classifier's own numbers.

**Root cause — the 600 ms channel signal is collapsed to one tag per row:**

| Fact | Value |
|---|---|
| Rows containing **both** owner and remote speech | **109 / 346 (32%)** |
| Speech time in those rows wrong by construction | 96 s |
| Owner speech landing in a `microphone`-classified 600 ms window | **86%** |
| Rows tagged `mixed` / `microphone` / `system` | 291 / 51 / **2** |
| Remote speech sitting in `mixed` windows | 96% |

The signal needed to place the boundary **already exists at 600 ms resolution and is
86% accurate**; `dominant_channel_for_span` discards it. Rows run up to 14 s and can
span several speakers. Two representative straddles (`O` = owner, `R` = remote, 200 ms
slices): `221.4–224.3s` tagged `microphone` → `..OO.......RRRR`, and `868.8–881.6s`
tagged `mixed` → `OOOOOOOOO....RRRR.RRRRRRRR..`.

**Why the `system` tag is dead (2/346):** the mic path is EBU R128-normalized, so
speaker bleed is boosted to match the clean system tap and neither channel clears the
3× dominance ratio. Almost everything lands in `mixed`, and a `mixed` row can never be
"You" (0047 W2 filters owner turns out of non-mic rows) — so the owner is labeled
correctly only when the whole-row RMS vote happens to land `microphone`.

**Candidate rules scored** on share of speech time assigned to the wrong *source*
(owner vs remote), the axis this bug is about:

| Rule | Wrong | |
|---|---|---|
| A. Current row-level RMS vote | 112.4 s / 1365.8 s | **8.23%** |
| B. Demote `microphone` when any system energy is present | 118.0 s | **8.64% — worse** |
| C. Split rows on the 600 ms channel timeline | 37.2 s | **2.72%** |

Tightening the tag (B) strips correct "You" labels faster than it removes wrong ones.
Only keeping the timeline (C) helps: a **67% cut** in source-level misattribution.

### Live path — corrected finding

The backend live diarizer is **not involved** for this owner: `diarization.json` has
`live_diarization_enabled: false` (dev and prod), so no `live-diarization-update` ever
fires. Live labels come solely from `TranscriptContext.tsx:394`
(`update.channel === 'microphone' ? 'You' : null`). The live symptom is therefore the
*same* root cause arriving through the channel tag: a straddling row tagged
`microphone` shows "You" over the next speaker's opening words, and the owner's own
speech inside a `mixed` row shows no name at all.

An earlier proposal to thread `channel` into the backend's `LiveSegment` was rejected:
the frontend already applies that rule, and the backend live diarizer is off.

## Goals

- **Stage 1 (this spec, live):** the channel signal reaches the UI at window
  resolution instead of one tag per row, and a live transcript row that straddles an
  owner↔remote handoff renders as separate labeled parts.
- **Stage 2 (separate slice):** persist the timeline and cut stored rows at channel
  boundaries so the *final* transcript gets the 2.72% number, replacing owner-turn
  reconstruction (`owner_turns.rs` + `filter_bleed_owner_segments`) as the boundary
  finder.

## Non-goals

- **Not touching clustering.** Which *remote* speaker a remote part belongs to is a
  separate axis (0039/0048/0050, DER ~8%). This spec only fixes the owner-vs-remote
  boundary, which is what the owner reported.
- **No DB migration in Stage 1.** `transcripts.channel` keeps its current meaning and
  the offline pass is untouched; stored rows still straddle until Stage 2.
- **Not re-tuning `classify_window_channel`.** At 600 ms it is already 86% accurate on
  owner speech; the loss is downstream. Rule B is measured evidence that tuning the
  aggregation is the wrong lever.
- **Not enabling live diarization.** It stays opt-in and off.

## Design (Stage 1)

Keep `dominant_channel_for_span` and the `channel` field exactly as they are (the
offline path and the persisted column depend on them). Add the timeline alongside.

1. **`audio/common.rs`** — new `ChannelRun { start: f64, end: f64, tag: ChannelTag }`
   in recording-relative seconds, with a compact wire shape (`{"s":..,"e":..,"c":".."}`)
   mirroring `word_timestamps`.
2. **`audio/pipeline.rs`** — new `channel_runs_for_span(windows, start_ms, end_ms) ->
   Vec<ChannelRun>`: the classified windows overlapping the span, silence windows
   dropped, adjacent same-class windows merged, clipped to the span. Pure, unit-tested
   next to `dominant_channel_for_span`.
3. **Plumbing** — `TranscriptionChunk` and `TranscriptUpdate` each gain
   `channel_runs`, populated at the existing `dispatch_segments` call site that already
   computes `channel` from the same windows. `#[serde(default)]` so nothing breaks.
4. **Frontend** — when a live row's runs contain an owner↔remote change, render it as
   consecutive parts, apportioning the text by run duration and snapping to whitespace
   (the same fallback `split.rs` uses when word timestamps are absent; the live event
   carries no word timings). Owner parts show "You"; other parts keep today's
   behavior (no name unless live diarization supplied one).

### Why the split is not done in the backend before emit

Emitting N `TranscriptUpdate`s per VAD segment would change `sequence_id` semantics,
the save payload, live de-dup, and the frontend's `sequence_id` correlation map — not
a small change, and Stage 2 makes the stored rows correct anyway. Stage 1 stays a
render-time concern.

## Acceptance

- `channel_runs_for_span` unit tests: single-class span, owner→remote handoff, silence
  gap merging, span clipping, empty history.
- A frontend test that a row whose runs go owner→remote renders two parts, the first
  labeled "You".
- The measured harness re-run shows Stage 1 changes **nothing** about persisted rows
  (`transcripts.channel` values byte-identical), confirming scope containment.
- DoD gate (`/check`) clean; no change to DER (`tests/diarization_tuning.rs`).

### Measured result (Stage 1, shipped logic)

Both `channel_runs_for_span` and `splitLiveRowByChannel` were re-implemented in the
Python harness and run end-to-end over the real capture:

| | Value |
|---|---|
| Rows the shipped logic splits | 111 / 346 |
| Source error before | **8.23%** (112.4 s / 1365.8 s) |
| Source error after | **3.68%** (50.2 s) |
| Remote speech mislabeled "You" before | **32.4 s** |
| Remote speech mislabeled "You" after | **8.2 s** |

A 55% cut in total source error and a **75% cut in the wrong-"You" case specifically** —
the one the owner notices most. The gap to rule C's idealized 2.72% is the cost of
proportional word apportioning plus the guards below; both shrink in Stage 2, where the
cut is word-exact against stored `word_timestamps`.

### Code-review outcomes

- **Short spurious owner runs (accepted, fixed).** On speakers, an inter-phrase dip in
  the system track can let normalized bleed classify one 600 ms window as `Microphone`.
  The whole-row vote used to absorb that; splitting would carve a "You" clip out of
  someone else's sentence — the regression 0043 W1.3 / 0047 W2 exist to prevent.
  `MIN_OWNER_PART_SECS` (0.75 s, the same value as the offline `MIN_SPLIT_PART_SECS`)
  demotes owner stretches below it. Deliberately asymmetric: a wrong "You" is worse
  than a missing one. Measured, this took total error 4.74% -> 3.68% and wrong-"You"
  12.0 s -> 8.2 s, on a broad plateau (0.7-1.0 s all land within 0.25 pt) rather than a
  tuned spike. Note the review framed this as a regression Stage 1 *introduced*; the
  measurement shows the opposite — Stage 1 already cut wrong-"You" from 32.4 s to
  12.0 s. The guard is an additional improvement, not a repair.
- **Silence-gap bias (measured, not changed).** `channel_runs_for_span` credits an
  unclassified gap entirely to the earlier run, which in principle over-credits the
  speaker before a handoff when words are apportioned by duration. Splitting the gap at
  its midpoint instead changes the measured result by **exactly zero** on the real
  capture: within a VAD speech segment there are essentially no silence windows to
  leave a gap, so the scenario does not arise in practice. Left as-is (contiguous
  tiling is the simpler invariant); revisit if Stage 2 finds real gaps.

## Watch-fors

- `channel_windows` is cleared on VAD resync (`SyncOutcome::Attached`) and capped at
  1024 entries; runs inherit those limits exactly as the single tag does today.
- Char-proportional apportioning is crude at the boundary. Stage 2's word-exact cut is
  the real answer; Stage 1 should not be judged on word placement.
- 19 rows in the sample are remote speech tagged `microphone`. Stage 1 makes those
  *partly* right (the owner-tagged prefix shrinks); it does not eliminate them.
