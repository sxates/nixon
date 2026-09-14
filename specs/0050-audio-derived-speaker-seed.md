# 0050 — Audio-derived speaker-count seed

- **Status:** In progress (implementing) — DER-gated
- **Owner agent(s):** audio-engineer
- **Roadmap phase:** Diarization accuracy (0039/0048 follow-through)

## Context / Problem
The `2026-08-09` DER review (`docs/reviews/2026-08-09-diarization-der-eval.md`) established that
clustering is already near-optimal (~8% macro DER) and the whole lever for the owner-visible
"noisy separation" is the **`AtMost(n)` speaker-count seed** — but the seed came from the
calendar **invite**, which is wrong two ways (owner-reported):

- **Distribution-list invite under-counts.** A DL is one attendee entry for many people →
  `AtMost(1)`. Because `AtMost` is applied as a cap, this **force-merges** real speakers.
  Simulated on the labelled 20-speaker meeting: `AtMost(1)` → **55.58% DER** (over half the
  meeting misattributed). This is the harmful case.
- **Big optional invite over-counts.** 50 invited, 5 speak → `AtMost(50)`. Benign (the ceiling
  never binds) but useless — the meeting reverts to Auto's cluster sprawl.

## Goals
- Seed the cap from the **audio** (who actually spoke), not the invite. Robust to both failures.
- No clustering change (that was ruled out in the review as unsafe). DER must not regress.

## Approach
`n_audio` = number of clusters whose **pooled speech ≥ `N_AUDIO_MIN_SECS`** — who spoke enough to
be a real speaker, independent of the invite. Cap at **`AtMost(min(n_audio, clean_invite))`**: the
audio does the work; a *clean* invite (no distribution list) is only a sanity **ceiling** (a true
upper bound — no one outside it speaks), guarding against `n_audio` over-counting on noisy audio,
while never under-capping (the harmful direction). `Fixed(n)` (manual override) still wins.

**Proven on the ground-truth DER harness** (`eval_seed_simulation`): at 10 s, `n_audio` = 2 / 19 /
4 vs true 2 / 20 / 4; `AtMost(n_audio)` lands **5.73 / 8.39 / 10.10%** — matching the AtMost(true-n)
oracle on all three files, rescuing the DL disaster (55.58 → 8.39) and the sprawl (29 → 5 on the
4-speaker meeting). Robust across 5–20 s thresholds.

## Design
- `sherpa.rs`: `pub const N_AUDIO_MIN_SECS: f32 = 10.0`; `pub fn estimate_speakers_by_duration`.
  `diarize_with_embeddings` offline path = consolidate with the **Auto budget** (the invite is a
  ceiling now, not a consolidation floor) → `AtMost(min(n_audio, invite))`. Fixed(n) and the live
  windowed pass are untouched.
- `settings.rs` `resolve_speaker_count`: produce the **clean invite ceiling** — if any attendee
  `is_distribution_list`, the count is unreliable → `Auto` (no ceiling; audio seeds it); else
  `AtMost(clean remote count)`; else `Auto`. **Reverts specs/0048's default `AtMost(10)`** — under
  self-seeding a fixed default cap is harmful (it would cap a genuine 20-person ad-hoc meeting
  below `n_audio`); `n_audio` supersedes it. (`Attendee` has no resource/response-status fields, so
  room/decline filtering is deferred.)
- `tests/diarization_tuning.rs`: `eval_regression_gate` re-pointed at the audio-seed path
  (`apply_audio_seed`), using the production `estimate_speakers_by_duration`.

## Acceptance criteria
- `eval_regression_gate` (audio-seed config) green: per-file DER under the pinned ceilings.
- `eval_seed_simulation` shows `AtMost(n_audio)` ≈ oracle and both invite failures rescued.
- `cargo`/`clippy`/`pnpm` + file-size gates clean.

## Risks / open questions
- `N_AUDIO_MIN_SECS` (10 s) is empirical on 3 files — robust across 5–20 s, erring low is safe (a
  high estimate just leaves a non-binding ceiling). Widen `zoom-samples/` (a real DL meeting, a
  big-invite-few-speakers meeting) to broaden coverage.
- EventKit can't flag distribution lists (always `false`); Google Calendar can. EventKit-sourced DL
  meetings therefore rely on `n_audio` alone (still correct — the invite just isn't used as a tight
  ceiling), not an explicit DL flag.

## Verification
`cd frontend/src-tauri && VINYL_DIARIZATION_PROVIDER=cpu cargo test --features metal --test
diarization_tuning eval_regression_gate -- --ignored --nocapture` (+ `eval_seed_simulation`).
Manual smoke: re-diarize a big-invite meeting and a DL meeting; confirm the speaker count tracks
who spoke, not the invite.
