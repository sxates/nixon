# 0046 — Diarization wave 3: owner↔remote boundary attribution + word-exact splits

- **Status:** Implemented (pending owner smoke + DER) — WS1 + WS2 built via subagent-driven TDD; cargo check/clippy + diarization/split/transcription/db suites green; file-size ratchet ok; whole-branch review clean after fixing a critical WS2 word-timestamp origin bug (segment- vs recording-relative). Two owner/CI checks remain: (1) the `EVAL_PINNED_DER` ground-truth run (no local models in the build env), (2) manual smoke — record a real back-and-forth where you interject and the other person replies immediately; your words should be "You" and theirs theirs at the boundaries (both directions), with split text landing on whole words.
- **Owner agent(s):** audio-engineer (WS1 + WS2)
- **Roadmap phase:** Identity & Organization (follows 0044 boundary-accurate speakers)
- **Ships with:** the 0045 release (both merged to `main` locally, released together via `./release.sh`)

## Context / Problem

Owner dogfooding v1.11 still sees a **one-segment attribution lag at every owner↔remote
turn**, despite the 0044 boundary-accurate-speaker work. Reported pattern: Person A (a remote
speaker) talks → the owner speaks → **the owner's words are attributed to Person A** → Person
A replies → **the first line of A's reply is attributed to the owner ("You")** → then it
returns to correctly attributing A. A clean one-row lag at each handoff, in both directions.

**Root cause (traced, confirmed):** two mechanisms combine at a quick back-and-forth.

1. **Redemption-merged rows.** A transcript row ends only after the VAD redemption silence
   (`REDEMPTION_TIME_MS = 400` ms live — `audio/stt_stage.rs:19`; ~2000 ms batch). A reply
   faster than that **glues the previous speaker's tail and the next speaker's onset into one
   transcript row** (`diarization/split.rs:2-8` doc). This is the lag mechanism.

