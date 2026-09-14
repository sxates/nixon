# Diarization Owner-Boundary + Word-Exact Splits (spec 0046) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the persistent one-segment attribution lag at owner↔remote turns by making the owner's mic a first-class diarization speaker (WS1), and make every split land on real word edges (WS2).

**Architecture:** Offline diarization currently produces speaker turns from the *system* WAV only, so the owner has no turn and the 0044 row-splitter can't cut owner↔remote boundaries. WS1 derives owner "turns" from the mic WAV via the existing VAD, injects them into the turn set, and re-tags each split part's channel from its run so both directions label correctly. WS2 captures the per-word timestamps Parakeet/Whisper already compute, persists them, and snaps split boundaries to word edges (falling back to today's char-proportional apportioning).

**Tech Stack:** Rust (Tauri backend), `sqlx`/SQLite migrations, Parakeet + whisper-rs STT, `cargo test --features metal`. Pure planning logic is unit-tested without a DB or model.

## Global Constraints

- **File-size ratchet (spec 0042):** no production source file over **800 lines** unless allowlisted; allowlisted files may only shrink. Run `bash scripts/check-file-size.sh` (from repo root) in **every task's gate** — this is not optional, and it applies to Rust too (a prior batch shipped a ratchet violation by omitting it). `pipeline.rs` is large — put new logic in NEW modules (`diarization/owner_turns.rs`, and for WS2 a `parakeet_engine/words.rs` / `whisper_engine/words.rs` and split-apportioning helpers) rather than growing existing files.
- **Definition of Done (`/CLAUDE.md`):** `cargo check`, `cargo clippy --features metal`, `cargo test --features metal` clean in `frontend/src-tauri`; the diarization ground-truth `EVAL_PINNED_DER` must not regress.
- **Known-failing test stays exempt:** the `vad_filter` adversarial-fixtures test on macOS 15.x is pre-existing — not a regression.
- **cargo env:** `cargo` is not on the default PATH — run `source ~/.cargo/env` first. First `--features metal` compile is several minutes; don't abort it.
- **cargo fmt is NOT clean repo-wide** — format only files you touch.
- **Rust conventions:** `anyhow::Result`; existing owner exclusions (voiceprint/gallery/cap) must stay intact — owner turns are injected AFTER diarization and never added to `embeddings`.
- **Owner identity key:** `LOCAL_SPEAKER_KEY = "local"` (`diarization/align.rs:35`) → displayed "You" (`diarization/pipeline.rs:250`). Channel strings: `"microphone"`, `"system"` (`diarization/segments.rs:22` maps DB → `Channel`).
- **Commit after each task's gate passes.** Branch is `0046-diarization-owner-boundary` (already created off `main`).
- Sequence: **WS1 (Tasks 1–4) before WS2 (Tasks 5–7).** WS1 is the reported-bug fix and is independently shippable; WS2 layers on top with a fallback.

---

## File Structure

**Create:**
- `frontend/src-tauri/src/diarization/owner_turns.rs` — resolve mic WAV, VAD it, convert speech intervals → owner `SpeakerTurn`s (`"local"`). Pure conversion fn unit-tested.
- `frontend/src-tauri/src/parakeet_engine/words.rs` (WS2) — token→word grouping for Parakeet's `TimestampedResult`.
- `frontend/src-tauri/migrations/<ts>_add_transcript_word_timestamps.sql` (WS2) — `word_timestamps` column.

**Modify:**
- `diarization/mod.rs` — declare `pub mod owner_turns;`.
- `diarization/pipeline.rs` — resolve mic WAV + inject owner turns after diarization (make `turns` mutable), before split + align. Add `resolve_mic_wav`.
- `diarization/split.rs` — `SplitPart.channel`; per-part channel from run speaker; owner-aware mic-row split gating; persist per-part channel; (WS2) word-edge apportioning + load `word_timestamps`.
- `parakeet_engine/parakeet_engine.rs` (WS2) — `transcribe_audio_timestamped` returning words.
- `parakeet_engine/model.rs` (WS2) — expose token grouping if needed (or keep in `words.rs`).
- `whisper_engine/whisper_engine.rs` (WS2) — token-data word extraction (secondary).
- `audio/common.rs` (WS2) — `create_transcript_segments` carries optional words.
- `transcripts.rs` (WS2) — `TranscriptSegment.word_timestamps`.
- `database/repositories/transcript.rs` (WS2) — persist/load `word_timestamps`.
- `audio/retranscription.rs` / `retranscription_channels.rs` (WS2) — batch path captures words.

---

## Task 1: Owner turns from the mic WAV (pure conversion + module)

**Files:**
- Create: `frontend/src-tauri/src/diarization/owner_turns.rs`
- Modify: `frontend/src-tauri/src/diarization/mod.rs` (add `pub mod owner_turns;`)

**Interfaces:**
- Consumes: `SpeakerTurn` (`diarization/mod.rs:49`), `LOCAL_SPEAKER_KEY` (`align.rs:35`), `audio::vad::SpeechSegment` (`audio/vad.rs:71`).
- Produces: `speech_segments_to_owner_turns(segments: &[crate::audio::vad::SpeechSegment]) -> Vec<SpeakerTurn>`; and `async fn owner_turns_for_meeting(app, meeting_folder: &Path) -> Vec<SpeakerTurn>` (resolves + decodes + VADs the mic WAV; returns empty on any miss).

- [ ] **Step 1: Write the failing test** (pure conversion)

```rust
// in owner_turns.rs #[cfg(test)] mod tests
use super::speech_segments_to_owner_turns;
use crate::audio::vad::SpeechSegment;
use crate::diarization::align::LOCAL_SPEAKER_KEY;

fn seg(start_ms: f64, end_ms: f64) -> SpeechSegment {
    SpeechSegment { samples: vec![], start_timestamp_ms: start_ms, end_timestamp_ms: end_ms, confidence: 1.0 }
}

#[test]
fn converts_ms_intervals_to_local_turns_in_seconds() {
    let turns = speech_segments_to_owner_turns(&[seg(0.0, 1500.0), seg(3400.0, 6300.0)]);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].speaker, LOCAL_SPEAKER_KEY);
    assert!((turns[0].start - 0.0).abs() < 1e-6 && (turns[0].end - 1.5).abs() < 1e-6);
    assert!((turns[1].start - 3.4).abs() < 1e-6 && (turns[1].end - 6.3).abs() < 1e-6);
}

#[test]
fn empty_segments_yield_no_turns() {
    assert!(speech_segments_to_owner_turns(&[]).is_empty());
}
```

- [ ] **Step 2: Run it to see it fail** — `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal owner_turns` → FAIL (module/fn missing).

- [ ] **Step 3: Implement**

```rust
//! Owner (microphone) turns for diarization (spec 0046 WS1).
//!
//! Offline diarization runs on the system WAV only, so the owner produces no
//! SpeakerTurn and the 0044 row-splitter can't cut owner↔remote boundaries.
//! Here we derive owner turns from the mic WAV using the same VAD the batch
//! transcription uses, tagged with LOCAL_SPEAKER_KEY. These are injected into the
//! turn set (never into embeddings/clustering — the owner is excluded from
//! voiceprints/gallery/cap elsewhere).

use std::path::Path;
use tauri::{AppHandle, Runtime};

use crate::audio::vad::SpeechSegment;
use crate::diarization::align::LOCAL_SPEAKER_KEY;
use crate::diarization::SpeakerTurn;

/// Pure: VAD speech intervals (ms) → owner turns (recording-relative seconds).
pub fn speech_segments_to_owner_turns(segments: &[SpeechSegment]) -> Vec<SpeakerTurn> {
    segments
        .iter()
        .map(|s| SpeakerTurn {
            start: (s.start_timestamp_ms / 1000.0) as f32,
            end: (s.end_timestamp_ms / 1000.0) as f32,
            speaker: LOCAL_SPEAKER_KEY.to_string(),
        })
        .collect()
}

/// Resolve the mic WAV for this meeting folder, decode it, VAD it, and return
/// owner turns. Returns an EMPTY vec on any problem (missing mic WAV, decode
/// error) so diarization behaves exactly as before when the owner track is
/// unavailable — never fatal.
pub async fn owner_turns_for_meeting<R: Runtime>(
    _app: &AppHandle<R>,
    meeting_folder: &Path,
) -> Vec<SpeakerTurn> {
    let mic_wav = crate::audio::channel_writer::mic_channel_wav(meeting_folder);
    if !mic_wav.exists() {
        log::info!("owner turns: no mic WAV at {mic_wav:?} — skipping owner track");
        return Vec::new();
    }
    // Decode → mono 16k, then VAD with the batch redemption window.
    let decoded = match crate::audio::decoder::decode_audio_file(&mic_wav) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("owner turns: mic WAV decode failed ({e:#}) — skipping owner track");
            return Vec::new();
        }
    };
    let samples = decoded.to_whisper_format();
    // Run the blocking VAD off the async runtime.
    let segments = tokio::task::spawn_blocking(move || {
        crate::audio::vad::get_speech_chunks(&samples, crate::audio::retranscription::VAD_REDEMPTION_TIME_MS)
    })
    .await;
    match segments {
        Ok(Ok(segs)) => {
            let turns = speech_segments_to_owner_turns(&segs);
            log::info!("owner turns: {} owner speech interval(s) from mic WAV", turns.len());
            turns
        }
        Ok(Err(e)) => {
            log::warn!("owner turns: VAD failed ({e:#}) — skipping owner track");
            Vec::new()
        }
        Err(e) => {
            log::warn!("owner turns: VAD task panicked ({e}) — skipping owner track");
            Vec::new()
        }
    }
}
```

> Verify while implementing: `decode_audio_file`'s return type and `to_whisper_format()` (used at `pipeline.rs:586`); `get_speech_chunks(samples, redemption_ms)` (`audio/vad.rs:428`); `VAD_REDEMPTION_TIME_MS` visibility in `audio/retranscription.rs` (make it `pub const` if it isn't). `mic_channel_wav` is at `audio/channel_writer.rs:131` (re-exported `audio/mod.rs:103`). If `SpeechSegment` fields differ, match the real struct.

