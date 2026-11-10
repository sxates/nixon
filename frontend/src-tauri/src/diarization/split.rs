//! Splitting transcript rows that straddle a speaker change (specs/0044 W1.2).
//!
//! A transcript row is one VAD segment, and a `SpeechEnd` only fires after the
//! redemption window of silence (400 ms live / 2000 ms batch). Real conversational
//! handoffs are faster than that, so a quick reply merges the previous speaker's
//! tail and the new speaker's onset into ONE row — which then takes a single
//! max-overlap label. That is the owner-reported "speaker name is off by one
//! segment" bug: the new speaker's first sentence rides in the old speaker's row.
//!
//! The offline diarization pass calls [`split_straddling_rows`] after computing
//! turns and BEFORE alignment: rows whose (pad-trimmed) span materially overlaps
//! two or more different-speaker turn runs are split at the turn boundaries, the
//! text apportioned time-proportionally (snapped to whitespace), and each part is
//! then labeled independently by the normal alignment pass.
//!
//! Safety rails:
//! - Mic-channel rows never split (they're all one speaker: the owner), except against
//!   the owner's own turns in a call, and in a room recording ([`SplitMode::Room`],
//!   specs/0078), where everyone is on the mic.
//! - Rows with a manual span correction (`transcript_speaker_overrides`) never
//!   split — override rows are keyed by stable `transcripts.id` (specs/0019) and
//!   a split would change what the id refers to.
//! - Hand-edited rows (`user_edited = 1`) never split either (specs/0061 review,
//!   M1) — the edit already cleared `word_timestamps`, so re-splitting one would
//!   fall back to char-proportional apportioning and machine-cut text a user fixed
//!   by hand, silently degrading its `edited` mark and count.
//! - The first part keeps the original row id (UPDATE), so FTS stays coherent via
//!   the existing `AFTER UPDATE OF transcript` trigger; later parts are new rows.
//! - Re-runs are idempotent: an already-split part overlaps a single speaker run
//!   and plans no further split.
//!
//! Planning ([`plan_split`]) is pure and unit-tested without a DB or model.

use anyhow::{Context, Result};
use std::collections::HashSet;

use crate::diarization::align::pad_trimmed;
use crate::diarization::SpeakerTurn;

/// Minimum overlap (seconds) each side of a boundary must have with its own
/// speaker's turns before we split there. Conservative: below this the part is
/// too small to attribute confidently and the row stays whole.
pub const MIN_SPLIT_PART_SECS: f32 = 0.75;

/// Upper bound on parts per row (a batch-path row bridging a rapid exchange can
/// straddle several turns; beyond this, splitting whole paragraphs by time
/// proportion gets too speculative).
pub const MAX_SPLIT_PARTS: usize = 4;

/// One planned part of a split row. Times are recording-relative seconds over
/// the ORIGINAL (untrimmed) row span; `text` is the apportioned slice, trimmed.
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
    /// This part's own word stamps (recording-relative), so a later cut — the turn
    /// split after the channel split, or a re-run — stays word-exact (specs/0055 Stage 2).
    pub words: Option<Vec<WordStamp>>,
}

/// A single word's timing, deserialized from a row's `word_timestamps` JSON
/// column (specs/0046 WS2). The column is written by
/// `create_transcript_segments_with_words` in `audio/common.rs` as the compact
/// wire shape `[{"w":"word","s":1.23,"e":1.45}, ...]`; the field renames here
/// deserialize that shape symmetrically. `None` rows (Whisper-transcribed or
/// legacy, no per-word timing) fall back to char-proportional apportioning.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct WordStamp {
    #[serde(rename = "w")]
    pub text: String,
    #[serde(rename = "s")]
    pub start: f32,
    #[serde(rename = "e")]
    pub end: f32,
}

/// Map a kept run's speaker key to the channel its split part should carry.
fn channel_for_speaker(speaker: &str) -> Option<String> {
    Some(if speaker == crate::diarization::align::LOCAL_SPEAKER_KEY {
        "microphone".to_string()
    } else {
        "system".to_string()
    })
}

/// A same-speaker run of consecutive overlapping turns.
struct SpeakerRun {
    speaker: String,
    overlap: f32,
    first_start: f32,
    last_end: f32,
}

