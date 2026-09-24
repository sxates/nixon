//! Unit tests for `split.rs` (moved out of the module for specs/0078 to keep it
//! under the file-size cap).

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