- [ ] **Step 4: Run tests** — `cargo test --features metal owner_turns` → PASS (2 tests).

- [ ] **Step 5: File-size + commit**

```bash
bash ../../scripts/check-file-size.sh   # from src-tauri: adjust to repo root; must pass
git add frontend/src-tauri/src/diarization/owner_turns.rs frontend/src-tauri/src/diarization/mod.rs
git commit -m "feat(0046): owner turns from mic-WAV VAD (WS1 W1.1)"
```

---

## Task 2: Inject owner turns into the offline pass

**Files:**
- Modify: `frontend/src-tauri/src/diarization/pipeline.rs` (add `resolve_mic_wav`; inject after diarization at ~line 726, before split at 743 and align at 770)

**Interfaces:**
- Consumes: `owner_turns_for_meeting` (Task 1); the existing `resolve_system_wav` pattern (`pipeline.rs:159-203`).
- Produces: owner turns present in the `turns` vec passed to `split_straddling_rows` and `align_turns_to_segments`.

- [ ] **Step 1: Make `turns` mutable and inject** — after the `spawn_blocking(diarize_audio_blocking)` result (`pipeline.rs:716-726`), change `let (turns, embeddings)` to `let (mut turns, embeddings)`, then inject owner turns before the split call (`pipeline.rs:743`):

