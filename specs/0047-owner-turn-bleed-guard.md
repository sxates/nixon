# 0047 — Owner-turn speaker-bleed guard

- **Status:** Done (pending release + owner smoke)
- **Owner agent(s):** audio-engineer
- **Roadmap phase:** Diarization accuracy (0046 follow-up)

## Context / Problem
v1.12.0 (specs/0046) made the owner's microphone a first-class diarization speaker by
re-running VAD on `mic.wav` and injecting every detected speech interval as a "You"
(`LOCAL_SPEAKER_KEY`) turn (`diarization/owner_turns.rs`). Those turns are then used by
both alignment (`align.rs`, via `pipeline.rs`) and the straddling-row splitter
(`split.rs`).

The generator never checked whether the audio it detected on the mic was actually the
owner. When the user listens on **laptop/external speakers (no headphones)**, remote
participants' voices travel speaker → air → mic. Because the mic path is EBU R128
normalized to −23 LUFS (`audio/pipeline.rs`), that bleed is boosted back to speech
loudness, VAD fires on it, and it becomes a "You" turn. Downstream:

1. `align_turns_to_segments` (max-overlap, Rule 2) now has `"local"` owner turns as
   candidates, so a genuine **System**-channel remote segment can be out-scored by the
   overlapping bleed turn and relabeled "You".
2. `split_straddling_rows` carves remote rows at the bleed turn and re-tags the overlap
   part as `"microphone"`, which then short-circuits to "You".

Net effect the owner reported: *"a lot of what's said in meetings is attributed to
you."* This is a regression introduced by 0046 — before it, no owner turn existed in the
turn set, so remote speech could never be captured as "You" this way.

The capture pipeline **already** solved this exact problem for its per-window channel
tags in specs/0043 W1.5 (`CHANNEL_BLEED_SYSTEM_RMS` in `audio/pipeline.rs`): a window
that looks mic-dominant but has the system track simultaneously at speech loudness is
demoted to `Mixed`, because "9.5 dB of RMS dominance carries no information about *whose*
voice it is." 0046 simply didn't apply that guard to the new owner-turn source.

## Goals
- Owner turns represent the **owner**, not remote audio bleeding into the mic.
- Reuse the existing, already-reasoned-through bleed logic rather than inventing a new
  threshold.
- Zero behavior change when there's no bleed (headphones), and a safe fallback when the
  system track is unavailable.

## Non-goals
- Real-time / live-path gating (this is the offline diarization pass at stop).
- Acoustic echo cancellation. We reject bleed intervals; we don't recover owner speech
  that genuinely overlapped remote speech (that resolves via diarized remote turns /
  Unknown, per specs/0043 W1.3).
- The separate "detect Zoom's in-app mute state" feature (still parked — muted-in-Zoom
  is the owner's own voice, which this guard neither targets nor suppresses).

## Approach
Add a bleed filter to owner-turn generation that reuses the capture pipeline's own
`classify_window_channel` + `dominant_channel_for_span` (specs/0029/0043). For each
candidate owner interval, window the time-aligned `mic.wav`/`system.wav` slices, classify
each window, and keep the interval only when its aggregate channel is `Microphone`.
Bleed aggregates to `Mixed` (mic-dominant echo under a loud system track) or `System` and
is dropped. Chosen over a bespoke align-only guard because it fixes the problem at the
source — killing bleed turns before they reach *both* align and split — with logic the
codebase already trusts.

## Design
- `diarization/owner_turns.rs`:
  - New pure `filter_bleed_owner_segments(segments, mic, sys, sample_rate)` +
    `owner_interval_is_genuine(..)` helper. Pure → unit-tested without a model/FS.
  - `owner_turns_for_meeting` now also decodes `system.wav` (best-effort) and filters the
    VAD segments before converting to turns, inside the existing `spawn_blocking`.
  - Missing/undecodable `system.wav` → empty slice → keep-all (pre-0047 behavior).

## Tasks
1. [x] Failing unit test: a bleed-only interval (mic echo + loud system) is dropped; a
   genuine owner interval (system silent) is kept; missing system keeps all.
2. [x] Implement the filter reusing `classify_window_channel`/`dominant_channel_for_span`.
3. [x] Wire `system.wav` decode + filter into `owner_turns_for_meeting`.

## Acceptance criteria
- `cargo test`/`clippy` clean; diarization suite green (owner-turn, split, align tests).
- On a speakers-recorded meeting, re-running diarization no longer attributes remote
  speech to "You"; genuine owner turns survive.

## Risks / open questions
- **Overlapped speech** (owner genuinely talks over remote) aggregates to `Mixed` and is
  dropped as an owner turn — accepted, consistent with specs/0043 W1.3 (ambiguity must not
  hard-label "You").
- **Data recovery:** re-diarizing (`api_diarize_meeting`) fixes labels on already-affected
  meetings; rows previously carved by `split.rs` won't auto-re-merge.
## W2 — owner turns must not win/carve non-mic rows (shipped v1.12.2)
The bleed guard (W1) suppressed most bleed owner turns, but the few that survived (partial
overlaps, brief remote pauses) could still corrupt remote rows, because owner turns were
injected into the SHARED turn set used by both alignment and splitting:
- **`align.rs`:** a System/Mixed segment took the greatest-overlap turn — including a
  `"local"` owner turn — so a surviving bleed turn could relabel remote speech "You".
- **`split.rs`:** a System/Mixed row straddling an owner turn was carved and the overlap
  re-tagged `"microphone"` → a "You" clip in the middle of someone else's turn.

Fix: owner turns now label ONLY the owner's own mic-tagged rows.
- `align_turns_to_segments` filters `LOCAL_SPEAKER_KEY` turns out of the max-overlap +
  nearest-turn contest for any non-Microphone segment (Microphone still short-circuits to
  "You").
- `split_straddling_rows` plans a non-mic row against remote turns only, so it can never
  gain a `"microphone"` part. Remote↔remote splitting (specs/0044) is unchanged.

## Verification
- `cd frontend/src-tauri && cargo test --features metal --lib owner_turns --lib diarization::align`
  and `--test diarization_split` (owner exclusion + remote↔remote guard). Full diarization
  suite green (146 lib + 14 integration).
- **Real-meeting eval** (`src/diarization/real_eval.rs`, `#[ignore]`; set `VINYL_EVAL_FOLDER`).
  On a real 51-minute meeting recorded on speakers (RODE NT-USB mic + USB DAC out): 346
  segments, 293 tagged `mixed`; the mic-VAD produced 175 owner intervals of which the W1
  bleed guard dropped **164 as bleed** (11 genuine). Segments labeled "You": **295 (pre-0047)
  → 51 (0047)** — the 51 being the genuinely mic-dominant ones; **244 remote segments
  (70.5%) rescued** from a wrong "You". (Note: remote *speaker separation* is a distinct,
  pre-existing over-clustering axis, spec 0039 — out of scope here.)
- Manual owner smoke on a speakers-recorded meeting: record → stop → confirm remote turns
  keep their speaker.
