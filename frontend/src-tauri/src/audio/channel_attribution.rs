//! Capture-channel attribution: which of the two pre-mix tracks was speaking
//! (specs/0029 WS3.4, /0043 W1.5, /0055).
//!
//! The pipeline mixes mic + system before STT, destroying channel identity. The
//! mixer sees the CLEAN, SEPARATED tracks for every 600 ms window beforehand,
//! which is the one place that identity still exists. We classify each window by
//! RMS dominance, then expose two views of the result for a VAD speech segment
//! (which may span many windows):
//!
//! - [`dominant_channel_for_span`] — one `ChannelTag` for the whole segment, the
//!   value persisted as `transcripts.channel`.
//! - [`channel_runs_for_span`] — the same evidence at window resolution
//!   (specs/0055). 32% of rows in a measured real meeting straddle an
//!   owner<->remote handoff, so the single tag necessarily mislabels one side of
//!   those; the runs keep the boundary the vote discards.
//!
//! Pure and model-free, so the tests run unconditionally.

use std::collections::VecDeque;

use super::common::{ChannelRun, ChannelTag};

// ---------------------------------------------------------------------------
// specs/0029 WS3.4 — per-window capture-channel dominance classification.
//
// The mixer sees the CLEAN, SEPARATED mic and system tracks for every 600 ms
// window before they are summed, which is the one place channel identity still
// exists. We classify each window by RMS dominance and, when a VAD speech
// segment is emitted (it may span many windows), aggregate the overlapping
// windows' classes into one `ChannelTag` that rides the transcription chunk.
// ---------------------------------------------------------------------------

/// RMS below which a track is considered inactive for channel attribution
/// (≈ −40 dBFS). Normalized mic speech sits around −23 LUFS (RMS ≈ 0.05–0.2) and
/// system speech is comparable, so real speech clears this comfortably while an
/// idle track's noise floor stays under it. Windows where BOTH tracks are below
/// this are unclassified (silence) and excluded from the segment vote.
const CHANNEL_ACTIVE_RMS: f32 = 0.01;

/// RMS ratio one track must have over the other (when both are active) to be
/// called dominant (≈ 9.5 dB). Large enough that acoustic bleed — speaker
/// playback picked up by the mic, or a headset side-tone — doesn't flip a
/// clearly one-sided window to `Mixed`; genuinely overlapped speech (both
/// parties talking) lands within the ratio and is tagged `Mixed`.
const CHANNEL_DOMINANCE_RMS_RATIO: f32 = 3.0;

/// specs/0043 W1.5 — bleed guard: system-track RMS at or above which a window
/// that would otherwise classify as Microphone-dominant is demoted to `Mixed`.
///
/// Why the dominance ratio alone isn't enough on speakers (no headphones): the
/// mic path is EBU R128-normalized to −23 LUFS, so when the mic hears ONLY the
/// laptop speakers (remote voice → air → mic), the normalizer boosts that bleed
/// back up to speech-like RMS. The 3× ratio then compares a *normalized* echo
/// against the raw system track and can happily call the echo "mic-dominant" —
/// 9.5 dB of RMS dominance carries no information about *whose voice* it is.
/// Genuine solo local speech, by contrast, has a near-silent system track: the
/// tap is a clean digital copy of process output, so with nobody remote talking
/// it sits at/near digital zero, far below this bar.
///
/// The bar is 5× [`CHANNEL_ACTIVE_RMS`] (0.05 ≈ −26 dBFS) — the bottom of the
/// speech-loudness RMS band — NOT merely "active" (0.01): the system must be
/// playing something at *talking* loudness simultaneously for the mic reading
/// to be suspect. Below it (notification dings, quiet music beds, comfort
/// noise) real local speech keeps its Microphone tag; only strong simultaneous
/// playback demotes. Demotion is safe post-W1.3: `Mixed` resolves via diarizer
/// turns / Unknown instead of being hard-labeled "You" (align.rs Rule 1).
///
/// Deliberately asymmetric — system-dominant windows are NEVER demoted this
/// way: the system channel is a clean digital tap, so local mic sound cannot
/// bleed into it on this machine. (A remote participant's device echoing the
/// owner's voice back is a remote-side artifact this classifier can't observe
/// or fix.) A loud mic under a system-dominant window is just overlapped
/// speech, which the dominance ratio already handles.
const CHANNEL_BLEED_SYSTEM_RMS: f32 = CHANNEL_ACTIVE_RMS * 5.0;

