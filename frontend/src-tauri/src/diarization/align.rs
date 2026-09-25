//! Aligning diarization speaker turns onto transcript segments. (specs/0010)
//!
//! Diarization yields *speaker turns* `(start, end, speaker)`; STT yields
//! *segments* `(start, end, text)`. Both carry recording-relative timestamps (the
//! VAD pipeline timestamps every segment), so we assign each transcript segment
//! the diarization turn whose **time overlap with the segment is greatest**
//! (max overlap-duration). This is robust to the small boundary differences
//! between VAD segmentation and pyannote segmentation.
//!
//! Per-channel short-circuit (ADR-0005 decision 3): we diarize only the *system*
//! channel (the unknown remote speakers). Any segment that came from the
//! **microphone** channel is the local user and short-circuits to
//! [`LOCAL_SPEAKER_KEY`] without consulting the clustering result.
//!
//! Segments with no overlapping turn are *ambiguous*, and ambiguity never
//! defaults to "You" (specs/0043 W1.3 — a wrong "You" is worse than an honest
//! Unknown): a non-mic segment first tries **nearest-turn attribution** within
//! [`NEAREST_TURN_WINDOW_SECS`], then falls back to [`SYSTEM_FALLBACK_KEY`]
//! ("Unknown speaker"). Only the mic channel tag can produce [`LOCAL_SPEAKER_KEY`].
//!
//! **Behavior change for legacy meetings (accepted, specs/0043 W1.3):** rows with
//! a NULL `transcripts.channel` map to [`Channel::Mixed`], so re-running
//! diarization on an all-null legacy meeting now labels its no-overlap segments
//! "Unknown speaker" instead of "You". The recovery path for a mislabeled span is
//! the manual span correction flow (specs/0039 WS2), which survives re-runs.
//!
//! This is pure, dependency-free logic with no model dependency, so its unit
//! tests run unconditionally.

use crate::diarization::SpeakerTurn;

/// Stable per-meeting key for the mic/local user. Display name is "You" (set in
/// the DB layer, a later slice). The audio device is named "microphone" per
/// `CLAUDE.md` conventions; this key is the channel-level identity.
pub const LOCAL_SPEAKER_KEY: &str = "local";

/// Fallback key for a non-mic segment that can't be attributed to any turn (no
/// overlap and, for Mixed/live segments, no turn within
/// [`NEAREST_TURN_WINDOW_SECS`]). Falling back to "You" would be wrong — we have
/// no evidence the owner spoke; it lands in the WS3.3 overflow bucket
/// ([`UNKNOWN_SPEAKER_KEY`](crate::diarization::UNKNOWN_SPEAKER_KEY) → "Unknown
/// speaker") instead.
pub const SYSTEM_FALLBACK_KEY: &str = crate::diarization::UNKNOWN_SPEAKER_KEY;

/// Nearest-turn attribution window (seconds) for segments that overlap no turn
/// (specs/0043 W1.3). We measure the **gap between the two intervals** — the
/// distance from the segment's closest boundary to the turn's closest edge,
/// `max(turn.start - seg.end, seg.start - turn.end)` (0 for touching/overlapping
/// intervals). If the closest turn's gap is ≤ this window (inclusive), the
/// segment takes that turn's speaker; otherwise it falls back to
/// [`SYSTEM_FALLBACK_KEY`]. Boundary-to-boundary distance (not midpoint) is used
/// because it degenerates continuously into the max-overlap rule at gap 0.
pub const NEAREST_TURN_WINDOW_SECS: f32 = 1.0;

/// Which capture channel a transcript segment originated from — the per-segment
/// `transcripts.channel` tag recorded at capture time from pre-mix RMS dominance
/// (specs/0029 WS3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Local user's microphone — short-circuits to [`LOCAL_SPEAKER_KEY`].
    Microphone,
    /// Remote participants' system audio — consults the clustering result.
    System,
    /// Overlapped speech ('mixed') or no tag (legacy NULL rows / batch sources):
    /// greatest-overlap turn, else nearest turn within
    /// [`NEAREST_TURN_WINDOW_SECS`], else [`SYSTEM_FALLBACK_KEY`] — identical to
    /// [`align_system_turns_to_segments`]. Never the local user (specs/0043 W1.3).
    Mixed,
}

