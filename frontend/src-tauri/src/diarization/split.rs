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
//! - Mic-channel rows never split (they're all one speaker: the owner).
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
}

/// A single word's timing, deserialized from a row's `word_timestamps` JSON
/// column (specs/0046 WS2). The column is written by
/// `create_transcript_segments_with_words` in `audio/common.rs` as the compact
/// wire shape `[{"w":"word","s":1.23,"e":1.45}, ...]`; the field renames here
/// deserialize that shape symmetrically. `None` rows (Whisper-transcribed or
/// legacy, no per-word timing) fall back to char-proportional apportioning.
#[derive(Debug, Clone, serde::Deserialize)]
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

    // Apportion text at each boundary: word-exact when `words` is available,
    // else the original char-proportional whitespace-snap fallback.
    let cut_bytes: Vec<(usize, usize)> = if let Some(words) = words {
        let mut cuts: Vec<usize> = Vec::new();
        for &b in &boundaries {
            let cut = word_cut_byte(text, words, b);
            if cuts.last().is_some_and(|&prev| cut <= prev) {
                return None; // two boundaries snapped to the same word edge
            }
            cuts.push(cut);
        }
        cuts.into_iter().map(|c| (c, c)).collect()
    } else {
        // Whitespace candidates: (byte start, byte end) per whitespace run.
        let ws_runs: Vec<(usize, usize)> = whitespace_runs(text);
        if ws_runs.len() < boundaries.len() {
            return None;
        }
        let total = end - start;
        let mut cut_bytes: Vec<(usize, usize)> = Vec::new();
        for &b in &boundaries {
            let frac = ((b - start) / total).clamp(0.0, 1.0);
            let target = (text.len() as f32 * frac) as usize;
            let &nearest = ws_runs
                .iter()
                .min_by_key(|(s, _)| s.abs_diff(target))
                .expect("ws_runs non-empty");
            if cut_bytes.last().is_some_and(|&prev| nearest.0 <= prev.0) {
                return None; // two boundaries snapped to the same gap — too dense
            }
            cut_bytes.push(nearest);
        }
        cut_bytes
    };

    // Assemble parts; any empty text cancels the whole split (conservative).
    let mut parts: Vec<SplitPart> = Vec::new();
    let mut t_prev = start;
    let mut b_prev = 0usize;
    for (i, &(cut_s, cut_e)) in cut_bytes.iter().enumerate() {
        let t_next = boundaries[i];
        let piece = text[b_prev..cut_s].trim();
        if piece.is_empty() {
            return None;
        }
        parts.push(SplitPart {
            start: t_prev,
            end: t_next,
            text: piece.to_string(),
            channel: part_channels[i].clone(),
        });
        t_prev = t_next;
        b_prev = cut_e;
    }
    let tail = text[b_prev..].trim();
    if tail.is_empty() {
        return None;
    }
    parts.push(SplitPart {
        start: t_prev,
        end,
        text: tail.to_string(),
        channel: part_channels[cut_bytes.len()].clone(),
    });
    Some(parts)
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
/// empty-tail check then conservatively cancels the split.
fn word_cut_byte(text: &str, words: &[WordStamp], b: f32) -> usize {
    word_byte_ranges(text)
        .iter()
        .zip(words.iter())
        .find(|(_, w)| w.start >= b)
        .map(|(&(s, _), _)| s)
        .unwrap_or(text.len())
}