```rust
    // spec 0046 W1.2: the owner (mic) has no system-diarization turn, so a merged
    // owner↔remote row can't be split. Derive owner turns from the mic WAV and add
    // them to the turn set (NOT to embeddings/clustering — owner stays excluded from
    // voiceprints/gallery/cap). Empty when the mic WAV is absent (safe no-op).
    let meeting_folder = resolve_meeting_folder(pool, &meeting_id).await; // see note
    if let Some(folder) = meeting_folder.as_deref() {
        let owner_turns = crate::diarization::owner_turns::owner_turns_for_meeting(&app, folder).await;
        if !owner_turns.is_empty() {
            turns.extend(owner_turns);
            turns.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
            log::info!("Diarization: injected owner turns; {} total turns for {meeting_id}", turns.len());
        }
    }
```

> `resolve_system_wav` (`pipeline.rs:159-203`) already backfills and returns the system WAV path; it derives the folder from the meeting. Reuse its folder-resolution logic: either factor out a `resolve_meeting_folder` returning the folder `PathBuf`, or call `mic_channel_wav(folder)` where `folder` is the parent of the resolved system WAV (`system_wav.parent()`). Pick whichever is the smaller change after reading `resolve_system_wav`; the mic and system WAVs are siblings in the meeting folder (`channel_writer.rs:125/131`). Keep this out of `pipeline.rs` bloat if it pushes the file over 800 — a thin `resolve_mic_wav` mirroring `resolve_system_wav` is fine.