/// Overlap-time ratio the leading channel must have over the other across a VAD
/// segment's windows to win the segment (2× — a clear majority of the speech
/// time, not a coin flip). Otherwise the segment is `Mixed`.
const SEGMENT_CHANNEL_DOMINANCE_RATIO: f64 = 2.0;

/// Bound on the per-window channel history (~10 minutes of 600 ms windows).
/// VAD segments normally close within seconds; a pathological single segment
/// longer than this just loses its oldest windows from the vote. Memory is
/// negligible (3 words per entry).
pub(crate) const CHANNEL_WINDOW_HISTORY_MAX: usize = 1024;

/// Classify one pre-mix window pair by RMS dominance. `None` = both tracks
/// inactive (silence — contributes nothing to a segment's vote).
pub(crate) fn classify_window_channel(
    mic_window: &[f32],
    sys_window: &[f32],
) -> Option<ChannelTag> {
    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }
    let mic = rms(mic_window);
    let sys = rms(sys_window);
    match (mic >= CHANNEL_ACTIVE_RMS, sys >= CHANNEL_ACTIVE_RMS) {
        (false, false) => None,
        (true, false) => Some(ChannelTag::Microphone),
        (false, true) => Some(ChannelTag::System),
        (true, true) => {
            if mic >= sys * CHANNEL_DOMINANCE_RMS_RATIO {
                // specs/0043 W1.5 bleed guard: mic-dominant, but the system track
                // is simultaneously at speech loudness — the "mic" energy may be
                // normalized speaker bleed (echo) of that very playback, which RMS
                // dominance cannot distinguish from real local speech. Don't hard-
                // claim Microphone; let Mixed resolve via diarizer turns (W1.3).
                if sys >= CHANNEL_BLEED_SYSTEM_RMS {
                    Some(ChannelTag::Mixed)
                } else {
                    Some(ChannelTag::Microphone)
                }
            } else if sys >= mic * CHANNEL_DOMINANCE_RMS_RATIO {
                Some(ChannelTag::System)
            } else {
                Some(ChannelTag::Mixed)
            }
        }
    }
}

/// Aggregate the dominant channel for a VAD segment spanning `[start_ms, end_ms)`
/// over the classified window history (each entry: window start/end ms in the same
/// VAD-fed time base, plus its class). Overlap-time weighted: the winning channel
/// needs a [`SEGMENT_CHANNEL_DOMINANCE_RATIO`] majority of the classified overlap,
/// else the segment is `Mixed`. `None` when no classified window overlaps (e.g. a
/// segment made entirely of silence-class windows — shouldn't happen for VAD
/// speech, but degrade to "untagged" rather than guessing).
pub(crate) fn dominant_channel_for_span(
    windows: &VecDeque<(f64, f64, ChannelTag)>,
    start_ms: f64,
    end_ms: f64,
) -> Option<ChannelTag> {
    let mut mic_ms = 0.0f64;
    let mut sys_ms = 0.0f64;
    let mut mixed_ms = 0.0f64;
    for &(w_start, w_end, class) in windows {
        let overlap = (end_ms.min(w_end) - start_ms.max(w_start)).max(0.0);
        if overlap <= 0.0 {
            continue;
        }
        match class {
            ChannelTag::Microphone => mic_ms += overlap,
            ChannelTag::System => sys_ms += overlap,
            ChannelTag::Mixed => mixed_ms += overlap,
        }
    }
    let total = mic_ms + sys_ms + mixed_ms;
    if total <= 0.0 {
        return None;
    }
    if mixed_ms > mic_ms && mixed_ms > sys_ms {
        return Some(ChannelTag::Mixed);
    }
    if mic_ms >= sys_ms * SEGMENT_CHANNEL_DOMINANCE_RATIO {
        Some(ChannelTag::Microphone)
    } else if sys_ms >= mic_ms * SEGMENT_CHANNEL_DOMINANCE_RATIO {
        Some(ChannelTag::System)
    } else {
        Some(ChannelTag::Mixed)
    }
}