/// Where the owner's voice is, for [`align_turns_to_segments`] (specs/0078).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMode {
    /// A call: the owner is the mic channel and the turns are the system track's
    /// remote speakers (plus owner turns derived from the mic, used only for splitting).
    Call,
    /// A room recording: everyone is on the mic and every turn, `local` included, is a
    /// clustered voice. Mic-tagged rows are attributed from the turns like any other.
    Room,
}

/// A transcript segment to be labeled. Times are recording-relative seconds.
///
/// This is intentionally a thin local struct (not the DB `Transcript` model) so
/// `align.rs` stays decoupled from the schema slice. The orchestration slice
/// (P1-B `pipeline.rs`) maps DB rows ↔ this and writes the resulting key back.
#[derive(Debug, Clone, PartialEq)]
pub struct AlignableSegment {
    pub start: f32,
    pub end: f32,
    /// Which channel this segment came from (drives the mic short-circuit).
    pub channel: Channel,
}

impl AlignableSegment {
    pub fn new(start: f32, end: f32, channel: Channel) -> Self {
        Self {
            start,
            end,
            channel,
        }
    }
}

/// Minimum core (seconds) [`pad_trimmed`] preserves. A row shorter than the
/// combined VAD pads (700 ms) still needs enough span left to score overlaps.
const MIN_TRIMMED_CORE_SECS: f32 = 0.25;

/// Compensate for the VAD's segment padding before overlap scoring (specs/0044
/// W1.1). Every stored transcript row is padded outward for STT context —
/// `start` is [`PRE_SPEECH_PAD_MS`](crate::audio::vad::PRE_SPEECH_PAD_MS) early
/// and `end` is [`POST_SPEECH_PAD_MS`](crate::audio::vad::POST_SPEECH_PAD_MS)
/// late (silero applies the post pad on both the live and batch paths) — which
/// biases boundary overlaps toward the *earlier* speaker. This returns the
/// approximate speech core. When the row is too short to absorb the full trims,
/// both are scaled down proportionally so at least [`MIN_TRIMMED_CORE_SECS`]
/// (or the row's own length, if shorter) remains. Stored timestamps are never
/// modified — playback anchoring keeps the pads.
pub fn pad_trimmed(start: f32, end: f32) -> (f32, f32) {
    let pre = crate::audio::vad::PRE_SPEECH_PAD_MS as f32 / 1000.0;
    let post = crate::audio::vad::POST_SPEECH_PAD_MS as f32 / 1000.0;
    let len = (end - start).max(0.0);
    let core = (len - pre - post).max(MIN_TRIMMED_CORE_SECS.min(len));
    let available = (len - core).max(0.0);
    let scale = (available / (pre + post)).min(1.0);
    (start + pre * scale, end - post * scale)
}

/// Overlap duration (seconds) of two `[start, end)` intervals; 0 if disjoint.
fn overlap(a_start: f32, a_end: f32, b_start: f32, b_end: f32) -> f32 {
    (a_end.min(b_end) - a_start.max(b_start)).max(0.0)
}

/// Gap (seconds) between two `[start, end)` intervals — the distance between
/// their closest edges; 0 if they touch or overlap. (The negative part of
/// [`overlap`]'s subtraction, clamped the other way.)
fn interval_gap(a_start: f32, a_end: f32, b_start: f32, b_end: f32) -> f32 {
    (a_start.max(b_start) - a_end.min(b_end)).max(0.0)
}

/// Nearest-turn attribution for a segment that overlaps no turn (specs/0043
/// W1.3): the turn with the smallest [`interval_gap`] to the segment, if that gap
/// is within [`NEAREST_TURN_WINDOW_SECS`] (inclusive). Ties go to the first such
/// turn in `turns` (`min_by` keeps the first minimum), which is deterministic
/// because turns arrive time-ordered from the diarizer.
fn nearest_turn_within_window(
    turns: &[SpeakerTurn],
    seg_start: f32,
    seg_end: f32,
) -> Option<&SpeakerTurn> {
    turns
        .iter()
        .map(|t| (t, interval_gap(seg_start, seg_end, t.start, t.end)))
        .filter(|(_, gap)| *gap <= NEAREST_TURN_WINDOW_SECS)
        .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(t, _)| t)
}