- [ ] **Step 2: Gate — confirm owner turns don't reach clustering** — verify by reading that `embeddings` is unchanged (owner turns are only added to `turns`, never `embeddings`), and `diarize_audio_blocking` (which feeds the sherpa cap) already returned before this injection. No test needed beyond the integration fixture in Task 4, but add a `debug_assert` or log that `embeddings` has no `"local"` key.

- [ ] **Step 3: Build + clippy**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo build --features metal 2>&1 | tail -5 && cargo clippy --features metal 2>&1 | tail -5`
Expected: compiles; clippy clean.

- [ ] **Step 4: File-size + commit**

```bash
bash scripts/check-file-size.sh   # repo root
git add frontend/src-tauri/src/diarization/pipeline.rs
git commit -m "feat(0046): inject owner turns into offline diarization pass (WS1 W1.2)"
```

---

## Task 3: Per-part channel re-tagging + owner-aware mic-row splitting

The crux: each split part gets a channel derived from its run's speaker so BOTH directions label correctly, with a guard that a mic-tagged row never loses its owner attribution.

**Files:**
- Modify: `frontend/src-tauri/src/diarization/split.rs`

**Interfaces:**
- Consumes: `LOCAL_SPEAKER_KEY` (`align.rs:35`).
- Produces: `SplitPart { start, end, text, channel: Option<String> }`; `plan_split` returns parts with per-run channel; `split_straddling_rows` persists per-part channel and only splits a mic row when the plan yields an owner part.

- [ ] **Step 1: Write failing tests** (add to `split.rs` tests)

```rust
fn channel_of(part: &SplitPart) -> &str { part.channel.as_deref().unwrap_or("system") }

#[test]
fn owner_run_part_is_tagged_microphone_and_system_run_part_system() {
    // [Person A tail (spk_0) + owner onset (local)] glued into one row.
    let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "local")];
    let parts = plan_split(0.0, 6.7, TEXT, &turns).expect("should split");
    assert_eq!(parts.len(), 2);
    assert_eq!(channel_of(&parts[0]), "system");     // spk_0 part
    assert_eq!(channel_of(&parts[1]), "microphone"); // owner part
}

#[test]
fn owner_first_then_remote_tags_each_part_by_run() {
    // [owner tail (local) + Person A onset (spk_0)].
    let turns = vec![turn(0.0, 3.0, "local"), turn(3.4, 6.3, "spk_0")];
    let parts = plan_split(0.0, 6.7, TEXT, &turns).expect("should split");
    assert_eq!(channel_of(&parts[0]), "microphone");
    assert_eq!(channel_of(&parts[1]), "system");
}

#[test]
fn system_only_split_parts_are_all_system() {
    let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
    let parts = plan_split(0.0, 6.7, TEXT, &turns).expect("should split");
    assert!(parts.iter().all(|p| channel_of(p) == "system"));
}
```

- [ ] **Step 2: Run → fail** — `cargo test --features metal --lib diarization::split` → FAIL (`SplitPart` has no `channel`).

- [ ] **Step 3: Add `channel` to `SplitPart` and derive it per run** — in `plan_split`, track each kept run's speaker and set each part's channel. Add the field (`split.rs:47`):

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct SplitPart {
    pub start: f32,
    pub end: f32,
    pub text: String,
    /// Channel for this part, derived from its run's speaker: the owner
    /// ("local") run → "microphone"; any system run ("spk_N") → "system".
    /// So a split part is re-tagged to match the speaker who actually owns it,
    /// which is what lets align.rs label both sides of an owner↔remote boundary
    /// correctly (spec 0046 W1.3).
    pub channel: Option<String>,
}
```