/// specs/0055 — the channel evidence for `[start_ms, end_ms)` at **window
/// resolution**, instead of [`dominant_channel_for_span`]'s single verdict.
///
/// Returns time-ordered [`ChannelRun`]s in recording-relative **seconds** that
/// **tile the span contiguously** (no holes), so a caller can apportion the
/// segment's text across them. Adjacent windows of the same class merge into one
/// run; unclassified (silence) windows are never recorded, and the gaps they
/// leave are absorbed by the surrounding runs rather than becoming holes. Empty
/// when no classified window overlaps the span — the same condition under which
/// [`dominant_channel_for_span`] returns `None`.
pub(crate) fn channel_runs_for_span(
    windows: &VecDeque<(f64, f64, ChannelTag)>,
    start_ms: f64,
    end_ms: f64,
) -> Vec<ChannelRun> {
    if end_ms <= start_ms {
        return Vec::new();
    }
    // Clip every classified window to the span. Sorted explicitly rather than
    // trusting the deque's push order, because merging is order-dependent (unlike
    // `dominant_channel_for_span`, which just sums).
    let mut clipped: Vec<(f64, f64, ChannelTag)> = windows
        .iter()
        .filter_map(|&(w_start, w_end, tag)| {
            let s = w_start.max(start_ms);
            let e = w_end.min(end_ms);
            (e > s).then_some((s, e, tag))
        })
        .collect();
    clipped.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut runs: Vec<ChannelRun> = Vec::new();
    for (s, e, tag) in clipped {
        match runs.last_mut() {
            Some(last) if last.tag == tag => last.end = last.end.max(e / 1000.0),
            _ => runs.push(ChannelRun {
                start: s / 1000.0,
                end: e / 1000.0,
                tag,
            }),
        }
    }
    if runs.is_empty() {
        return runs;
    }
    // Close the holes left by unrecorded (silence) windows so the runs tile the
    // span: each run extends to where the next begins, and the ends reach the
    // span's own bounds. A caller apportioning text needs a partition, not a
    // sparse cover.
    for i in 0..runs.len() - 1 {
        runs[i].end = runs[i + 1].start;
    }
    let last = runs.len() - 1;
    runs[0].start = start_ms / 1000.0;
    runs[last].end = end_ms / 1000.0;
    runs
}

#[cfg(test)]
mod channel_attribution_tests {
    //! specs/0029 WS3.4 — per-window dominance classifier + per-segment aggregation.

    use super::*;

    /// A window of constant absolute amplitude `level` (RMS == level).
    fn tone(level: f32) -> Vec<f32> {
        vec![level; 480]
    }

    // --- classify_window_channel ---

    #[test]
    fn mic_only_window_is_microphone() {
        let mic = tone(0.1); // typical normalized speech RMS
        let sys = tone(0.0);
        assert_eq!(
            classify_window_channel(&mic, &sys),
            Some(ChannelTag::Microphone)
        );
    }

    #[test]
    fn system_only_window_is_system() {
        let mic = tone(0.002); // idle mic noise floor, below CHANNEL_ACTIVE_RMS
        let sys = tone(0.1);
        assert_eq!(
            classify_window_channel(&mic, &sys),
            Some(ChannelTag::System)
        );
    }

    #[test]
    fn both_active_similar_levels_is_mixed() {
        let mic = tone(0.1);
        let sys = tone(0.08); // within the 3x dominance ratio
        assert_eq!(classify_window_channel(&mic, &sys), Some(ChannelTag::Mixed));
    }

    #[test]
    fn both_active_with_clear_dominance_picks_the_louder_track() {
        // System speech + mic bleed (speaker playback picked up by the mic):
        // 10x RMS gap clears the 3x dominance ratio → System, not Mixed.
        let mic = tone(0.015);
        let sys = tone(0.15);
        assert_eq!(
            classify_window_channel(&mic, &sys),
            Some(ChannelTag::System)
        );
        // And the mirror case.
        assert_eq!(
            classify_window_channel(&sys, &mic),
            Some(ChannelTag::Microphone)
        );
    }

    // --- specs/0043 W1.5 bleed guard ---

    #[test]
    fn solo_mic_with_quiet_system_stays_microphone() {
        // Genuine solo local speech: system active (a low bed — ding tail,
        // quiet music) but below the CHANNEL_BLEED_SYSTEM_RMS speech bar.
        // The bleed guard must NOT demote this — the failure mode to avoid.
        let mic = tone(0.1);
        let sys = tone(0.02); // active (≥ 0.01) but well under the 0.05 bleed bar
        assert_eq!(
            classify_window_channel(&mic, &sys),
            Some(ChannelTag::Microphone)
        );
    }

    #[test]
    fn mic_dominant_with_loud_simultaneous_system_is_demoted_to_mixed() {
        // Speakers, no headphones: system plays remote speech at talking
        // loudness while the normalized mic track reads 5x louder. That "mic"
        // energy may BE the playback (echo) — demote to Mixed instead of
        // hard-claiming Microphone (post-W1.3, Mixed resolves via diarizer).
        let mic = tone(0.3);
        let sys = tone(0.06); // ≥ CHANNEL_BLEED_SYSTEM_RMS, and mic ≥ 3x sys
        assert_eq!(classify_window_channel(&mic, &sys), Some(ChannelTag::Mixed));
    }