/// Plan how to split one row, or `None` when the row should stay whole.
///
/// `start`/`end` are the row's stored (padded) timestamps; `text` its transcript.
/// `turns` are the meeting's diarization turns, time-ordered. `words`, when
/// `Some`, are that row's per-word timestamps (specs/0046 WS2) and are used to
/// snap each boundary to an exact word edge instead of the char-proportional
/// whitespace-snap fallback used when `words` is `None`.
pub fn plan_split(
    start: f32,
    end: f32,
    text: &str,
    turns: &[SpeakerTurn],
    words: Option<&[WordStamp]>,
) -> Option<Vec<SplitPart>> {
    if end <= start || text.split_whitespace().count() < 2 {
        return None;
    }
    let (core_s, core_e) = pad_trimmed(start, end);

    // Group the core-overlapping turns into same-speaker runs, in time order.
    let mut runs: Vec<SpeakerRun> = Vec::new();
    for t in turns {
        let ov = (core_e.min(t.end) - core_s.max(t.start)).max(0.0);
        if ov <= 0.0 {
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.speaker == t.speaker => {
                last.overlap += ov;
                last.last_end = last.last_end.max(t.end);
            }
            _ => runs.push(SpeakerRun {
                speaker: t.speaker.clone(),
                overlap: ov,
                first_start: t.start,
                last_end: t.end,
            }),
        }
    }

    // Keep only runs with a confident share of the row, then re-merge any
    // now-adjacent same-speaker runs (a weak interjection between them dropped).
    let mut kept: Vec<SpeakerRun> = Vec::new();
    for run in runs
        .into_iter()
        .filter(|r| r.overlap >= MIN_SPLIT_PART_SECS)
    {
        match kept.last_mut() {
            Some(last) if last.speaker == run.speaker => {
                last.overlap += run.overlap;
                last.last_end = last.last_end.max(run.last_end);
            }
            _ => kept.push(run),
        }
    }
    if kept.len() < 2 {
        return None;
    }
    kept.truncate(MAX_SPLIT_PARTS);

    // Capture each kept run's channel before `kept` is consumed for boundaries;
    // parts are built in kept-run order below, so part `i` ↔ `kept[i]`.
    let part_channels: Vec<Option<String>> = kept
        .iter()
        .map(|r| channel_for_speaker(&r.speaker))
        .collect();

    // Boundary between consecutive runs: midpoint of the adjacent turn edges,
    // clamped inside the core. Must come out strictly increasing.
    let mut boundaries: Vec<f32> = Vec::new();
    for pair in kept.windows(2) {
        let b = ((pair[0].last_end + pair[1].first_start) / 2.0).clamp(core_s, core_e);
        if boundaries.last().is_some_and(|&prev| b <= prev) {
            return None;
        }
        boundaries.push(b);
    }

    let pieces = cut_at_times(start, end, text, &boundaries, words)?;
    let mut parts: Vec<SplitPart> = Vec::new();
    let mut t_prev = start;
    for (i, piece) in pieces.into_iter().enumerate() {
        let t_next = boundaries.get(i).copied().unwrap_or(end);
        parts.push(SplitPart {
            start: t_prev,
            end: t_next,
            text: piece.text,
            channel: part_channels[i].clone(),
            words: piece.words,
        });
        t_prev = t_next;
    }
    Some(parts)
}

/// One text slice from [`cut_at_times`], with its own word stamps when known.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextPiece {
    pub text: String,
    /// The slice's word stamps; `None` when the row had none, or when its stamps
    /// didn't line up 1:1 with its words (then a slice could misattribute timing).
    pub words: Option<Vec<WordStamp>>,
}