Add a helper and thread the kept-run speakers into part assembly. Parts are built in kept-run order (part `i` ↔ `kept[i]`), so capture `kept[i].speaker` before consuming `kept` for boundaries, then set each `SplitPart.channel`:

```rust
fn channel_for_speaker(speaker: &str) -> Option<String> {
    Some(if speaker == crate::diarization::align::LOCAL_SPEAKER_KEY {
        "microphone".to_string()
    } else {
        "system".to_string()
    })
}
```

In `plan_split`, after computing `kept` (before it is used for boundaries), collect `let part_channels: Vec<Option<String>> = kept.iter().map(|r| channel_for_speaker(&r.speaker)).collect();`. Then in the assembly loop (`split.rs:150-176`), set `channel: part_channels[i].clone()` for each of the `part_channels.len()` parts (part `i` for `i` in `0..boundaries.len()` plus the tail part = index `boundaries.len()`). Update every existing `SplitPart { .. }` literal in the tests to include `channel: Some("system".into())` (or the expected value) so they still compile.

- [ ] **Step 4: Run the new + existing split tests → pass** — `cargo test --features metal --lib diarization::split`. Update the pre-existing tests' `SplitPart` literals / `assert_eq!(parts[..].text, ..)` to also account for the new field (they compare `.text`, `.start`, `.end`, so add `channel` only where a whole `SplitPart` is constructed in a test).

- [ ] **Step 5: Persist per-part channel + owner-aware mic gating in `split_straddling_rows`** — three edits:

(a) Relax the mic-skip **only when owner turns are present** and guard mic rows so they never lose owner attribution. Replace the skip at `split.rs:236`:

```rust
    let has_owner_turns = turns.iter().any(|t| t.speaker == crate::diarization::align::LOCAL_SPEAKER_KEY);
    // ... inside the loop:
        if overridden.contains(&id) {
            continue; // override id-stability (specs/0019)
        }
        let is_mic_row = channel.as_deref() == Some("microphone");
        if is_mic_row && !has_owner_turns {
            continue; // no owner timeline to split against → keep today's behavior
        }
        if let Some(parts) = plan_split(start as f32, end as f32, &text, turns) {
            // A mic-tagged row must retain the owner: only split it if the plan
            // yields at least one owner ("microphone") part; otherwise a bad RMS
            // tag could re-attribute the owner's words to a remote speaker.
            if is_mic_row && !parts.iter().any(|p| p.channel.as_deref() == Some("microphone")) {
                continue;
            }
            planned.push((id, timestamp, parts));  // note: original `channel` no longer needed
        }
```

(b) First-part UPDATE now also writes the part channel (`split.rs:250-261`):

```rust
        sqlx::query(
            "UPDATE transcripts SET transcript = ?, audio_end_time = ?, duration = ?, channel = ? WHERE id = ?",
        )
        .bind(&first.text)
        .bind(first.end as f64)
        .bind((first.end - first.start) as f64)
        .bind(&first.channel)
        .bind(id)
```

(c) Later-part INSERT binds the PART's channel instead of the original row's (`split.rs:277`): `.bind(&part.channel)`.

Update the `planned` tuple type to drop the now-unused original `channel` (it was only used for the INSERT bind).