2. **A glued owner↔remote row carries a single channel tag and can't be split.** 0044 fixed
   this for **remote↔remote** boundaries by splitting a row at the boundary between two
   *system* diarization turns (`diarization/split.rs` `plan_split`). But the owner↔remote case
   is uncovered:
   - **Offline diarization runs on the system WAV only** (`diarization/pipeline.rs:586-655`),
     so the **owner produces no `SpeakerTurn`**. A glued `[owner + A]` row overlaps only *one*
     run (A's), so `plan_split` returns `None` (`split.rs:112-114`, needs ≥2 distinct runs) —
     no split.
   - `split.rs:236` also explicitly **skips `microphone`-tagged rows**.
   - The whole glued row then takes one label. `dominant_channel_for_span`
     (`audio/pipeline.rs:166`) tags the row by whichever *channel* (pre-mix mic vs system
     track) is louder across the span, and `align_turns_to_segments`
     (`diarization/align.rs:168`) labels it all-or-nothing: a `microphone` tag hard-returns
     the owner ("You", short-circuit `align.rs:176`); a `system`/`mixed` tag takes the
     max-overlap remote turn. So `[A tail + owner onset]` (system-dominated) → all "Person A"
     (owner's words mislabeled); `[owner tail + A onset]` (mic-dominated) → all "You" (A's
     first line mislabeled). Exactly the reported symptom.

   `pad_trimmed` (0044 W1.1, `align.rs:108`) does not help here — it only re-weights overlap
   between two competing *system* turns; it cannot rescue a row whose problem is a single
   channel tag with no owner turn to split against.

Separately, 0044's split apportions a split row's text **time-proportionally** (character
offset ∝ time, whitespace-snapped — `split.rs:128-176`), so even a correct split can land a
word or two off. The owner explicitly asked to also make splits **word-exact** by using the
per-word timestamps the STT engines already compute but currently discard.

**Owner-approved approach (this spec):** do both —
- **WS1** — make the **owner a first-class diarization speaker**: derive owner "turns" from
  the mic track offline and inject them into the turn set so the existing splitter cuts
  owner↔remote boundaries, re-tagging each split part's channel so both directions label
  correctly.
- **WS2** — **word-exact splits**: capture the per-word timestamps (already produced by
  Parakeet, extractable from Whisper), persist them per row, and snap split boundaries to real
  word edges instead of character proportions.

## Goals

- At every owner↔remote handoff, the owner's words are attributed to "You" and the remote's
  words to the remote speaker — in **both** directions, measured on a synthetic fixture and
  with no DER regression on the ground-truth harness.
- Split boundaries land on real word edges (word-exact), not time-proportional character
  offsets, when word timestamps are available; graceful fallback to the current char-snapping
  when they are not.
- Owner turns never inflate the remote-speaker cluster cap, get voiceprinted, or get
  gallery-matched (owner is already excluded from those paths — keep it that way).

## Non-goals

- Changing VAD redemption windows (0044 rejected this — trades STT quality for granularity;
  the split fixes attribution without touching STT chunking).
- The **mid-recording live-STT attach timeline offset** (0044 Risks): a separate,
  uncompensated global shift that only affects meetings where live STT attaches partway
  (Live→Defer→Live mid-meeting). It is **not** the reported symptom (which is a clean
  per-boundary lag, not a global shift), and fully-deferred low-power meetings rebuild their
  transcript offline from the WAVs on a shared origin, so it does not apply. Left as a tracked
  follow-up.
- Live-path word timestamps (WS2 targets the **batch/retranscription** path — deferred
  low-power meetings, which is where the owner's meetings are transcribed). Live word
  timestamps are a later extension.
- Re-clustering or changing the remote (system) diarization itself.

## Design

### WS1 — Owner as a first-class diarization speaker (audio-engineer)

**W1.1 — Resolve + VAD the mic WAV offline → owner turns.**
- Add `resolve_mic_wav(...)` mirroring `resolve_system_wav` (`diarization/pipeline.rs:159-203`)
  — reuse the existing `mic_channel_wav(folder)` resolver (`audio/channel_writer.rs:131`) and
  the same `folder_path` backfill.
- In the offline pass, after the system diarization returns `(turns, embeddings)`
  (`pipeline.rs:716-726`): decode the mic WAV (`audio::decoder::decode_audio_file` →
  `to_whisper_format()`, mono 16k) and run `audio::vad::get_speech_chunks_with_progress`
  (`audio/vad.rs:437`, the same VAD retranscription uses, `VAD_REDEMPTION_TIME_MS`). Convert
  each returned `SpeechSegment { start_timestamp_ms, end_timestamp_ms, .. }` to
  `SpeakerTurn { start: ms/1000.0, end: ms/1000.0, speaker: LOCAL_SPEAKER_KEY }`
  (`diarization/mod.rs:49`, `align.rs:35` `"local"`).
- **Timeline:** the mic WAV, the system WAV, and the (offline-rebuilt) transcript rows all
  share the recording-start origin, so owner turns align with the transcript rows without
  compensation. If the mic WAV is missing (older recording, mic-off), owner turns are empty and
  behavior is exactly today's — safe fallback.

**W1.2 — Inject owner turns; keep them out of clustering/embeddings.**
- Merge the owner turns into the `turns: Vec<SpeakerTurn>` **after** diarization and **before**
  both `split_straddling_rows(pool, &meeting_id, &turns)` (`pipeline.rs:743`) and
  `align_turns_to_segments(&turns, &alignable)` (`pipeline.rs:770`).
- Do **not** add owner turns to `embeddings` (the map `persist` iterates for voiceprints,
  `pipeline.rs:341-348`). Owner turns are injected *after* sherpa, so they never reach the
  cluster cap (`roster_speaker_cap`, `pipeline.rs:1132`) or the gallery matcher (owner already
  excluded at `pipeline.rs:955,976`). The `"local"` key already carries a NULL embedding
  everywhere, so all owner exclusions remain intact.

**W1.3 — Split owner↔remote rows AND re-tag each part's channel (the crux — fixes BOTH
directions).**
- Relax the mic guard at `split.rs:236`: drop the `channel == Some("microphone")` clause (keep
  the `overridden.contains(&id)` clause — override id-stability, specs/0019). With owner turns
  present, a glued owner↔remote row now overlaps ≥2 distinct runs (`"local"` + `"spk_N"`) and
  `plan_split` cuts it at the boundary; a *pure* owner row overlaps only the one `"local"` run →
  `plan_split` returns `None` → not split (still labeled "You" via the `align.rs:176`
  short-circuit). No change to `plan_split`'s run logic (it already keys on the speaker string,
  so `"local"` is a distinct run for free).
- **Per-part channel re-tagging (required for correctness).** Today `SplitPart { start, end,
  text }` (`split.rs:46`) and split parts inherit the *original* row's single channel — which
  is why a `microphone`-tagged `[owner tail + A onset]` row, even if split, would have its
  A-part inherit `microphone` and wrongly short-circuit to "You". Fix: `plan_split` already
  knows each part's owning run (its speaker). Add a `channel` to `SplitPart`, set from the run's
  speaker: `"local"` run → `"microphone"`, any `"spk_N"` run → `"system"`. Persist that per-part
  channel (`split_straddling_rows`: the first part UPDATEs the original row's `channel` too;
  later parts INSERT with their part channel). Then `load_segments` (`segments.rs:22`
  `channel_from_db`) re-reads the corrected channel and `align_turns_to_segments` labels each
  part correctly with **no change to `align.rs:176`**:
  - `[A tail + owner onset]` (system-tagged) → A-part `system`→overlap→Person A; owner-part
    `microphone`→"You". ✓
  - `[owner tail + A onset]` (mic-tagged) → owner-part `microphone`→"You"; A-part
    `system`→overlap→Person A. ✓
  Both directions fixed by construction (each part is single-speaker by the split boundary, so
  the channel derived from its run is correct).

