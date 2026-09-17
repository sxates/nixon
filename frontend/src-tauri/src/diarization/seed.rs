//! specs/0050 — audio-derived speaker-count seed.
//!
//! The 2026-08-09 diarization DER review (a design record, see CONTRIBUTING) showed clustering
//! is near-optimal and the whole lever is the `AtMost(n)` seed. The calendar invite is
//! an unreliable `n` (a distribution-list invite under-counts; a big optional invite
//! over-counts), so we derive the count from the AUDIO instead: how many clusters
//! actually spoke enough to be a real speaker.

use crate::diarization::SpeakerTurn;

/// A cluster counts as a "real speaker" once its POOLED speech reaches this many
/// seconds. Chosen on the ground-truth DER harness (`eval_seed_simulation`): at 10 s
/// the estimate is 2 / 19 / 4 vs true 2 / 20 / 4, and `AtMost(n_audio)` matches the
/// AtMost(true-n) oracle DER on all three labelled meetings. Erring LOW is the safe
/// direction — a slightly high estimate just leaves a non-binding upper bound, whereas
/// a low one would force-merge real speakers.
pub const N_AUDIO_MIN_SECS: f32 = 10.0;

/// The number of distinct clusters whose POOLED speech duration is at least
/// `min_secs` — who actually spoke enough to be a real speaker, independent of any
/// invite. Clamped to at least 2 (a multi-cluster result should never seed one
/// speaker). Pure; validated on the DER harness (`eval_seed_simulation`).
pub fn estimate_speakers_by_duration(turns: &[SpeakerTurn], min_secs: f32) -> usize {
    let mut dur: std::collections::HashMap<&str, f32> = std::collections::HashMap::new();
    for t in turns {
        *dur.entry(t.speaker.as_str()).or_insert(0.0) += t.duration();
    }
    dur.values().filter(|&&d| d >= min_secs).count().max(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(speaker: &str, secs: f32) -> SpeakerTurn {
        SpeakerTurn {
            start: 0.0,
            end: secs,
            speaker: speaker.to_string(),
        }
    }

    #[test]
    fn counts_only_clusters_above_the_duration_floor() {
        // spk_0 speaks a lot, spk_1 a little; a 2 s sliver is below the floor.
        let turns = vec![
            turn("spk_0", 40.0),
            turn("spk_1", 15.0),
            turn("spk_2", 2.0), // sliver — excluded at 10 s
        ];
        assert_eq!(estimate_speakers_by_duration(&turns, 10.0), 2);
        // Lower the floor and the sliver counts.
        assert_eq!(estimate_speakers_by_duration(&turns, 1.0), 3);
    }

    #[test]
    fn pools_a_speaker_across_turns() {
        // Two 6 s turns pool to 12 s ≥ 10 s → one real speaker.
        let turns = vec![turn("spk_0", 6.0), turn("spk_0", 6.0), turn("spk_1", 3.0)];
        assert_eq!(estimate_speakers_by_duration(&turns, 10.0), 2); // clamped to >= 2
    }

    #[test]
    fn clamps_to_at_least_two() {
        let turns = vec![turn("spk_0", 100.0)];
        assert_eq!(estimate_speakers_by_duration(&turns, 10.0), 2);
    }
}