- [ ] **Step 6: Build + clippy + full diarization test subset**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo clippy --features metal 2>&1 | tail -5 && cargo test --features metal --lib diarization 2>&1 | tail -20`
Expected: clean; all `diarization::split` + `diarization::align` unit tests pass.

- [ ] **Step 7: File-size + commit**

```bash
bash scripts/check-file-size.sh
git add frontend/src-tauri/src/diarization/split.rs
git commit -m "feat(0046): per-part channel re-tag + owner-aware mic-row split (WS1 W1.3)"
```

---

## Task 4: Both-direction integration fixture + DER non-regression

**Files:**
- Modify/Create: a diarization integration test (follow the 0044 pattern — check `frontend/src-tauri/tests/` and any existing `diarization` integration test, e.g. where 0044 W1.2's fixture lives; if split integration is unit-level in `split.rs`, add a DB-backed test in the existing diarization test module).

**Interfaces:**
- Consumes: `split_straddling_rows`, `align_turns_to_segments`, an in-memory sqlx pool seeded with transcript rows.

- [ ] **Step 1: Write the failing integration test** — seed a meeting with two glued rows and owner turns, run `split_straddling_rows` then load+align, assert labels. (Adapt to the repo's test harness for a sqlx pool + `transcripts` schema; mirror how 0044's split test seeds rows.)

```rust
// Pseudocode-precise: seed rows, inject owner + system turns, split, align, assert.
// Row A (system-tagged): "...A tail... my onset..." spanning spk_0 then local.
// Row B (microphone-tagged): "...my tail... A onset..." spanning local then spk_0.
// After split_straddling_rows + load_segments + align_turns_to_segments:
//   Row A → 2 rows: system part key == "spk_0", owner part key == LOCAL_SPEAKER_KEY.
//   Row B → 2 rows: owner part key == LOCAL_SPEAKER_KEY, system part key == "spk_0".
// Assert both directions; assert an override'd row is left whole.
```

Write it concretely against the real harness (seed via `insert_segments` or direct `INSERT`; build turns with `SpeakerTurn`). Assert the four resulting parts' aligned keys.

- [ ] **Step 2: Run → confirm it passes** (the logic landed in Tasks 1–3; this test proves the end-to-end DB path). If it fails, debug against Task 3 — do not add new logic here.

- [ ] **Step 3: DER non-regression**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal --test diarization_tuning 2>&1 | tail -20`
Expected: `EVAL_PINNED_DER` at or below pinned values (test skips cleanly if models absent — note in the report if skipped).

- [ ] **Step 4: File-size + commit**

```bash
bash scripts/check-file-size.sh
git add frontend/src-tauri/tests/  # or the modified test file
git commit -m "test(0046): both-direction owner↔remote split integration + DER check (WS1 W1.4)"
```

---

## Task 5: Parakeet token→word timestamps (WS2 capture)

**Files:**
- Create: `frontend/src-tauri/src/parakeet_engine/words.rs` (token→word grouping)
- Modify: `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs` (add `transcribe_audio_timestamped`)
- Modify: `frontend/src-tauri/src/parakeet_engine/mod.rs` (declare `words`; export `WordStamp`/`TranscribedWords`)

**Interfaces:**
- Consumes: `TimestampedResult { text, timestamps: Vec<f32>, tokens: Vec<String> }` (`parakeet_engine/model.rs:23`), `DECODE_SPACE_RE` grouping (`model.rs:453`).
- Produces: `WordStamp { text: String, start: f32, end: f32 }`; `TranscribedWords { text: String, words: Vec<WordStamp> }`; `group_tokens_to_words(tokens: &[String], timestamps: &[f32]) -> Vec<WordStamp>`; `ParakeetEngine::transcribe_audio_timestamped(&self, audio: Vec<f32>) -> Result<TranscribedWords>`.

- [ ] **Step 1: Write failing tests** (pure grouping)

```rust
// words.rs tests
use super::{group_tokens_to_words, WordStamp};

#[test]
fn groups_subword_tokens_into_words_on_space_prefix() {
    // Parakeet uses a leading-space marker ("▁"); adapt to the real marker from DECODE_SPACE_RE.
    let tokens = vec!["▁he".into(), "llo".into(), "▁world".into()];
    let ts = vec![0.0f32, 0.1, 0.5];
    let words = group_tokens_to_words(&tokens, &ts);
    assert_eq!(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), vec!["hello", "world"]);
    assert!((words[0].start - 0.0).abs() < 1e-6 && (words[0].end - 0.1).abs() < 1e-6);
    assert!((words[1].start - 0.5).abs() < 1e-6);
}

#[test]
fn empty_tokens_yield_no_words() {
    assert!(group_tokens_to_words(&[], &[]).is_empty());
}
```

> Before writing the impl, read `model.rs:440-476` (`decode_tokens`) and the `DECODE_SPACE_RE` definition to learn the EXACT space marker/regex Parakeet uses; the test's `"▁"` is a placeholder — use the real marker. Word `start` = first token ts in the word; `end` = last token ts (or next word's start; pick first-token-of-next-word if last-token end is unavailable — document the choice).