**W1.4 — Idempotency & overrides.** Re-running diarization must be stable: a row already tight
to one run plans no split (`plan_split` → `None`). Rows with a manual speaker override are
still skipped (`overridden` clause retained). The split persistence reuses the existing
transaction + `AFTER UPDATE OF transcript` FTS trigger path from 0044 (same `transcripts.id`
for the first part; new uuids for later parts).

**Verification:** unit tests for owner-turn conversion and per-part channel tagging; an
integration fixture with a synthetic owner↔remote fast handoff in **both** directions (glued
row → split + owner-part "You", remote-part remote, for both a system-tagged and a
mic-tagged glued row); `diarization_tuning.rs` ground-truth run must not regress
`EVAL_PINNED_DER`.

### WS2 — Word-exact split apportioning (audio-engineer)

**W2.1 — Capture per-word timestamps (batch path; Parakeet primary).**
- Parakeet already returns `TimestampedResult { text, timestamps: Vec<f32>, tokens }` per token
  (`parakeet_engine/model.rs:23`), dropped at the provider boundary
  (`parakeet_engine/parakeet_engine.rs:501`, returns `result.text` only). Add a
  timestamp-preserving entry point (e.g. `transcribe_audio_timestamped(...) ->
  Result<TranscribedWords>` where `TranscribedWords { text: String, words: Vec<WordStamp> }`,
  `WordStamp { text: String, start: f32, end: f32 }`) that groups tokens into words on the
  space boundary (`model.rs:453` `DECODE_SPACE_RE`) and assigns each word `start` = first
  token ts, `end` = last token ts. Keep the existing text-only `transcribe_audio` for callers
  that don't need words (no signature churn on the hot live path or the STT-lock sites).
- Whisper (secondary): a matching path extracting per-token `t0/t1` via
  `full_get_token_data(i, j)` (currently unused; `whisper_engine.rs:786-818` reads only
  segment-level times into discarded `_start_time/_end_time`), grouped into words. Behind the
  same `TranscribedWords` shape. If Whisper token grouping proves unreliable, WS2 may ship
  Parakeet-only with a fallback for Whisper (see W2.4) — Parakeet is the default engine
  (`settings/commands.rs:283`).
- The **batch/retranscription** path (`audio/retranscription.rs` / `retranscription_channels.rs`,
  which transcribes the deferred low-power meetings that later get diarized) calls the
  timestamped entry point and carries the words through to persistence. Live path unchanged.

**W2.2 — Persist word timestamps per row.**
- Migration `…_add_transcript_word_timestamps.sql`: `ALTER TABLE transcripts ADD COLUMN
  word_timestamps TEXT` (nullable JSON array of `{w,s,e}`; forward-only, idempotent).
- Thread through `TranscriptSegment` (`transcripts.rs:17`) and `insert_segments`
  (`database/repositories/transcript.rs:33`). `create_transcript_segments`
  (`audio/common.rs:89`) gains the optional words alongside text. Old rows and word-less
  engines store `NULL`.

**W2.3 — Snap split boundaries to word edges.**
- In `plan_split` apportioning (`split.rs:128-176`), when the row has word timestamps, replace
  the char-proportional `target = text.len() * frac` + whitespace snap (`split.rs:134-146`)
  with: for each boundary time `b`, pick the word whose `[start,end]` brackets `b` (or the
  nearest word edge) and cut at that word's byte offset in the row text. Falls back to the
  existing char-proportional path when `word_timestamps` is absent (backward-compatible).
- `load_segments`/`split_straddling_rows` must load `word_timestamps` alongside the row so
  `plan_split` can use them.