/// Split every eligible straddling row of `meeting_id` per `turns`. Returns the
/// number of rows that were split. All mutations run in one transaction.
pub async fn split_straddling_rows(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    turns: &[SpeakerTurn],
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
        let is_mic_row = channel.as_deref() == Some("microphone");
        if is_mic_row && !has_owner_turns {
            // No owner timeline to split against → keep today's behavior (mic
            // rows are single-speaker by construction, stay whole).
            continue;
        }
        // specs/0047 W2: owner turns refine only the owner's OWN (mic-tagged) rows.
        // For a system/mixed row, a "local" turn is either mic-bleed of the remote
        // voice (owner on speakers) or an RMS/turn disagreement; carving the row at
        // it would cut a spurious "You" clip out of the middle of remote speech.
        // Consider only the remote turns there, so a non-mic row can never gain a
        // "microphone" part. Remote<->remote splitting (specs/0044) is unaffected.
        let filtered_turns: Vec<SpeakerTurn>;
        let row_turns: &[SpeakerTurn] = if is_mic_row {
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
        if let Some(parts) =
            plan_split(start as f32, end as f32, &text, row_turns, words.as_deref())
        {
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

    let mut tx = pool.begin().await.context("begin split transaction")?;
    for (id, timestamp, parts) in &planned {
        let first = &parts[0];
        sqlx::query(
            "UPDATE transcripts
             SET transcript = ?, audio_end_time = ?, duration = ?, channel = ?,
                 word_timestamps = NULL
             WHERE id = ?",
        )
        .bind(&first.text)
        .bind(first.end as f64)
        .bind((first.end - first.start) as f64)
        .bind(&first.channel)
        .bind(id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("shrink split row {id}"))?;

        for part in &parts[1..] {
            sqlx::query(
                "INSERT INTO transcripts
                     (id, meeting_id, transcript, timestamp, audio_start_time,
                      audio_end_time, duration, channel)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(format!("transcript-{}", uuid::Uuid::new_v4()))
            .bind(meeting_id)
            .bind(&part.text)
            .bind(timestamp)
            .bind(part.start as f64)
            .bind(part.end as f64)
            .bind((part.end - part.start) as f64)
            .bind(&part.channel)
            .execute(&mut *tx)
            .await
            .context("insert split part")?;
        }
    }
    tx.commit().await.context("commit split transaction")?;

    log::info!(
        "diarization split: {} straddling row(s) split for meeting {meeting_id}",
        planned.len()
    );
    Ok(planned.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start: f32, end: f32, speaker: &str) -> SpeakerTurn {
        SpeakerTurn {
            start,
            end,
            speaker: speaker.to_string(),
        }
    }

    // A 6-word row whose padded span [0.0, 6.7] covers spk_0 [0,3] and spk_1
    // [3.4, 6.3] (fast handoff bridged by redemption into one row).
    const TEXT: &str = "so that works for me. Great lets ship it tomorrow then";

    #[test]
    fn straddling_row_splits_at_the_turn_boundary() {
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
        let parts = plan_split(0.0, 6.7, TEXT, &turns, None).expect("should split");
        assert_eq!(parts.len(), 2);
        // Boundary at the midpoint of the adjacent edges: (3.0 + 3.4) / 2 = 3.2.
        assert!(
            (parts[0].end - 3.2).abs() < 1e-4,
            "boundary was {}",
            parts[0].end
        );
        assert_eq!(parts[0].start, 0.0);
        assert_eq!(parts[1].end, 6.7);
        assert_eq!(parts[1].start, parts[0].end);
        // Time-proportional apportioning: boundary at 3.2/6.7 ≈ 48% of the text,
        // snapped to the nearest whitespace. That lands one word past the true
        // sentence boundary — approximate by design (see spec Risks); the win is
        // that the speaker CHANGE now exists at all.
        assert_eq!(parts[0].text, "so that works for me. Great");
        assert_eq!(parts[1].text, "lets ship it tomorrow then");
    }

    #[test]
    fn word_timestamps_snap_boundary_to_exact_word_edge() {
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
        // Word timings place the true boundary exactly between "me." and
        // "Great" ("Great" starts at 3.4, matching spk_1's turn onset); every
        // earlier word starts well before the computed boundary (3.2), so
        // word_cut_byte's "first word with start >= b" rule lands on "Great".
        let words = vec![
            WordStamp {
                text: "so".into(),
                start: 0.0,
                end: 0.2,
            },
            WordStamp {
                text: "that".into(),
                start: 0.2,
                end: 0.5,
            },
            WordStamp {
                text: "works".into(),
                start: 0.5,
                end: 0.9,
            },
            WordStamp {
                text: "for".into(),
                start: 0.9,
                end: 1.1,
            },
            WordStamp {
                text: "me.".into(),
                start: 1.1,
                end: 3.0,
            },
            WordStamp {
                text: "Great".into(),
                start: 3.4,
                end: 3.8,
            },
            WordStamp {
                text: "lets".into(),
                start: 3.8,
                end: 4.0,
            },
            WordStamp {
                text: "ship".into(),
                start: 4.0,
                end: 4.3,
            },
            WordStamp {
                text: "it".into(),
                start: 4.3,
                end: 4.5,
            },
            WordStamp {
                text: "tomorrow".into(),
                start: 4.5,
                end: 5.0,
            },
            WordStamp {
                text: "then".into(),
                start: 5.0,
                end: 6.7,
            },
        ];
        let parts = plan_split(0.0, 6.7, TEXT, &turns, Some(&words)).expect("split");
        assert_eq!(parts[0].text, "so that works for me.");
        assert_eq!(parts[1].text, "Great lets ship it tomorrow then");
    }

    /// specs/0046 WS2 critical fix regression: every other word-timestamp test
    /// in this module uses a row starting at t=0.0, where segment-relative and
    /// recording-relative times are numerically identical — the exact blind
    /// spot that let the origin bug ship. This mirrors
    /// `word_timestamps_snap_boundary_to_exact_word_edge` but at a row that
    /// starts well into the recording, with `words` already offset onto that
    /// origin (as `common.rs::create_transcript_segments_with_words` now
    /// produces). Before the fix, comparing recording-relative `turns`/`b`
    /// against segment-relative (0-based) `words` would find no word with
    /// `start >= b`, `word_cut_byte` would fall through to `text.len()`, and
    /// the empty-tail guard would cancel the split entirely.
    #[test]
    fn word_timestamps_snap_boundary_to_exact_word_edge_at_nonzero_row_start() {
        const OFFSET: f32 = 120.0;
        let turns = vec![
            turn(OFFSET, OFFSET + 3.0, "spk_0"),
            turn(OFFSET + 3.4, OFFSET + 6.3, "spk_1"),
        ];
        let raw_words: &[(&str, f32, f32)] = &[
            ("so", 0.0, 0.2),
            ("that", 0.2, 0.5),
            ("works", 0.5, 0.9),
            ("for", 0.9, 1.1),
            ("me.", 1.1, 3.0),
            ("Great", 3.4, 3.8),
            ("lets", 3.8, 4.0),
            ("ship", 4.0, 4.3),
            ("it", 4.3, 4.5),
            ("tomorrow", 4.5, 5.0),
            ("then", 5.0, 6.7),
        ];
        let words: Vec<WordStamp> = raw_words
            .iter()
            .map(|&(text, start, end)| WordStamp {
                text: text.to_string(),
                start: OFFSET + start,
                end: OFFSET + end,
            })
            .collect();

        let parts = plan_split(OFFSET, OFFSET + 6.7, TEXT, &turns, Some(&words)).expect("split");
        assert_eq!(parts[0].text, "so that works for me.");
        assert_eq!(parts[1].text, "Great lets ship it tomorrow then");
        assert!(
            (parts[0].end - (OFFSET + 3.2)).abs() < 1e-4,
            "boundary was {}",
            parts[0].end
        );
    }

    #[test]
    fn falls_back_to_char_proportional_without_words() {
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
        let parts = plan_split(0.0, 6.7, TEXT, &turns, None).expect("split");
        // Identical to the pre-WS2 behavior (see straddling_row_splits_at_the_turn_boundary).
        assert_eq!(parts[0].text, "so that works for me. Great");
        assert_eq!(parts[1].text, "lets ship it tomorrow then");
    }

    #[test]
    fn single_speaker_row_stays_whole() {
        let turns = vec![turn(0.0, 7.0, "spk_0")];
        assert_eq!(plan_split(0.0, 6.7, TEXT, &turns, None), None);
    }

    #[test]
    fn weak_second_speaker_overlap_does_not_split() {
        // spk_1 only brushes the row for 0.4 s (< MIN_SPLIT_PART_SECS).
        let turns = vec![turn(0.0, 5.9, "spk_0"), turn(5.9, 6.3, "spk_1")];
        assert_eq!(plan_split(0.0, 6.7, TEXT, &turns, None), None);
    }

    #[test]
    fn brief_interjection_between_same_speaker_runs_does_not_split() {
        // spk_1's 0.3 s "mm-hm" inside spk_0's row: the weak run is dropped and
        // the flanking spk_0 runs re-merge → no split.
        let turns = vec![
            turn(0.0, 3.0, "spk_0"),
            turn(3.0, 3.3, "spk_1"),
            turn(3.3, 6.5, "spk_0"),
        ];
        assert_eq!(plan_split(0.0, 6.7, TEXT, &turns, None), None);
    }

    #[test]
    fn one_word_row_never_splits() {
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
        assert_eq!(plan_split(0.0, 6.7, "Okay", &turns, None), None);
    }

    #[test]
    fn three_speaker_row_splits_into_three_parts() {
        let turns = vec![
            turn(0.0, 2.0, "spk_0"),
            turn(2.2, 4.2, "spk_1"),
            turn(4.4, 6.4, "spk_2"),
        ];
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let parts = plan_split(0.0, 6.7, text, &turns, None).expect("should split");
        assert_eq!(parts.len(), 3);
        assert!((parts[0].end - 2.1).abs() < 1e-4);
        assert!((parts[1].end - 4.3).abs() < 1e-4);
        // Contiguous, covering the whole row.
        assert_eq!(parts[0].start, 0.0);
        assert_eq!(parts[2].end, 6.7);
        for pair in parts.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        // All text preserved, no words lost or duplicated.
        let rejoined = parts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(rejoined, text);
    }

    #[test]
    fn already_split_part_is_idempotent() {
        // Re-running over a part that now covers a single turn: no further split.
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
        assert_eq!(
            plan_split(0.0, 3.2, "so that works for me.", &turns, None),
            None
        );
        assert_eq!(
            plan_split(3.2, 6.7, "Great lets ship it tomorrow then", &turns, None),
            None
        );
    }

    #[test]
    fn unicode_text_splits_on_char_boundaries() {
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
        let text = "naïve café résumé — ok großartig übrigens";
        let parts = plan_split(0.0, 6.7, text, &turns, None).expect("should split");
        assert_eq!(parts.len(), 2);
        let rejoined: String = format!("{} {}", parts[0].text, parts[1].text);
        // Whitespace-normalized round trip (the em-dash gap may host the cut).
        assert_eq!(
            rejoined.split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }

    fn channel_of(part: &SplitPart) -> &str {
        part.channel.as_deref().unwrap_or("system")
    }

    #[test]
    fn owner_run_part_is_tagged_microphone_and_system_run_part_system() {
        // [Person A tail (spk_0) + owner onset (local)] glued into one row.
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "local")];
        let parts = plan_split(0.0, 6.7, TEXT, &turns, None).expect("should split");
        assert_eq!(parts.len(), 2);
        assert_eq!(channel_of(&parts[0]), "system"); // spk_0 part
        assert_eq!(channel_of(&parts[1]), "microphone"); // owner part
    }

    #[test]
    fn owner_first_then_remote_tags_each_part_by_run() {
        // [owner tail (local) + Person A onset (spk_0)].
        let turns = vec![turn(0.0, 3.0, "local"), turn(3.4, 6.3, "spk_0")];
        let parts = plan_split(0.0, 6.7, TEXT, &turns, None).expect("should split");
        assert_eq!(channel_of(&parts[0]), "microphone");
        assert_eq!(channel_of(&parts[1]), "system");
    }

    #[test]
    fn system_only_split_parts_are_all_system() {
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")];
        let parts = plan_split(0.0, 6.7, TEXT, &turns, None).expect("should split");
        assert!(parts.iter().all(|p| channel_of(p) == "system"));
    }
}