    #[test]
    fn bleed_guard_boundary_at_channel_bleed_system_rms() {
        // Just above the bar → demoted; just below → still Microphone.
        // (1% margins avoid asserting exact f32 RMS round-tripping.)
        let mic = tone(0.3); // ≥ 3x sys in both cases → mic-dominant pre-guard
        assert_eq!(
            classify_window_channel(&mic, &tone(CHANNEL_BLEED_SYSTEM_RMS * 1.01)),
            Some(ChannelTag::Mixed)
        );
        assert_eq!(
            classify_window_channel(&mic, &tone(CHANNEL_BLEED_SYSTEM_RMS * 0.99)),
            Some(ChannelTag::Microphone)
        );
    }

    #[test]
    fn genuine_overlap_with_loud_system_is_still_mixed() {
        // Both tracks loud and within the dominance ratio: was Mixed before
        // the bleed guard, stays Mixed after it.
        let mic = tone(0.12);
        let sys = tone(0.1);
        assert_eq!(classify_window_channel(&mic, &sys), Some(ChannelTag::Mixed));
    }

    #[test]
    fn system_dominant_with_loud_mic_is_not_demoted() {
        // Asymmetry (specs/0043 W1.5): the system channel is a clean digital
        // tap — the mic cannot bleed into it locally — so a system-dominant
        // window is never demoted, however loud the mic track is.
        let mic = tone(0.05); // at the bleed bar level, active
        let sys = tone(0.3); // ≥ 3x mic → System stays System
        assert_eq!(
            classify_window_channel(&mic, &sys),
            Some(ChannelTag::System)
        );
    }

    #[test]
    fn silence_on_both_tracks_is_unclassified() {
        let mic = tone(0.001);
        let sys = tone(0.0);
        assert_eq!(classify_window_channel(&mic, &sys), None);
        assert_eq!(classify_window_channel(&[], &[]), None);
    }

    // --- dominant_channel_for_span ---

    fn history(entries: &[(f64, f64, ChannelTag)]) -> VecDeque<(f64, f64, ChannelTag)> {
        entries.iter().copied().collect()
    }

    #[test]
    fn segment_over_mic_windows_is_microphone() {
        let h = history(&[
            (0.0, 600.0, ChannelTag::Microphone),
            (600.0, 1200.0, ChannelTag::Microphone),
        ]);
        assert_eq!(
            dominant_channel_for_span(&h, 100.0, 1100.0),
            Some(ChannelTag::Microphone)
        );
    }

    #[test]
    fn segment_over_system_windows_is_system() {
        let h = history(&[
            (0.0, 600.0, ChannelTag::System),
            (600.0, 1200.0, ChannelTag::System),
        ]);
        assert_eq!(
            dominant_channel_for_span(&h, 0.0, 1200.0),
            Some(ChannelTag::System)
        );
    }

    #[test]
    fn segment_split_evenly_between_channels_is_mixed() {
        // 600 ms of mic + 600 ms of system: neither has a 2x majority.
        let h = history(&[
            (0.0, 600.0, ChannelTag::Microphone),
            (600.0, 1200.0, ChannelTag::System),
        ]);
        assert_eq!(
            dominant_channel_for_span(&h, 0.0, 1200.0),
            Some(ChannelTag::Mixed)
        );
    }

    #[test]
    fn clear_time_majority_wins_the_segment() {
        // 1800 ms mic vs 600 ms system: 3x >= the 2x majority → Microphone.
        let h = history(&[
            (0.0, 600.0, ChannelTag::Microphone),
            (600.0, 1200.0, ChannelTag::Microphone),
            (1200.0, 1800.0, ChannelTag::Microphone),
            (1800.0, 2400.0, ChannelTag::System),
        ]);
        assert_eq!(
            dominant_channel_for_span(&h, 0.0, 2400.0),
            Some(ChannelTag::Microphone)
        );
    }

    #[test]
    fn mostly_mixed_windows_yield_mixed() {
        let h = history(&[
            (0.0, 600.0, ChannelTag::Mixed),
            (600.0, 1200.0, ChannelTag::Mixed),
            (1200.0, 1800.0, ChannelTag::Microphone),
        ]);
        assert_eq!(
            dominant_channel_for_span(&h, 0.0, 1800.0),
            Some(ChannelTag::Mixed)
        );
    }