**W2.4 — Fallback.** Any row without word timestamps (old data, mic-off, Whisper if deferred,
engine that didn't emit them) uses the existing 0044 char-proportional apportioning. WS2 is a
precision improvement layered on WS1, never a regression.

**Verification:** unit tests for token→word grouping (Parakeet), word-boundary snapping vs the
char-proportional fallback (same fixture, assert the split lands on the exact word edge with
timestamps and within ±1 word without), and the `word_timestamps` round-trip
(persist → load). DER harness non-regression.

### Data model

- `…_add_transcript_word_timestamps.sql` — `transcripts.word_timestamps TEXT` (nullable JSON;
  forward-only, idempotent). No other schema change. WS1 mutates rows within the existing
  schema (reuses 0044's split persistence).

### Tauri IPC

None. No new commands or events (diarization already emits `diarization-complete`).

## Tasks

1. [ ] **W1.1** `resolve_mic_wav` + offline mic-WAV decode + VAD → owner `SpeakerTurn`s
       (`"local"`); unit test for the SpeechSegment→turn conversion; missing-mic-WAV → empty
       turns. *(audio-engineer)*
2. [ ] **W1.2** Inject owner turns into `turns` after diarization, before split + align; keep
       them out of `embeddings`/clustering; unit/assert owner turns don't reach the cap or
       gallery. *(audio-engineer)*
3. [ ] **W1.3** `SplitPart.channel` derived from run speaker (local→microphone, spk_N→system);
       relax the `split.rs:236` mic clause (keep override clause); persist per-part channel
       (first part UPDATE, later parts INSERT); unit tests for per-part channel tagging.
       *(audio-engineer)*
4. [ ] **W1.4** Integration fixture: both-direction owner↔remote fast handoff (system-tagged
       and mic-tagged glued rows) → correct split + labels; idempotent re-run; override rows
       unsplit; `EVAL_PINNED_DER` non-regression. *(audio-engineer)*
5. [ ] **W2.1** Parakeet `transcribe_audio_timestamped` → `TranscribedWords` (token→word
       grouping); Whisper token-data extraction behind the same shape; unit tests for grouping.
       *(audio-engineer)*
6. [ ] **W2.2** Migration `word_timestamps` column + thread through `TranscriptSegment` /
       `create_transcript_segments` / `insert_segments`; batch/retranscription path populates
       it; round-trip test. *(audio-engineer)*
7. [ ] **W2.3** Word-edge snapping in `plan_split` apportioning with char-proportional
       fallback; load `word_timestamps` in `split_straddling_rows`/`load_segments`; unit tests
       (word-exact vs fallback). *(audio-engineer)*

## Acceptance criteria

- **Both-direction fix:** a synthetic fast-handoff fixture where (a) the owner interjects into
  a remote speaker's turn and (b) a remote speaker replies immediately after the owner — the
  merged rows are split and the owner's portion is "You" while the remote portion is the remote
  speaker, for both a system-tagged and a microphone-tagged glued row. A row with a manual
  override is never split.
- Owner turns do not change the remote-speaker cluster count, are never voiceprinted, and are
  never gallery-matched (owner exclusions intact).
- `EVAL_PINNED_DER` does not regress on the ground-truth samples.
- With word timestamps present, a split lands on the exact word boundary (unit-assertable);
  without them, the existing char-proportional split still runs (fallback proven).
- `word_timestamps` round-trips (persist → load → available in `plan_split`).
- Definition of Done per `/CLAUDE.md`: `cargo check`/`clippy`/`test` (incl. `--features metal`
  STT/diarization tests), `pnpm lint`/`test`, **file-size ratchet** (`scripts/check-file-size.sh`
  — run it in EVERY task's gate; likely new modules to stay under 800 lines: e.g.
  `diarization/owner_turns.rs`, `parakeet_engine/words.rs`). Known `vad_filter` failure exempt.

## Risks / open questions

- **Short interjections below `MIN_SPLIT_PART_SECS` (0.75s, `split.rs:37`) won't split** — a
  one- or two-word owner interjection glued into a remote row (or vice-versa) stays one label.
  This is strictly better than today (every boundary wrong) but not perfect. Consider a lower
  threshold for owner-involved splits (the channel signal is authoritative, unlike a
  diarization guess); left as a tuning knob, revisit if the owner still sees short-interjection
  drift.
- **Owner turns depend on the mic WAV existing.** Older recordings or mic-off meetings have no
  mic WAV → no owner turns → today's behavior (safe, no regression).
- **WS2 is the larger, schema-touching workstream.** It is layered strictly on top of WS1 and
  always falls back to char-proportional, so WS1 (the actual reported-bug fix) is independently
  complete and shippable even if WS2 needs iteration. Sequence WS1 (tasks 1–4) before WS2
  (tasks 5–7).
- **Whisper word grouping** may be less clean than Parakeet's (token DTW vs subword tokens);
  acceptable to ship WS2 Parakeet-first with Whisper falling back to char-proportional if its
  grouping is unreliable — Parakeet is the default engine.
- **Mid-attach timeline offset** (Non-goals) can still cause a *global* shift on the specific
  Live→Defer→Live mid-meeting case; out of scope here, tracked separately.

## Verification

- `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal` (owner-turn
  conversion, per-part channel tagging, token→word grouping, word-snap vs fallback,
  `word_timestamps` round-trip, split integration fixtures).
- Ground-truth: `cargo test --features metal --test diarization_tuning` (models present) —
  DER at or below pinned values.
- Manual smoke (owner): record a real back-and-forth where you interject and the other person
  replies immediately → after processing, your words are "You" and theirs are theirs at the
  turn boundaries (both directions), and split text lands on whole words.
