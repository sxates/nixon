//! Cutting stored transcript rows at owner<->remote handoffs (specs/0055 Stage 2).
//!
//! A transcript row is one VAD segment, and conversational handoffs are faster than
//! the VAD's redemption window, so a row often holds the previous speaker's tail AND
//! the next speaker's opening sentence. Its single `channel` tag is a whole-row RMS
//! vote, and 84% of rows come out `mixed` (speaker bleed is normalized up to match the
//! system tap). [`crate::diarization::split`] may not carve an owner part out of a
//! non-`microphone` row (specs/0047 W2 — that is how mic bleed became fake "You"
//! clips), so the owner's opening sentence after a remote speaker stayed with that
//! remote speaker: the "first sentence goes to the previous speaker" report.
//!
//! The evidence to place the boundary exists at 600 ms resolution: the same
//! per-window classifier the capture path uses ([`classify_window_channel`]), here
//! recomputed offline from the meeting's `mic` + `system` tracks. For each row we take
//! its windows, group them into owner (`microphone`) vs remote (`system`/`mixed`),
//! demote owner stretches shorter than [`MIN_OWNER_PART_SECS`] (a wrong "You" is worse
//! than a missing one — the same asymmetric guard Stage 1 measured live), and cut the
//! row at the group boundaries, word-exact when the row has word stamps.
//!
//! This runs before the turn split, in call mode only. After it, owner parts are
//! tagged `microphone` and remote parts never are, so the turn split only has to
//! separate remote speakers from each other.

use std::collections::{HashSet, VecDeque};

use anyhow::{Context, Result};

/// Re-exported so evals and integration tests can build a timeline from windows.
pub use crate::audio::common::ChannelTag;
use crate::audio::pipeline::{
    channel_runs_for_span, classify_window_channel, dominant_channel_for_span,
};
use crate::diarization::split::{apply_splits, cut_at_times, SplitPart, WordStamp};

/// Window size (ms): the capture mixer's 600 ms window, which
/// [`classify_window_channel`] was tuned against (specs/0029, /0043).
pub const CHANNEL_WINDOW_MS: f64 = 600.0;

/// Shortest owner stretch (seconds) that may be cut out as a "You" part. Mirrors
/// `MIN_OWNER_PART_SECS` in `frontend/src/lib/live-channel-split.ts` (specs/0055
/// Stage 1, measured on a broad 0.7–1.0 s plateau). On speakers, an inter-phrase dip
/// in the system track can let normalized bleed classify one window `Microphone`;
/// this keeps such a blip from becoming a "You" clip in someone else's sentence.
/// Deliberately owner-only: a brief remote stretch is NOT demoted.
pub const MIN_OWNER_PART_SECS: f64 = 0.75;

/// Shortest remote stretch (seconds) cut out from BETWEEN two owner stretches.
/// Shorter, it is a backchannel ("mm-hm") or overlap under the owner's own speech:
/// it can't hold words of its own, so cutting there would only hand the owner's
/// surrounding words to a remote speaker.
pub const MIN_INTERIOR_REMOTE_SECS: f64 = 0.75;

/// A stretch at the very start or end of a row shorter than this (seconds) is folded
/// into its neighbour before cutting. It sits inside the VAD's 300 ms / 400 ms
/// padding, where there is too little speech to hold a word; cut off, the
/// char-proportional fallback would still hand it the row's first or last word.
pub const EDGE_SLIVER_SECS: f64 = 0.4;

/// How far (seconds) the transcript may run past the channel tracks before the
/// tracks are treated as a different timeline. Mirrors `MAX_DURATION_DRIFT_SECONDS`
/// in `audio/retranscription_channels.rs`: a resumed meeting whose channel files
/// cover only part of it would otherwise be cut against the wrong audio.
pub const MAX_TRACK_SHORTFALL_SECS: f64 = 5.0;

/// Upper bound on parts per row. A long batch row can cross several handoffs; past
/// this, the row stays whole rather than being cut speculatively.
pub const MAX_CHANNEL_PARTS: usize = 6;

/// The whole recording's per-window channel classes, in recording-relative ms.
/// Silence windows (both tracks inactive) are not recorded.
#[derive(Debug, Clone, Default)]
pub struct ChannelTimeline {
    windows: Vec<(f64, f64, ChannelTag)>,
    /// Length of the shorter track, seconds.
    duration_secs: f64,
}