/// Cut a row's `text` at the strictly increasing boundary TIMES `boundaries`
/// (recording-relative seconds inside `[start, end]`), returning
/// `boundaries.len() + 1` trimmed, non-empty pieces — or `None` when any piece would
/// be empty or two boundaries snap to the same cut (conservative: keep the row whole).
///
/// Word-exact when `words` is available (cut before the first word starting at/after
/// the boundary); otherwise char-proportional with a whitespace snap. Shared by the
/// turn split here and the channel split (specs/0055 Stage 2).
pub(crate) fn cut_at_times(
    start: f32,
    end: f32,
    text: &str,
    boundaries: &[f32],
    words: Option<&[WordStamp]>,
) -> Option<Vec<TextPiece>> {
    // Word-exact cutting (and slicing) needs stamps that line up 1:1 with the text's
    // words; otherwise every cut after a mismatch would land a word off, so fall back
    // to char-proportional instead (specs/0055 Stage 2 review).
    let words = words.filter(|w| w.len() == word_byte_ranges(text).len());
    // (cut byte start, cut byte end, word index of the next piece's first word)
    let cuts: Vec<(usize, usize, usize)> = if let Some(words) = words {
        let mut cuts: Vec<(usize, usize, usize)> = Vec::new();
        for &b in boundaries {
            let (cut, idx) = word_cut(text, words, b);
            if cuts.last().is_some_and(|&(prev, _, _)| cut <= prev) {
                return None; // two boundaries snapped to the same word edge
            }
            cuts.push((cut, cut, idx));
        }
        cuts
    } else {
        // Whitespace candidates: (byte start, byte end) per whitespace run.
        let ws_runs: Vec<(usize, usize)> = whitespace_runs(text);
        if ws_runs.len() < boundaries.len() || end <= start {
            return None;
        }
        let total = end - start;
        let mut cuts: Vec<(usize, usize, usize)> = Vec::new();
        for &b in boundaries {
            let frac = ((b - start) / total).clamp(0.0, 1.0);
            let target = (text.len() as f32 * frac) as usize;
            let &nearest = ws_runs
                .iter()
                .min_by_key(|(s, _)| s.abs_diff(target))
                .expect("ws_runs non-empty");
            if cuts.last().is_some_and(|&(prev, _, _)| nearest.0 <= prev) {
                return None; // two boundaries snapped to the same gap — too dense
            }
            cuts.push((nearest.0, nearest.1, 0));
        }
        cuts
    };

    let aligned_words = words;

    let mut pieces: Vec<TextPiece> = Vec::new();
    let mut b_prev = 0usize;
    let mut w_prev = 0usize;
    for &(cut_s, cut_e, w_next) in &cuts {
        let piece = text[b_prev..cut_s].trim();
        if piece.is_empty() {
            return None;
        }
        pieces.push(TextPiece {
            text: piece.to_string(),
            words: aligned_words.map(|w| w[w_prev..w_next.max(w_prev)].to_vec()),
        });
        b_prev = cut_e;
        w_prev = w_next;
    }
    let tail = text[b_prev..].trim();
    if tail.is_empty() {
        return None;
    }
    pieces.push(TextPiece {
        text: tail.to_string(),
        words: aligned_words.map(|w| w[w_prev.min(w.len())..].to_vec()),
    });
    Some(pieces)
}

/// Byte ranges of each maximal whitespace run in `text` (UTF-8 safe: whitespace
/// chars are matched on char boundaries).
fn whitespace_runs(text: &str) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            match current.as_mut() {
                Some((_, e)) => *e = i + c.len_utf8(),
                None => current = Some((i, i + c.len_utf8())),
            }
        } else if let Some(run) = current.take() {
            runs.push(run);
        }
    }
    // A trailing whitespace run is deliberately NOT a candidate: cutting there
    // would leave an empty tail part.
    runs
}

/// Byte ranges of each maximal non-whitespace run ("word") in `text`, in
/// order and UTF-8 safe (char-boundary matched, mirroring [`whitespace_runs`]).
fn word_byte_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(run) = current.take() {
                runs.push(run);
            }
        } else {
            match current.as_mut() {
                Some((_, e)) => *e = i + c.len_utf8(),
                None => current = Some((i, i + c.len_utf8())),
            }
        }
    }
    if let Some(run) = current.take() {
        runs.push(run);
    }
    runs
}