- [ ] **Step 2: Run → fail** — `cargo test --features metal words` → FAIL.

- [ ] **Step 3: Implement grouping** in `words.rs` (group on the real space marker; map each word to its first/last token timestamp) and `transcribe_audio_timestamped` in `parakeet_engine.rs` (calls `model.transcribe_samples`, then `group_tokens_to_words(&result.tokens, &result.timestamps)`, returns `TranscribedWords { text: result.text, words }`). Keep the existing `transcribe_audio -> Result<String>` unchanged (no churn on live/STT-lock sites).

- [ ] **Step 4: Run → pass** — `cargo test --features metal words`.

- [ ] **Step 5: File-size + commit**

```bash
bash scripts/check-file-size.sh
git add frontend/src-tauri/src/parakeet_engine/
git commit -m "feat(0046): parakeet token→word timestamps (WS2 W2.1)"
```

---

## Task 6: Persist word timestamps through the batch path

**Files:**
- Create: `frontend/src-tauri/migrations/<ts>_add_transcript_word_timestamps.sql`
- Modify: `transcripts.rs` (`TranscriptSegment`), `audio/common.rs` (`create_transcript_segments`), `database/repositories/transcript.rs` (`insert_segments` + the load query), `audio/retranscription.rs` / `retranscription_channels.rs` (capture words via `transcribe_audio_timestamped` on the batch path).

**Interfaces:**
- Produces: `transcripts.word_timestamps TEXT` (JSON `[{"w":..,"s":..,"e":..}, ..]`); `TranscriptSegment.word_timestamps: Option<String>`; batch-transcribed rows carry words.