impl ChannelTimeline {
    /// Classify `mic` and `sys` (same time base and sample rate, sample-aligned from
    /// t = 0) in [`CHANNEL_WINDOW_MS`] windows. `None` when either track is missing:
    /// with no system track every active window would read `Microphone` and the whole
    /// meeting would become "You".
    pub fn from_tracks(mic: &[f32], sys: &[f32], sample_rate: usize) -> Option<Self> {
        if mic.is_empty() || sys.is_empty() || sample_rate == 0 {
            return None;
        }
        let win = (sample_rate as f64 * CHANNEL_WINDOW_MS / 1000.0) as usize;
        if win == 0 {
            return None;
        }
        let len = mic.len().min(sys.len());
        let to_ms = |i: usize| i as f64 * 1000.0 / sample_rate as f64;
        let mut windows = Vec::with_capacity(len / win + 1);
        let mut w = 0usize;
        while w < len {
            let we = (w + win).min(len);
            if let Some(tag) = classify_window_channel(&mic[w..we], &sys[w..we]) {
                windows.push((to_ms(w), to_ms(we), tag));
            }
            w = we;
        }
        Some(Self {
            windows,
            duration_secs: len as f64 / sample_rate as f64,
        })
    }

    /// Build from already-classified windows (ms). For tests and evals.
    pub fn from_windows(mut windows: Vec<(f64, f64, ChannelTag)>) -> Self {
        windows.sort_by(|a, b| a.0.total_cmp(&b.0));
        let duration_secs = windows.last().map_or(0.0, |w| w.1 / 1000.0);
        Self {
            windows,
            duration_secs,
        }
    }

    /// The whole-span verdict for `[start, end)` seconds — what capture stores as
    /// `transcripts.channel` for a row. For evals.
    pub fn dominant_tag(&self, start: f32, end: f32) -> Option<ChannelTag> {
        let (s, e) = (start as f64 * 1000.0, end as f64 * 1000.0);
        dominant_channel_for_span(&self.windows_in(s, e), s, e)
    }

    /// The windows overlapping `[start_ms, end_ms)`, in the shape the shared
    /// [`channel_runs_for_span`] / [`dominant_channel_for_span`] take.
    fn windows_in(&self, start_ms: f64, end_ms: f64) -> VecDeque<(f64, f64, ChannelTag)> {
        let first = self.windows.partition_point(|w| w.1 <= start_ms);
        self.windows[first..]
            .iter()
            .take_while(|w| w.0 < end_ms)
            .copied()
            .collect()
    }
}

/// One owner-or-remote stretch of a row.
struct Group {
    start: f64,
    end: f64,
    owner: bool,
}

/// Merge adjacent same-side groups.
fn push_group(groups: &mut Vec<Group>, g: Group) {
    match groups.last_mut() {
        Some(last) if last.owner == g.owner => last.end = last.end.max(g.end),
        _ => groups.push(g),
    }
}

/// A row's owner/remote groups after the brief-owner demotion and edge-sliver fold:
/// exactly what the channel split cuts on.
fn guarded_groups(start: f32, end: f32, timeline: &ChannelTimeline) -> Vec<Group> {
    let (start_ms, end_ms) = (start as f64 * 1000.0, end as f64 * 1000.0);
    let windows = timeline.windows_in(start_ms, end_ms);
    let runs = channel_runs_for_span(&windows, start_ms, end_ms);

    let mut groups: Vec<Group> = Vec::new();
    for run in &runs {
        push_group(
            &mut groups,
            Group {
                start: run.start,
                end: run.end,
                owner: run.tag == ChannelTag::Microphone,
            },
        );
    }
    // Demote brief owner stretches, then re-merge: a blip between two remote
    // stretches leaves one remote group, so the row stays whole.
    let mut guarded: Vec<Group> = Vec::new();
    for g in groups {
        let owner = g.owner && g.end - g.start >= MIN_OWNER_PART_SECS;
        push_group(&mut guarded, Group { owner, ..g });
    }
    // Fold a brief remote stretch BETWEEN two owner stretches back into the owner
    // (see MIN_INTERIOR_REMOTE_SECS), then re-merge.
    let mut folded: Vec<Group> = Vec::new();
    for i in 0..guarded.len() {
        let g = &guarded[i];
        let interior = i > 0 && i + 1 < guarded.len();
        let brief_remote = !g.owner && interior && g.end - g.start < MIN_INTERIOR_REMOTE_SECS;
        push_group(
            &mut folded,
            Group {
                start: g.start,
                end: g.end,
                owner: g.owner || brief_remote,
            },
        );
    }
    let mut guarded = folded;
    // Fold edge slivers into their neighbour (see EDGE_SLIVER_SECS).
    if guarded.len() >= 2 && guarded[0].end - guarded[0].start < EDGE_SLIVER_SECS {
        let first = guarded.remove(0);
        guarded[0].start = first.start;
    }
    if guarded.len() >= 2 {
        let n = guarded.len();
        if guarded[n - 1].end - guarded[n - 1].start < EDGE_SLIVER_SECS {
            let last = guarded.pop().expect("len >= 2");
            guarded[n - 2].end = last.end;
        }
    }
    guarded
}