/// Assign a speaker key to each segment.
///
/// Rules, in order, per segment:
/// 1. **Mic short-circuit:** a [`Channel::Microphone`] segment is always
///    [`LOCAL_SPEAKER_KEY`] (the known local user) — turns are not consulted, so
///    a system turn fully overlapping it can never steal the attribution.
/// 2. **Max overlap:** a [`Channel::System`] or [`Channel::Mixed`] segment takes
///    the `speaker` of the turn with the greatest time overlap.
/// 3. **Fallback (no overlapping turn):** a [`Channel::System`] segment gets
///    [`SYSTEM_FALLBACK_KEY`] directly (we know it isn't the mic user); a
///    [`Channel::Mixed`] segment first tries nearest-turn attribution within
///    [`NEAREST_TURN_WINDOW_SECS`], else [`SYSTEM_FALLBACK_KEY`]. **Never**
///    [`LOCAL_SPEAKER_KEY`]: the old "nobody remote spoke ⇒ the owner spoke"
///    heuristic was removed in specs/0043 W1.3 — ambiguity must not default to
///    "You". This changes what re-runs produce for all-null legacy meetings
///    (Mixed → Unknown instead of "You"); accepted, with specs/0039 WS2 span
///    correction as the recovery path.
///
/// Returns one key per input segment, in the same order. `turns` are the
/// system-channel diarization result (recording-relative seconds).
///
/// In [`AlignMode::Room`] (specs/0078) rule 1 is dropped: everyone spoke into the mic, so
/// a [`Channel::Microphone`] segment follows the [`Channel::Mixed`] rules, and `local`
/// turns (the owner's clustered voice) label segments like any other turn.
pub fn align_turns_to_segments(
    turns: &[SpeakerTurn],
    segments: &[AlignableSegment],
    mode: AlignMode,
) -> Vec<String> {
    // Call mode: owner ("local") turns exist only to split the owner's OWN mic-tagged
    // rows; they must never LABEL a System/Mixed segment — only the Microphone channel
    // tag yields "You" (specs/0047 W2). Filtering them out here keeps a bleed-
    // induced owner turn from stealing a remote speaker's segment (the reported
    // "You clip in the middle of someone else" symptom). Microphone segments short-
    // circuit before this set is consulted, so their attribution is unaffected.
    // Room mode: `local` is a real clustered voice, so nothing is filtered.
    let remote_turns: Vec<SpeakerTurn> = match mode {
        AlignMode::Call => turns
            .iter()
            .filter(|t| t.speaker != LOCAL_SPEAKER_KEY)
            .cloned()
            .collect(),
        AlignMode::Room => turns.to_vec(),
    };
    segments
        .iter()
        .map(|seg| {
            // 1. The local user is known by channel; no clustering needed (call only).
            if mode == AlignMode::Call && seg.channel == Channel::Microphone {
                return LOCAL_SPEAKER_KEY.to_string();
            }

            // 2. Greatest-overlap remote turn wins (owner turns excluded above).
            let best = remote_turns
                .iter()
                .map(|t| (t, overlap(seg.start, seg.end, t.start, t.end)))
                .filter(|(_, ov)| *ov > 0.0)
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            match best {
                Some((turn, _)) => turn.speaker.clone(),
                // 3. No confident overlap → never "You" (specs/0043 W1.3).
                // System-tagged segments are known-not-mic → straight to the
                // unknown bucket; Mixed/untagged tries the nearest remote turn
                // within the window first, then the unknown bucket. (Microphone
                // reaches here only in room mode, where it follows the Mixed rule.)
                None => {
                    if seg.channel == Channel::System {
                        SYSTEM_FALLBACK_KEY.to_string()
                    } else {
                        nearest_turn_within_window(&remote_turns, seg.start, seg.end)
                            .map(|t| t.speaker.clone())
                            .unwrap_or_else(|| SYSTEM_FALLBACK_KEY.to_string())
                    }
                }
            }
        })
        .collect()
}