    #[test]
    fn segment_with_no_overlapping_windows_is_untagged() {
        // Silence windows are never recorded, so a span with no classified
        // overlap yields None (persisted as NULL, like legacy rows).
        let h = history(&[(0.0, 600.0, ChannelTag::Microphone)]);
        assert_eq!(dominant_channel_for_span(&h, 5000.0, 6000.0), None);
        assert_eq!(dominant_channel_for_span(&history(&[]), 0.0, 600.0), None);
    }

    // --- channel_runs_for_span (specs/0055) ---

    fn runs(v: &[ChannelRun]) -> Vec<(f64, f64, ChannelTag)> {
        v.iter().map(|r| (r.start, r.end, r.tag)).collect()
    }

    #[test]
    fn a_span_of_one_class_collapses_to_a_single_clipped_run() {
        let h = history(&[
            (0.0, 600.0, ChannelTag::Microphone),
            (600.0, 1200.0, ChannelTag::Microphone),
        ]);
        // Clipped to the span, in seconds, merged across the window boundary.
        assert_eq!(
            runs(&channel_runs_for_span(&h, 100.0, 1100.0)),
            vec![(0.1, 1.1, ChannelTag::Microphone)]
        );
    }

    #[test]
    fn an_owner_to_remote_handoff_yields_two_runs_split_at_the_boundary() {
        // The specs/0055 case: one VAD row, owner then remote. The single-tag
        // path calls this whole row "microphone"; runs keep the boundary.
        let h = history(&[
            (0.0, 600.0, ChannelTag::Microphone),
            (600.0, 1200.0, ChannelTag::Mixed),
        ]);
        // The single-tag path calls this ENTIRE row "microphone" (mic 600 ms vs
        // system 0 ms clears the 2x rule), so align.rs Rule 1 hard-labels the
        // remote half "You" — the specs/0055 symptom in miniature.
        assert_eq!(
            dominant_channel_for_span(&h, 0.0, 1200.0),
            Some(ChannelTag::Microphone)
        );
        assert_eq!(
            runs(&channel_runs_for_span(&h, 0.0, 1200.0)),
            vec![
                (0.0, 0.6, ChannelTag::Microphone),
                (0.6, 1.2, ChannelTag::Mixed),
            ]
        );
    }

    #[test]
    fn a_silence_gap_between_same_class_windows_does_not_break_the_run() {
        // Silence windows are never recorded; the hole they leave must be
        // absorbed, not emitted as a gap.
        let h = history(&[
            (0.0, 600.0, ChannelTag::Microphone),
            (1200.0, 1800.0, ChannelTag::Microphone),
        ]);
        assert_eq!(
            runs(&channel_runs_for_span(&h, 0.0, 1800.0)),
            vec![(0.0, 1.8, ChannelTag::Microphone)]
        );
    }

    #[test]
    fn runs_tile_the_span_contiguously_across_a_silence_gap_between_classes() {
        // Different classes either side of an unrecorded gap: the earlier run
        // extends to meet the later one so the tiling has no hole.
        let h = history(&[
            (0.0, 600.0, ChannelTag::Microphone),
            (1200.0, 1800.0, ChannelTag::System),
        ]);
        let got = channel_runs_for_span(&h, 0.0, 1800.0);
        assert_eq!(
            runs(&got),
            vec![
                (0.0, 1.2, ChannelTag::Microphone),
                (1.2, 1.8, ChannelTag::System),
            ]
        );
        // Explicit tiling invariant: first starts at the span start, last ends at
        // the span end, and every run begins where the previous one ended.
        assert_eq!(got.first().unwrap().start, 0.0);
        assert_eq!(got.last().unwrap().end, 1.8);
        for pair in got.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn a_span_with_no_classified_windows_has_no_runs() {
        let h = history(&[(0.0, 600.0, ChannelTag::Microphone)]);
        assert!(channel_runs_for_span(&h, 5000.0, 6000.0).is_empty());
        assert!(channel_runs_for_span(&history(&[]), 0.0, 600.0).is_empty());
    }

    #[test]
    fn partial_overlap_is_weighted_by_time() {
        // Segment 300..1500: 300 ms of mic (0..600 tail) + 600 ms of system +
        // 300 ms of mic again → 600 mic vs 600 sys → Mixed.
        let h = history(&[
            (0.0, 600.0, ChannelTag::Microphone),
            (600.0, 1200.0, ChannelTag::System),
            (1200.0, 1800.0, ChannelTag::Microphone),
        ]);
        assert_eq!(
            dominant_channel_for_span(&h, 300.0, 1500.0),
            Some(ChannelTag::Mixed)
        );
    }
}