- [ ] **Step 1: Migration** (forward-only, idempotent) — `ALTER TABLE transcripts ADD COLUMN word_timestamps TEXT;` Follow the exact style of `20260703000002_add_transcript_channel.sql` (guarded/idempotent per this repo's migration convention).

- [ ] **Step 2: Write failing round-trip test** — insert a `TranscriptSegment` with `word_timestamps: Some(json)`, load it back, assert equality. (Add to the transcript repository tests / db_lifecycle test.)

- [ ] **Step 3: Thread the field** — add `word_timestamps: Option<String>` to `TranscriptSegment` (`transcripts.rs:17`); `create_transcript_segments` (`audio/common.rs:89`) accepts optional per-segment words and serializes them to JSON; `insert_segments` (`database/repositories/transcript.rs:33`) adds the column to the INSERT and the row loader reads it. On the batch transcription path (`retranscription.rs` / `retranscription_channels.rs`), call `transcribe_audio_timestamped` (Parakeet) and pass the words into segment creation; Whisper batch path may pass `None` for now (fallback covers it — see Task 7).

- [ ] **Step 4: Run → pass** (`cargo test --features metal` round-trip + `--test db_lifecycle`).

- [ ] **Step 5: File-size + commit**

```bash
bash scripts/check-file-size.sh
git add frontend/src-tauri/migrations/ frontend/src-tauri/src/transcripts.rs frontend/src-tauri/src/audio/common.rs frontend/src-tauri/src/database/repositories/transcript.rs frontend/src-tauri/src/audio/retranscription.rs frontend/src-tauri/src/audio/retranscription_channels.rs
git commit -m "feat(0046): persist per-word timestamps on the batch path (WS2 W2.2)"
```

---

## Task 7: Word-edge split apportioning (WS2 consume) + fallback

**Files:**
- Modify: `frontend/src-tauri/src/diarization/split.rs` (`plan_split` apportioning + `split_straddling_rows` loads `word_timestamps`)

**Interfaces:**
- Consumes: `WordStamp`-shaped word timestamps parsed from the row's `word_timestamps` JSON.
- Produces: `plan_split(start, end, text, turns, words: Option<&[WordStamp]>)` — snaps each boundary to the word whose `[start,end]` brackets the boundary time; falls back to the existing char-proportional whitespace-snap when `words` is `None`.

- [ ] **Step 1: Write failing tests** — same fixture as the existing split test, once with word timestamps (assert the cut lands on the EXACT word boundary) and once with `None` (assert the current char-proportional result, unchanged).

```rust
#[test]
fn word_timestamps_snap_boundary_to_exact_word_edge() {
    let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
    // words with timings placing the true boundary between "me." and "Great"
    let words = /* Vec<WordStamp> for TEXT, with "Great" starting at 3.4 */;
    let parts = plan_split(0.0, 6.7, TEXT, &turns, Some(&words)).expect("split");
    assert_eq!(parts[0].text, "so that works for me.");
    assert_eq!(parts[1].text, "Great lets ship it tomorrow then");
}

#[test]
fn falls_back_to_char_proportional_without_words() {
    let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
    let parts = plan_split(0.0, 6.7, TEXT, &turns, None).expect("split");
    // identical to the pre-WS2 behavior:
    assert_eq!(parts[0].text, "so that works for me. Great");
    assert_eq!(parts[1].text, "lets ship it tomorrow then");
}
```

- [ ] **Step 2: Run → fail** (signature change / new snapping path).

- [ ] **Step 3: Implement** — add the `words` param to `plan_split`; when `Some`, for each boundary time `b`, find the word boundary nearest `b` (the gap between the word ending ≤ `b` and the word starting ≥ `b`) and compute its byte offset in `text` for the cut, replacing the char-proportional `target`/whitespace-snap block (`split.rs:134-146`). When `None`, keep the existing block verbatim. Update all `plan_split(..)` call sites (the `split_straddling_rows` loop and every unit test) to pass the new arg (`None` for the pre-WS2 tests). In `split_straddling_rows`, extend the row SELECT (`split.rs:210`) to also load `word_timestamps`, parse the JSON to `Vec<WordStamp>`, and pass `Some(&words)` (or `None` if NULL/parse-fail) to `plan_split`.

- [ ] **Step 4: Run → pass** — `cargo test --features metal --lib diarization::split` (both new tests + all existing, updated for the new arg).

- [ ] **Step 5: DER non-regression + file-size + commit**

```bash
cargo test --features metal --test diarization_tuning 2>&1 | tail -5   # DER unchanged
bash scripts/check-file-size.sh
git add frontend/src-tauri/src/diarization/split.rs
git commit -m "feat(0046): word-exact split apportioning with char-proportional fallback (WS2 W2.3)"
```

---

## Task 8: Full verification

- [ ] **Step 1: Rust gate**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal && cargo clippy --features metal && cargo test --features metal 2>&1 | tail -25`
Expected: clean (pre-existing `vad_filter` failure exempt — confirm it's the only failure and unchanged).

- [ ] **Step 2: DER + file-size**

Run: `cargo test --features metal --test diarization_tuning 2>&1 | tail -5` (DER at/below pinned) and `cd ../.. && bash scripts/check-file-size.sh` (ok).

- [ ] **Step 3: Frontend gate** (unchanged by this spec, but confirm nothing broke): `cd frontend && pnpm lint && pnpm test 2>&1 | tail -4`.

- [ ] **Step 4: Manual smoke (owner)** — record a real back-and-forth where you interject and the other person replies immediately → after processing, your words are "You" and theirs are theirs at the turn boundaries (both directions), and split text lands on whole words.

- [ ] **Step 5: Update spec + INDEX status to "Implemented (pending owner smoke)"; commit.**

---

## Self-Review Notes (spec coverage)

- **Spec WS1 W1.1** → Task 1 (owner turns from mic VAD).
- **W1.2** → Task 2 (inject after diarization, before split/align; not into embeddings).
- **W1.3** → Task 3 (per-part channel from run + owner-aware mic gating + persist). The refinement beyond the spec: gate mic-row splitting on owner-turn presence AND require an owner part, so a mic row never loses owner attribution.
- **W1.4** → Task 4 (both-direction integration + idempotency/override + DER).
- **WS2 W2.1** → Task 5 (Parakeet token→word; Whisper deferred to fallback).
- **W2.2** → Task 6 (migration + persistence threading; batch path).
- **W2.3/W2.4** → Task 7 (word-edge snapping + char-proportional fallback).
- **File-size ratchet** in every task gate (Global Constraints) — the lesson from the prior batch.
- **DER non-regression** in Tasks 4, 7, 8.
