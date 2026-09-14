# 0048 — Default speaker-count cap for unknown-size meetings

- **Status:** Shipped v1.12.3 — **superseded by specs/0050** (audio-derived cap). The fixed
  `AtMost(10)` default was a stopgap; it wrongly caps a genuine large ad-hoc meeting below the
  real speaker count. 0050 caps at `AtMost(n_audio)` from the audio instead and reverts this.
- **Owner agent(s):** audio-engineer
- **Roadmap phase:** Diarization accuracy (0039/0041 follow-through)

## Context / Problem
Diarization resolves a speaker-count seed by precedence (specs/0011/0017): manual
"expected speakers" override → participant roster → calendar attendees → **`Auto`**. The
whole `AtMost(n)` cap + consolidation machinery already works when a seed exists. The gap:
an **ad-hoc meeting** (no manual override, empty roster, no linked calendar event) falls to
uncapped `Auto`, which over-clusters unbounded on hard/long calls. Measured on a real
51-minute, ~6-speaker meeting recorded on speakers, `Auto` produced **81 clusters** (raw 133
→ 81 after spec-0039 consolidation). There is an over-cluster *warning* at >30 clusters, but
it only logs — nothing caps the run.

## Goals
- Stop unknown-size meetings from over-clustering unbounded, with no risk to meetings that
  already resolve a seed and no risk of force-splitting small meetings.

## Non-goals
- Re-tuning the Auto clustering threshold / consolidation floor (separate, higher-risk).
- Detecting live participant count (Zoom monitor has none; deferred).

## Approach
Replace the final `Auto` fallthrough in the **pure** `resolve_speaker_count`
(`diarization/settings.rs`) with a default `AtMost(DEFAULT_MEETING_SPEAKER_CAP)` ceiling and a
new `SpeakerCountSource::Default`. `AtMost(n)` only merges clusters *down* when Auto exceeds
`n` (folding a genuine overflow into the "unknown" bucket, top-n by talk time), so meetings
with ≤ cap real speakers are untouched — it is a pure safety ceiling. The per-meeting
"expected speakers" override (precedence branch 1) still refines it exactly.

`DEFAULT_MEETING_SPEAKER_CAP = 10` — generous enough that ordinary calls never hit it, low
enough to bound the runaway. **Proven on the real meeting:** `AtMost(6)` collapsed the 81 to
7 (≈ the ground-truth 6); a default `AtMost(10)` bounds any unknown meeting to ≤10 named
clusters.

## Design
- `diarization/settings.rs`: `pub const DEFAULT_MEETING_SPEAKER_CAP: u32 = 10`; new
  `SpeakerCountSource::Default` (wire `"default"`); `resolve_speaker_count` step 3 returns
  `(AtMost(cap), Default)`. Pure, unit-tested.
- Consumers get the new source: `diarization/pipeline.rs` (`resolve_meeting_speaker_count`
  log) and `audio/recording_commands.rs` (live path log) gain a `Default` match arm.
- Frontend `hooks/useDiarization.ts`: `speakerCountSource` union adds `'default'`; the
  "Speakers identified" toast shows "Capped at up to N (set an expected count to refine)".

## Acceptance criteria
- `resolve_speaker_count` with no manual/roster/calendar → `AtMost(10)` / `Default` (unit
  tests). Existing manual/calendar precedence unchanged. `cargo`/`clippy`/`pnpm` gates green.
- On the real two-channel sample meeting, re-diarizing under the default cap yields ≤10 clusters
  (was 81).

## Risks / open questions
- A genuine >10-distinct-speaker meeting with no seed is capped to 10 (overflow → "unknown").
  Rare; recovery is the per-meeting override / merge UI. Accepted.
- Cap value is a prior; `10` chosen from the real-meeting evidence. Tunable constant.

## Verification
`cd frontend/src-tauri && cargo test --features metal --lib diarization::settings`. Real-data:
`VINYL_EVAL_ATMOST=10 … cargo test --lib diarization::real_eval -- --ignored --nocapture`.