/// True when the timeline puts the whole of `[start, end)` seconds on ONE side (owner
/// or remote) — nothing for an owner<->remote cut to do. The turn split uses this to
/// leave such mic rows alone instead of re-cutting them on the coarser owner turns.
pub fn is_single_side(start: f32, end: f32, timeline: &ChannelTimeline) -> bool {
    end <= start || guarded_groups(start, end, timeline).len() < 2
}

/// Plan how to cut one row at its owner<->remote changes, or `None` to keep it whole.
///
/// `start`/`end` are the row's stored seconds; `words` its word stamps when present.
/// Owner parts carry `channel = "microphone"`; remote parts carry `"system"` when the
/// system track clearly led their windows, else `"mixed"` (including a remote group
/// made partly of demoted owner blips) — never `"microphone"`.
pub fn plan_channel_split(
    start: f32,
    end: f32,
    text: &str,
    timeline: &ChannelTimeline,
    words: Option<&[WordStamp]>,
) -> Option<Vec<SplitPart>> {
    if end <= start || text.split_whitespace().count() < 2 {
        return None;
    }
    let guarded = guarded_groups(start, end, timeline);
    if guarded.len() < 2 || guarded.len() > MAX_CHANNEL_PARTS {
        return None;
    }

    let boundaries: Vec<f32> = guarded[..guarded.len() - 1]
        .iter()
        .map(|g| g.end as f32)
        .collect();
    let pieces = cut_at_times(start, end, text, &boundaries, words)?;

    let mut parts = Vec::with_capacity(pieces.len());
    let mut t_prev = start;
    for (i, (piece, group)) in pieces.into_iter().zip(&guarded).enumerate() {
        let t_next = boundaries.get(i).copied().unwrap_or(end);
        let channel = if group.owner {
            "microphone"
        } else {
            let own = timeline.windows_in(group.start * 1000.0, group.end * 1000.0);
            match dominant_channel_for_span(&own, group.start * 1000.0, group.end * 1000.0) {
                Some(ChannelTag::System) => "system",
                _ => "mixed",
            }
        };
        parts.push(SplitPart {
            start: t_prev,
            end: t_next,
            text: piece.text,
            channel: Some(channel.to_string()),
            words: piece.words,
        });
        t_prev = t_next;
    }
    Some(parts)
}

/// Cut every eligible row of `meeting_id` at its owner<->remote changes per
/// `timeline`. Same safety rails as the turn split: rows with a manual span
/// correction or a hand edit stay whole. Returns the number of rows cut.
pub async fn split_rows_by_channel(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    timeline: &ChannelTimeline,
) -> Result<usize> {
    let rows = sqlx::query_as::<_, (String, String, String, f64, f64, Option<String>, i64)>(
        "SELECT id, transcript, timestamp, audio_start_time, audio_end_time, word_timestamps,
                user_edited
         FROM transcripts
         WHERE meeting_id = ?
           AND audio_start_time IS NOT NULL AND audio_end_time IS NOT NULL
         ORDER BY audio_start_time",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("load channel-splittable rows for meeting {meeting_id}"))?;

    let transcript_end = rows.iter().map(|r| r.4).fold(0.0f64, f64::max);
    if transcript_end > timeline.duration_secs + MAX_TRACK_SHORTFALL_SECS {
        log::warn!(
            "diarization channel split: transcript runs to {transcript_end:.1}s but the \
             channel tracks cover {:.1}s for meeting {meeting_id} — skipping the channel cut",
            timeline.duration_secs
        );
        return Ok(0);
    }

    let overridden: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT transcript_id FROM transcript_speaker_overrides WHERE meeting_id = ?",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .context("load speaker overrides")?
    .into_iter()
    .collect();

    let mut planned: Vec<(String, String, Vec<SplitPart>)> = Vec::new();
    for (id, text, timestamp, start, end, words_json, user_edited) in rows {
        if overridden.contains(&id) || user_edited != 0 {
            continue;
        }
        let words: Option<Vec<WordStamp>> = words_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());
        if let Some(parts) =
            plan_channel_split(start as f32, end as f32, &text, timeline, words.as_deref())
        {
            planned.push((id, timestamp, parts));
        }
    }
    if planned.is_empty() {
        return Ok(0);
    }
    apply_splits(pool, meeting_id, &planned).await?;
    log::info!(
        "diarization channel split: {} row(s) cut at owner<->remote handoffs for meeting {meeting_id}",
        planned.len()
    );
    Ok(planned.len())
}

#[cfg(test)]
mod tests;
