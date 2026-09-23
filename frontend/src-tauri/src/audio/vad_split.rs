//! Bounding the length of a VAD speech segment (specs/0071 W4).
//!
//! Split out of `vad.rs` rather than added to it: that file was at 975 lines with this in it,
//! and the file-size gate (specs/0065) is right that new behaviour belongs in a new module.
//! The split is also genuinely separable — it is pure arithmetic on one finished segment and
//! needs nothing from the VAD session.

use super::vad::SpeechSegment;
use log::info;

/// Longest single speech segment handed to the transcription engine.
///
/// specs/0071 W4. The VAD closes a segment when it hears end-of-speech, so continuously
/// voiced audio — a video, a monologue, a meeting recorded off a room speaker — produced one
/// enormous segment: an offline re-pass of a 114.6s recording emitted a **108.7-second**
/// unit, and the only thing that had ever happened about it was a "possible memory issue"
/// warning past ~62s.
///
/// 30s is Whisper's own encoder window, so a split piece is no worse a transcription unit
/// than a naturally-ended one — and several right-sized units transcribe better than one
/// oversized one.
const MAX_SPEECH_SEGMENT_SECS: f64 = 30.0;

/// How long a single speech run may go without an end-of-speech before it is worth a line in
/// the log. Well above [`MAX_SPEECH_SEGMENT_SECS`], because a run being long is normal now —
/// it just gets split. This threshold is about audio that is *never* going to end.
pub(super) const LONG_RUN_LOG_SECS: f64 = 60.0;