/// Snap boundary TIME `b` to a word edge in `text`.
///
/// `words` is matched positionally to `text`'s whitespace-split tokens (the
/// row's transcript and its word timestamps come from the same STT pass, so
/// they're expected to line up 1:1; a small count drift just truncates to the
/// shorter length via `zip`, degrading gracefully rather than panicking).
///
/// Rule: cut immediately before the first word whose `start` is ≥ `b` — that
/// word becomes the first word of the next part. This also covers the case
/// where `b` falls inside a word's own span (e.g. a boundary computed as a
/// midpoint that lands mid-word): the whole word goes to the next part rather
/// than being split or dropped. If no word starts at/after `b` (the boundary
/// is at or beyond the last word), cut at the end of `text`; the caller's
/// empty-tail check then conservatively cancels the split. Returns the cut byte and
/// the index of the word the cut lands before.
fn word_cut(text: &str, words: &[WordStamp], b: f32) -> (usize, usize) {
    word_byte_ranges(text)
        .iter()
        .zip(words.iter())
        .enumerate()
        .find(|(_, (_, w))| w.start >= b)
        .map(|(i, (&(s, _), _))| (s, i))
        .unwrap_or((text.len(), words.len()))
}

/// Where the owner's voice is, for [`split_straddling_rows`] (specs/0078).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitMode {
    /// A call: mic rows are the owner's and split only against owner turns; each part is
    /// re-tagged to the channel of the speaker who owns it (spec 0046 W1.3).
    Call,
    /// A room recording: every turn is a clustered voice, so mic rows split like any
    /// other row, and every part KEEPS its row's original `channel`. The tag stays the
    /// raw capture record, so switching the meeting back to call mode still has it.
    Room,
}

/// Split every eligible straddling row of `meeting_id` per `turns`. Returns the
/// number of rows that were split. All mutations run in one transaction.
pub async fn split_straddling_rows(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    turns: &[SpeakerTurn],
    mode: SplitMode,
) -> Result<usize> {
    split_straddling_rows_with_timeline(pool, meeting_id, turns, mode, None).await
}

/// [`split_straddling_rows`], after the channel split (specs/0055 Stage 2): a mic row
/// the `timeline` puts wholly on one side is left alone — the channel cut already
/// decided it, and the owner turns (mic VAD, 2 s redemption) are too coarse to improve
/// on it. Mic rows the channel split wanted to cut but couldn't still get the
/// owner-turn split as a backstop.
pub async fn split_straddling_rows_with_timeline(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    turns: &[SpeakerTurn],
    mode: SplitMode,
    timeline: Option<&crate::diarization::channel_split::ChannelTimeline>,
) -> Result<usize> {
    if turns.is_empty() {
        return Ok(0);
    }

    let rows = sqlx::query_as::<_, (String, String, String, f64, f64, Option<String>, Option<String>, i64)>(
        "SELECT id, transcript, timestamp, audio_start_time, audio_end_time, channel, word_timestamps, user_edited
         FROM transcripts
         WHERE meeting_id = ?
           AND audio_start_time IS NOT NULL AND audio_end_time IS NOT NULL
         ORDER BY audio_start_time",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("load splittable rows for meeting {meeting_id}"))?;

    let overridden: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT transcript_id FROM transcript_speaker_overrides WHERE meeting_id = ?",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .context("load speaker overrides")?
    .into_iter()
    .collect();

    // Owner turns must be present before we trust a mic row's split plan to
    // preserve the owner's attribution (see is_mic_row guard below).
    let has_owner_turns = turns
        .iter()
        .any(|t| t.speaker == crate::diarization::align::LOCAL_SPEAKER_KEY);

    let mut planned: Vec<(String, String, Vec<SplitPart>)> = Vec::new();
    for (id, text, timestamp, start, end, channel, word_timestamps_json, user_edited) in rows {
        // Overridden rows must keep their id ↔ span meaning stable (specs/0019).
        if overridden.contains(&id) {
            continue;
        }
        // specs/0061 review, M1 — a hand-edited row (`user_edited = 1`) must be left
        // whole exactly like an overridden row: its `word_timestamps` is already
        // NULL (the edit save cleared them), so re-splitting it here would fall back
        // to char-proportional apportioning, silently machine-cut text the user
        // fixed by hand, and re-insert the remainder as a fresh `user_edited = 0`
        // row — degrading both the `edited` mark and its count with no words lost.
        if user_edited != 0 {
            continue;
        }
        let room = mode == SplitMode::Room;
        let is_mic_row = channel.as_deref() == Some("microphone");
        if is_mic_row && !has_owner_turns && !room {
            // No owner timeline to split against → keep today's behavior (mic
            // rows are single-speaker by construction, stay whole).
            continue;
        }
        if is_mic_row
            && !room
            && timeline.is_some_and(|tl| {
                crate::diarization::channel_split::is_single_side(start as f32, end as f32, tl)
            })
        {
            continue;
        }
        // specs/0047 W2: owner turns refine only the owner's OWN (mic-tagged) rows.
        // For a system/mixed row, a "local" turn is either mic-bleed of the remote
        // voice (owner on speakers) or an RMS/turn disagreement; carving the row at
        // it would cut a spurious "You" clip out of the middle of remote speech.
        // Consider only the remote turns there, so a non-mic row can never gain a
        // "microphone" part. Remote<->remote splitting (specs/0044) is unaffected.
        let filtered_turns: Vec<SpeakerTurn>;
        let row_turns: &[SpeakerTurn] = if is_mic_row || room {
            turns
        } else {
            filtered_turns = turns
                .iter()
                .filter(|t| t.speaker != crate::diarization::align::LOCAL_SPEAKER_KEY)
                .cloned()
                .collect();
            &filtered_turns
        };
        // NULL (Whisper/legacy rows) or a malformed JSON payload both fall
        // back to char-proportional apportioning inside plan_split.
        let words: Option<Vec<WordStamp>> = word_timestamps_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());
        if let Some(mut parts) =
            plan_split(start as f32, end as f32, &text, row_turns, words.as_deref())
        {
            if room {
                // Room mode never rewrites `transcripts.channel` (specs/0078).
                for part in &mut parts {
                    part.channel = channel.clone();
                }
                planned.push((id, timestamp, parts));
                continue;
            }
            // A mic-tagged row must retain the owner: only split it if the plan
            // yields at least one owner ("microphone") part; otherwise a bad RMS
            // tag could re-attribute the owner's words to a remote speaker.
            if is_mic_row
                && !parts
                    .iter()
                    .any(|p| p.channel.as_deref() == Some("microphone"))
            {
                continue;
            }
            planned.push((id, timestamp, parts));
        }
    }
    if planned.is_empty() {
        return Ok(0);
    }

    apply_splits(pool, meeting_id, &planned).await?;
    log::info!(
        "diarization split: {} straddling row(s) split for meeting {meeting_id}",
        planned.len()
    );
    Ok(planned.len())
}