/// P1 attribution for the **system-only** diarization path (P1-B2).
///
/// In P1 the transcript segments come from the *mixed* stream (channel identity is
/// destroyed before STT, see the spec), so we have no per-segment [`Channel`] tag.
/// We diarize only the **system** channel, which gives us turns for the remote
/// speakers. The attribution rule for each transcript segment is therefore:
///
/// 1. If the segment overlaps a system turn → the remote speaker of the
///    greatest-overlap turn (`spk_0`, `spk_1`, …).
/// 2. If the segment overlaps **no** system turn → the nearest turn within
///    [`NEAREST_TURN_WINDOW_SECS`], else [`SYSTEM_FALLBACK_KEY`] ("Unknown
///    speaker"). **Never** [`LOCAL_SPEAKER_KEY`]: without a channel tag there is
///    no evidence the owner spoke, and a wrong "You" is worse than an honest
///    Unknown (specs/0043 W1.3 — this replaced the P1 "nobody remote spoke ⇒ the
///    owner spoke" heuristic).
///
/// This matches [`align_turns_to_segments`]'s [`Channel::Mixed`] rule exactly
/// (the offline pass maps untagged rows to Mixed since specs/0029 WS3.4); this
/// function remains the live diarizer's alignment (live segments carry no
/// channel tag). Live "You" labels therefore come only from the offline pass at
/// stop, via mic-tagged segments.
///
/// `segments` are `(start, end)` recording-relative seconds; output is one key per
/// segment, in the same order.
pub fn align_system_turns_to_segments(
    turns: &[SpeakerTurn],
    segments: &[(f32, f32)],
) -> Vec<String> {
    segments
        .iter()
        .map(|&(seg_start, seg_end)| {
            let best = turns
                .iter()
                .map(|t| (t, overlap(seg_start, seg_end, t.start, t.end)))
                .filter(|(_, ov)| *ov > 0.0)
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            match best {
                Some((turn, _)) => turn.speaker.clone(),
                // No remote speaker overlapped → nearest turn within the window,
                // else Unknown. Never "You" (specs/0043 W1.3).
                None => nearest_turn_within_window(turns, seg_start, seg_end)
                    .map(|t| t.speaker.clone())
                    .unwrap_or_else(|| SYSTEM_FALLBACK_KEY.to_string()),
            }
        })
        .collect()
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

    #[test]
    fn mic_segments_short_circuit_to_local() {
        // Even with a system turn covering the exact span, a mic segment is local.
        let turns = vec![turn(0.0, 10.0, "spk_0")];
        let segs = vec![AlignableSegment::new(0.0, 5.0, Channel::Microphone)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["local"]
        );
    }

    #[test]
    fn system_segment_takes_max_overlap_turn() {
        // Segment 4.0..6.0 overlaps spk_0 by 1.0s and spk_1 by 1.0s, but spk_1's
        // overlap is larger when we tilt the boundaries.
        let turns = vec![turn(0.0, 5.0, "spk_0"), turn(5.0, 12.0, "spk_1")];
        // 4.5..10.0: overlaps spk_0 by 0.5s, spk_1 by 5.0s -> spk_1 wins.
        let segs = vec![AlignableSegment::new(4.5, 10.0, Channel::System)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["spk_1"]
        );
    }

    #[test]
    fn boundary_drift_is_tolerated() {
        // pyannote turn 2.0..8.0; VAD segment drifts to 1.8..7.5. Still spk_0.
        let turns = vec![turn(2.0, 8.0, "spk_0")];
        let segs = vec![AlignableSegment::new(1.8, 7.5, Channel::System)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["spk_0"]
        );
    }

    #[test]
    fn system_segment_with_no_overlap_falls_back() {
        let turns = vec![turn(0.0, 5.0, "spk_0")];
        // 20.0..25.0 overlaps nothing.
        let segs = vec![AlignableSegment::new(20.0, 25.0, Channel::System)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec![SYSTEM_FALLBACK_KEY]
        );
    }

    #[test]
    fn empty_turns_with_system_segments_all_fall_back() {
        let segs = vec![
            AlignableSegment::new(0.0, 1.0, Channel::System),
            AlignableSegment::new(1.0, 2.0, Channel::System),
        ];
        assert_eq!(
            align_turns_to_segments(&[], &segs, AlignMode::Call),
            vec![SYSTEM_FALLBACK_KEY, SYSTEM_FALLBACK_KEY]
        );
    }

    #[test]
    fn empty_turns_still_label_mic_as_local() {
        let segs = vec![AlignableSegment::new(0.0, 1.0, Channel::Microphone)];
        assert_eq!(
            align_turns_to_segments(&[], &segs, AlignMode::Call),
            vec!["local"]
        );
    }

    #[test]
    fn multi_speaker_conversation_in_order() {
        // Three remote speakers, interleaved; output preserves segment order.
        let turns = vec![
            turn(0.0, 3.0, "spk_0"),
            turn(3.0, 6.0, "spk_1"),
            turn(6.0, 9.0, "spk_2"),
            turn(9.0, 12.0, "spk_0"),
        ];
        let segs = vec![
            AlignableSegment::new(0.2, 2.8, Channel::System), // spk_0
            AlignableSegment::new(3.1, 5.9, Channel::System), // spk_1
            AlignableSegment::new(2.0, 4.0, Channel::Microphone), // local (overlaps both, but mic)
            AlignableSegment::new(6.5, 8.5, Channel::System), // spk_2
            AlignableSegment::new(9.5, 11.5, Channel::System), // spk_0
        ];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["spk_0", "spk_1", "local", "spk_2", "spk_0"]
        );
    }

    // --- system-only P1 attribution (align_system_turns_to_segments) ---

    #[test]
    fn live_no_overlap_far_from_turns_is_unknown() {
        // specs/0043 W1.3: a segment overlapping no turn and far (>1.0s) from every
        // turn is ambiguous -> Unknown, never "You" (was: local fallback).
        let turns = vec![turn(0.0, 5.0, "spk_0")];
        let segs = vec![(20.0, 25.0)];
        assert_eq!(
            align_system_turns_to_segments(&turns, &segs),
            vec![SYSTEM_FALLBACK_KEY]
        );
    }

    #[test]
    fn live_near_turn_within_window_takes_turn_speaker() {
        // No overlap, but the segment starts 0.5s after spk_0's turn ends (and is
        // 0.7s from spk_1's) -> nearest-turn attribution picks spk_0.
        let turns = vec![turn(0.0, 5.0, "spk_0"), turn(7.2, 9.0, "spk_1")];
        let segs = vec![(5.5, 6.5)];
        assert_eq!(align_system_turns_to_segments(&turns, &segs), vec!["spk_0"]);
    }

    #[test]
    fn nearest_turn_window_is_inclusive_at_exactly_the_window() {
        // Gap of exactly NEAREST_TURN_WINDOW_SECS (1.0s) still attributes.
        let turns = vec![turn(0.0, 3.0, "spk_0")];
        let segs = vec![(3.0 + NEAREST_TURN_WINDOW_SECS, 5.0)];
        assert_eq!(align_system_turns_to_segments(&turns, &segs), vec!["spk_0"]);
    }

    #[test]
    fn system_only_overlap_takes_remote_speaker() {
        let turns = vec![turn(0.0, 5.0, "spk_0"), turn(5.0, 12.0, "spk_1")];
        // 4.5..10.0 overlaps spk_1 by 5.0s, spk_0 by 0.5s -> spk_1.
        let segs = vec![(4.5, 10.0)];
        assert_eq!(align_system_turns_to_segments(&turns, &segs), vec!["spk_1"]);
    }

    #[test]
    fn system_only_mixed_conversation() {
        // Two remote speakers with a mid-conversation gap. specs/0043 W1.3: a
        // gap segment close to a turn takes the nearest speaker; one far from
        // every turn is Unknown. (Previously both gaps were "local".)
        let turns = vec![turn(0.0, 3.0, "spk_0"), turn(6.0, 9.0, "spk_1")];
        let segs = vec![
            (0.2, 2.8),   // spk_0 (overlap)
            (3.2, 4.2),   // no overlap; 0.2s after spk_0 -> nearest-turn spk_0
            (6.5, 8.5),   // spk_1 (overlap)
            (20.0, 25.0), // far beyond every turn -> Unknown
        ];
        assert_eq!(
            align_system_turns_to_segments(&turns, &segs),
            vec!["spk_0", "spk_0", "spk_1", SYSTEM_FALLBACK_KEY]
        );
    }

    #[test]
    fn system_only_empty_turns_all_unknown() {
        // No remote speech detected at all -> every segment is ambiguous, so
        // Unknown (specs/0043 W1.3; previously every segment became "local").
        let segs = vec![(0.0, 1.0), (1.0, 2.0)];
        assert_eq!(
            align_system_turns_to_segments(&[], &segs),
            vec![SYSTEM_FALLBACK_KEY, SYSTEM_FALLBACK_KEY]
        );
    }

    // --- specs/0029 WS3.4: per-channel path with Mixed/untagged segments ---

    #[test]
    fn mic_segment_keeps_local_despite_full_system_turn_overlap() {
        // The WS3.4 acceptance case: a system turn covers the mic segment's exact
        // span (e.g. cross-talk / bleed picked up by the diarizer) — the channel
        // tag must win and the segment stays "You".
        let turns = vec![turn(1.0, 4.0, "spk_0")];
        let segs = vec![AlignableSegment::new(1.0, 4.0, Channel::Microphone)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec![LOCAL_SPEAKER_KEY]
        );
    }

    #[test]
    fn system_fallback_is_the_unknown_bucket_not_you() {
        // A system-tagged segment with no overlapping turn is known-not-mic, so it
        // must NOT fall back to the local user; it lands in the WS3.3 overflow
        // bucket ("unknown" → "Unknown speaker").
        let segs = vec![AlignableSegment::new(20.0, 25.0, Channel::System)];
        assert_eq!(
            align_turns_to_segments(&[turn(0.0, 5.0, "spk_0")], &segs, AlignMode::Call),
            vec![crate::diarization::UNKNOWN_SPEAKER_KEY]
        );
    }

    #[test]
    fn mixed_segment_takes_max_overlap_turn() {
        let turns = vec![turn(0.0, 5.0, "spk_0"), turn(5.0, 12.0, "spk_1")];
        let segs = vec![AlignableSegment::new(4.5, 10.0, Channel::Mixed)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["spk_1"]
        );
    }

    // --- specs/0047 W2: owner turns must not win a non-mic segment ---

    #[test]
    fn system_segment_is_not_stolen_by_an_overlapping_owner_turn() {
        // On speakers, mic-bleed produced a "local" owner turn that overlaps a
        // remote segment MORE than the real remote turn (local 3.0s vs spk_0 2.5s).
        // A System-tagged segment is known-remote, so the owner must not win it —
        // it stays with the real remote speaker (only the Microphone channel tag
        // yields "You").
        let turns = vec![turn(0.0, 10.0, LOCAL_SPEAKER_KEY), turn(1.5, 4.0, "spk_0")];
        let segs = vec![AlignableSegment::new(1.0, 4.0, Channel::System)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["spk_0"]
        );
    }

    #[test]
    fn mixed_segment_is_not_stolen_by_an_overlapping_owner_turn() {
        // Same as above for a Mixed-tagged segment (overlapped speech / bleed):
        // it resolves to the remote speaker, never the owner.
        let turns = vec![turn(0.0, 10.0, LOCAL_SPEAKER_KEY), turn(1.5, 4.0, "spk_0")];
        let segs = vec![AlignableSegment::new(1.0, 4.0, Channel::Mixed)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["spk_0"]
        );
    }

    #[test]
    fn system_segment_overlapping_only_an_owner_turn_is_unknown_not_you() {
        // A remote (System-tagged) segment that overlaps ONLY a bleed owner turn
        // must fall back to Unknown, never "You".
        let turns = vec![turn(0.0, 10.0, LOCAL_SPEAKER_KEY)];
        let segs = vec![AlignableSegment::new(1.0, 4.0, Channel::System)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec![SYSTEM_FALLBACK_KEY]
        );
    }

    #[test]
    fn mixed_segment_near_only_an_owner_turn_is_unknown_not_you() {
        // Nearest-turn attribution must also ignore owner turns for a Mixed
        // segment: with only a "local" turn 0.3s away, the result is Unknown.
        let turns = vec![turn(0.0, 5.0, LOCAL_SPEAKER_KEY)];
        let segs = vec![AlignableSegment::new(5.3, 6.5, Channel::Mixed)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec![SYSTEM_FALLBACK_KEY]
        );
    }

    #[test]
    fn owner_turn_still_lets_a_mic_segment_stay_you() {
        // Regression guard: the mic short-circuit is untouched — a Microphone
        // segment is still "You" even though owner turns are now filtered out of
        // the System/Mixed contest.
        let turns = vec![turn(0.0, 10.0, LOCAL_SPEAKER_KEY), turn(1.0, 4.0, "spk_0")];
        let segs = vec![AlignableSegment::new(1.0, 4.0, Channel::Microphone)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec![LOCAL_SPEAKER_KEY]
        );
    }

    #[test]
    fn mixed_segment_far_from_any_turn_is_unknown() {
        // specs/0043 W1.3: Mixed/untagged no longer defaults to "You". No overlap
        // and no turn within the 1.0s window -> Unknown speaker.
        let turns = vec![turn(0.0, 5.0, "spk_0")];
        let segs = vec![AlignableSegment::new(20.0, 25.0, Channel::Mixed)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec![SYSTEM_FALLBACK_KEY]
        );
    }

    #[test]
    fn mixed_segment_near_turn_within_window_takes_turn_speaker() {
        // No overlap, but spk_1's turn starts 0.3s after the segment ends (spk_0
        // ends 0.5s before it starts) -> the nearest turn wins: spk_1.
        let turns = vec![turn(0.0, 5.0, "spk_0"), turn(6.8, 9.0, "spk_1")];
        let segs = vec![AlignableSegment::new(5.5, 6.5, Channel::Mixed)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["spk_1"]
        );
    }

    #[test]
    fn system_segment_near_turn_still_goes_straight_to_unknown() {
        // Deliberate asymmetry (specs/0043 W1.3 scope): nearest-turn attribution
        // applies to Mixed/untagged and live segments only; a System-tagged
        // segment with no overlap keeps its direct Unknown fallback.
        let turns = vec![turn(0.0, 5.0, "spk_0")];
        let segs = vec![AlignableSegment::new(5.5, 6.5, Channel::System)];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec![SYSTEM_FALLBACK_KEY]
        );
    }

    #[test]
    fn legacy_untagged_segments_behave_exactly_like_the_system_only_path() {
        // WS3.4 invariant, still true post-W1.3: a legacy meeting (every row's
        // channel is NULL → mapped to Channel::Mixed) must produce byte-identical
        // assignments to the live path (align_system_turns_to_segments), across
        // overlap, nearest-turn, far-gap-Unknown, and empty-turn cases. (What
        // both paths produce for gaps changed together in specs/0043 W1.3.)
        let turns = vec![
            turn(0.0, 3.0, "spk_0"),
            turn(6.0, 9.0, "spk_1"),
            turn(9.0, 12.0, "spk_0"),
        ];
        let windows: Vec<(f32, f32)> = vec![
            (0.2, 2.8),   // spk_0
            (4.0, 4.9),   // gap; 1.0s after spk_0 (inclusive window) -> spk_0
            (6.5, 8.5),   // spk_1
            (8.8, 11.0),  // straddles spk_1/spk_0 -> max overlap
            (20.0, 25.0), // beyond all turns -> Unknown
        ];
        let legacy = align_system_turns_to_segments(&turns, &windows);
        let via_mixed: Vec<String> = align_turns_to_segments(
            &turns,
            &windows
                .iter()
                .map(|&(s, e)| AlignableSegment::new(s, e, Channel::Mixed))
                .collect::<Vec<_>>(),
            AlignMode::Call,
        );
        assert_eq!(via_mixed, legacy);

        // Empty turn set: every untagged segment is Unknown (specs/0043 W1.3;
        // previously all "local"), both paths.
        let legacy_empty = align_system_turns_to_segments(&[], &windows);
        let via_mixed_empty: Vec<String> = align_turns_to_segments(
            &[],
            &windows
                .iter()
                .map(|&(s, e)| AlignableSegment::new(s, e, Channel::Mixed))
                .collect::<Vec<_>>(),
            AlignMode::Call,
        );
        assert_eq!(via_mixed_empty, legacy_empty);
        assert!(via_mixed_empty.iter().all(|k| k == SYSTEM_FALLBACK_KEY));
    }

    // --- specs/0044 W1.1: pad compensation ---

    #[test]
    fn pad_trimmed_removes_full_pads_on_long_rows() {
        // 5 s row: full 300 ms pre + 400 ms post trim.
        let (s, e) = pad_trimmed(10.0, 15.0);
        assert!((s - 10.3).abs() < 1e-4, "start was {s}");
        assert!((e - 14.6).abs() < 1e-4, "end was {e}");
    }

    #[test]
    fn pad_trimmed_scales_down_on_short_rows_keeping_a_core() {
        // 0.8 s row can't absorb 0.7 s of trim; both trims scale so 250 ms remain.
        let (s, e) = pad_trimmed(2.0, 2.8);
        assert!(e > s, "core must be non-empty");
        assert!((e - s - 0.25).abs() < 1e-4, "core was {}", e - s);
        // Trims keep their 3:4 ratio.
        assert!(((s - 2.0) / (2.8 - e) - 0.75).abs() < 1e-3);
    }

    #[test]
    fn pad_trimmed_leaves_tiny_rows_untouched() {
        // A 0.2 s row is already below the core floor: no trim at all.
        let (s, e) = pad_trimmed(1.0, 1.2);
        assert!((s - 1.0).abs() < 1e-6 && (e - 1.2).abs() < 1e-6);
    }

    #[test]
    fn pad_trimmed_shifts_boundary_attribution_to_the_new_speaker() {
        // The specs/0044 symptom in miniature: spk_1's short reply row is padded
        // 300 ms early / 400 ms late, and the diarizer ran spk_0's turn a touch
        // long — raw overlap favors spk_0 (0.4 s vs 0.3 s); the trimmed core
        // lands on spk_1 (0.1 s vs 0.3 s).
        let turns = vec![turn(0.0, 5.6, "spk_0"), turn(5.6, 5.9, "spk_1")];
        let raw = AlignableSegment::new(5.2, 6.3, Channel::System);
        assert_eq!(
            align_turns_to_segments(&turns, std::slice::from_ref(&raw), AlignMode::Call),
            vec!["spk_0"],
            "without trimming the padded row is misattributed"
        );
        let (ts, te) = pad_trimmed(raw.start, raw.end);
        let trimmed = AlignableSegment::new(ts, te, Channel::System);
        assert_eq!(
            align_turns_to_segments(&turns, &[trimmed], AlignMode::Call),
            vec!["spk_1"]
        );
    }

    #[test]
    fn tie_is_resolved_deterministically_to_first_max() {
        // Equal overlap with two turns: max_by keeps the first maximum encountered.
        let turns = vec![turn(0.0, 4.0, "spk_0"), turn(4.0, 8.0, "spk_1")];
        // 2.0..6.0 overlaps each by exactly 2.0s.
        let segs = vec![AlignableSegment::new(2.0, 6.0, Channel::System)];
        // `max_by` returns the last element among equal maxima; assert the
        // observed deterministic behavior so a future change is caught.
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["spk_1"]
        );
    }

    // ----- specs/0078: room mode -------------------------------------------------

    #[test]
    fn room_mode_attributes_mic_rows_from_the_turns() {
        // Everyone is on the mic: a mic row takes its max-overlap turn, the owner's
        // clustered `local` turn included, instead of short-circuiting to "You".
        let turns = vec![turn(0.0, 5.0, "spk_0"), turn(5.0, 10.0, "local")];
        let segs = vec![
            AlignableSegment::new(0.5, 4.5, Channel::Microphone),
            AlignableSegment::new(5.5, 9.5, Channel::Microphone),
        ];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Room),
            vec!["spk_0", "local"]
        );
        // The same rows in call mode are all "You" (rule 1).
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Call),
            vec!["local", "local"]
        );
    }

    #[test]
    fn room_mode_mic_rows_follow_the_mixed_fallbacks() {
        let turns = vec![turn(0.0, 5.0, "spk_1")];
        let segs = vec![
            // No overlap, but within the nearest-turn window.
            AlignableSegment::new(5.5, 7.0, Channel::Microphone),
            // Far from every turn: the unknown bucket, never "You".
            AlignableSegment::new(30.0, 32.0, Channel::Microphone),
        ];
        assert_eq!(
            align_turns_to_segments(&turns, &segs, AlignMode::Room),
            vec!["spk_1", SYSTEM_FALLBACK_KEY]
        );
    }
}