/// Split one VAD speech run into pieces of at most [`MAX_SPEECH_SEGMENT_SECS`].
///
/// Deliberately applied at EMISSION rather than by cutting `current_speech` mid-run. The
/// `SpeechEnd` transition carries its own `samples` *and* `current_speech` accumulates in
/// parallel, so cutting mid-run means reconciling two accumulations and re-deriving
/// timestamps against two different origins — which is how specs/0046 and 0051 both ended up
/// with scrambled transcript clocks. Here there is exactly one buffer and one timespan, and
/// the split is arithmetic on both.
///
/// The pieces are contiguous and together cover the original span exactly: piece boundaries
/// are interpolated from sample position, so no audio is dropped and no timestamp is invented.
/// A run at or under the cap comes back as itself, untouched and un-reallocated.
pub(super) fn split_long_segment(segment: SpeechSegment, sample_rate_hz: f64) -> Vec<SpeechSegment> {
    let max_samples = (MAX_SPEECH_SEGMENT_SECS * sample_rate_hz) as usize;
    if max_samples == 0 || segment.samples.len() <= max_samples {
        return vec![segment];
    }

    let total = segment.samples.len();
    let span_ms = segment.end_timestamp_ms - segment.start_timestamp_ms;
    // Interpolate from sample position rather than assuming the nominal rate: the span and
    // the sample count are both authoritative and can disagree by a rounding window.
    let ms_at = |offset: usize| {
        segment.start_timestamp_ms + span_ms * (offset as f64 / total as f64)
    };

    let mut pieces = Vec::with_capacity(total.div_ceil(max_samples));
    let mut offset = 0usize;
    while offset < total {
        let end = (offset + max_samples).min(total);
        pieces.push(SpeechSegment {
            samples: segment.samples[offset..end].to_vec(),
            start_timestamp_ms: ms_at(offset),
            // The last piece ends exactly where the original did, so the run's end is never
            // moved by rounding.
            end_timestamp_ms: if end == total { segment.end_timestamp_ms } else { ms_at(end) },
            confidence: segment.confidence,
        });
        offset = end;
    }

    info!(
        "VAD: split a {:.1}s speech run into {} segments (cap {:.0}s)",
        span_ms / 1000.0,
        pieces.len(),
        MAX_SPEECH_SEGMENT_SECS
    );
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(secs: f64, start_ms: f64, rate: f64) -> SpeechSegment {
        SpeechSegment {
            samples: vec![0.25f32; (secs * rate) as usize],
            start_timestamp_ms: start_ms,
            end_timestamp_ms: start_ms + secs * 1000.0,
            confidence: 0.9,
        }
    }

    const RATE: f64 = 16_000.0;

    #[test]
    fn a_short_run_is_returned_untouched() {
        let original = seg(5.0, 1_000.0, RATE);
        let expected = original.samples.len();
        let pieces = split_long_segment(original, RATE);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].samples.len(), expected);
        assert_eq!(pieces[0].start_timestamp_ms, 1_000.0);
        assert_eq!(pieces[0].end_timestamp_ms, 6_000.0);
    }

    #[test]
    fn a_run_exactly_at_the_cap_is_not_split() {
        let pieces = split_long_segment(seg(MAX_SPEECH_SEGMENT_SECS, 0.0, RATE), RATE);
        assert_eq!(pieces.len(), 1, "the cap is inclusive");
    }

    /// The reported case: a 108.7-second run from an offline re-pass.
    #[test]
    fn the_reported_run_is_split_into_transcription_sized_pieces() {
        let pieces = split_long_segment(seg(108.7, 0.0, RATE), RATE);
        assert_eq!(pieces.len(), 4, "108.7s / 30s cap");
        for p in &pieces {
            let secs = p.samples.len() as f64 / RATE;
            assert!(
                secs <= MAX_SPEECH_SEGMENT_SECS + 1e-6,
                "piece is {secs}s, over the {MAX_SPEECH_SEGMENT_SECS}s cap"
            );
        }
    }

    /// The guard against the hazard that made this an emission-time split rather than a
    /// mid-run cut: pieces must tile the original span with no gap, no overlap, and no
    /// invented time. specs/0046 and 0051 were both scrambled transcript clocks.
    #[test]
    fn pieces_tile_the_original_span_exactly() {
        let original = seg(95.0, 7_500.0, RATE);
        let (start, end, total) = (
            original.start_timestamp_ms,
            original.end_timestamp_ms,
            original.samples.len(),
        );
        let pieces = split_long_segment(original, RATE);

        assert_eq!(pieces.first().unwrap().start_timestamp_ms, start, "starts where it started");
        assert_eq!(pieces.last().unwrap().end_timestamp_ms, end, "ends where it ended");
        assert_eq!(
            pieces.iter().map(|p| p.samples.len()).sum::<usize>(),
            total,
            "no audio dropped or duplicated"
        );
        for pair in pieces.windows(2) {
            assert_eq!(
                pair[0].end_timestamp_ms, pair[1].start_timestamp_ms,
                "contiguous: no gap and no overlap between pieces"
            );
        }
        for p in &pieces {
            assert!(p.end_timestamp_ms > p.start_timestamp_ms, "every piece advances");
            assert!(p.start_timestamp_ms >= start && p.end_timestamp_ms <= end, "inside the span");
        }
    }

    #[test]
    fn the_samples_themselves_are_preserved_in_order() {
        // Distinct values per sample so a reordering or a duplicated slice is detectable.
        let total = (70.0 * RATE) as usize;
        let original = SpeechSegment {
            samples: (0..total).map(|i| i as f32).collect(),
            start_timestamp_ms: 0.0,
            end_timestamp_ms: 70_000.0,
            confidence: 0.9,
        };
        let rejoined: Vec<f32> = split_long_segment(original, RATE)
            .into_iter()
            .flat_map(|p| p.samples)
            .collect();
        assert_eq!(rejoined, (0..total).map(|i| i as f32).collect::<Vec<f32>>());
    }

    #[test]
    fn confidence_rides_along_so_a_forced_end_stays_marked_as_one() {
        // flush() emits at 0.8 ("estimated confidence for forced end"); the split must not
        // launder that into a normal 0.9 segment.
        let mut original = seg(80.0, 0.0, RATE);
        original.confidence = 0.8;
        for p in split_long_segment(original, RATE) {
            assert_eq!(p.confidence, 0.8);
        }
    }

    #[test]
    fn a_non_16k_rate_caps_on_seconds_not_samples() {
        let pieces = split_long_segment(seg(90.0, 0.0, 48_000.0), 48_000.0);
        assert_eq!(pieces.len(), 3, "90s at 48kHz is still 3 x 30s");
        for p in &pieces {
            assert!(p.samples.len() as f64 / 48_000.0 <= MAX_SPEECH_SEGMENT_SECS + 1e-6);
        }
    }

    #[test]
    fn an_empty_run_does_not_panic() {
        let empty = SpeechSegment {
            samples: Vec::new(),
            start_timestamp_ms: 0.0,
            end_timestamp_ms: 0.0,
            confidence: 0.9,
        };
        assert_eq!(split_long_segment(empty, RATE).len(), 1);
    }
}