/// Serialize a part's word stamps to the stored compact shape, `None` when absent.
fn words_json(words: &Option<Vec<WordStamp>>) -> Option<String> {
    words
        .as_ref()
        .filter(|w| !w.is_empty())
        .and_then(|w| serde_json::to_string(w).ok())
}

/// Write planned splits in one transaction: each row's first part UPDATEs the original
/// row (keeping its id, so FTS stays coherent via the `AFTER UPDATE OF transcript`
/// trigger) and later parts INSERT new rows. `planned` is `(row id, row timestamp,
/// parts)`. Shared by the turn split and the channel split (specs/0055 Stage 2).
pub(crate) async fn apply_splits(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    planned: &[(String, String, Vec<SplitPart>)],
) -> Result<()> {
    let mut tx = pool.begin().await.context("begin split transaction")?;
    for (id, timestamp, parts) in planned {
        let first = &parts[0];
        sqlx::query(
            "UPDATE transcripts
             SET transcript = ?, audio_end_time = ?, duration = ?, channel = ?,
                 word_timestamps = ?
             WHERE id = ?",
        )
        .bind(&first.text)
        .bind(first.end as f64)
        .bind((first.end - first.start) as f64)
        .bind(&first.channel)
        .bind(words_json(&first.words))
        .bind(id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("shrink split row {id}"))?;

        for part in &parts[1..] {
            sqlx::query(
                "INSERT INTO transcripts
                     (id, meeting_id, transcript, timestamp, audio_start_time,
                      audio_end_time, duration, channel, word_timestamps)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(format!("transcript-{}", uuid::Uuid::new_v4()))
            .bind(meeting_id)
            .bind(&part.text)
            .bind(timestamp)
            .bind(part.start as f64)
            .bind(part.end as f64)
            .bind((part.end - part.start) as f64)
            .bind(&part.channel)
            .bind(words_json(&part.words))
            .execute(&mut *tx)
            .await
            .context("insert split part")?;
        }
    }
    tx.commit().await.context("commit split transaction")?;
    Ok(())
}

#[cfg(test)]
mod tests;
