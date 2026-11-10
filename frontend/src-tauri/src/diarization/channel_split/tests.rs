//! Unit tests for `channel_split.rs` (specs/0055 Stage 2).

use super::*;
use crate::audio::common::ChannelTag::{Microphone as M, Mixed as X, System as S};

/// A timeline of back-to-back 600 ms windows starting at `t0` seconds.
fn timeline(t0: f64, tags: &[ChannelTag]) -> ChannelTimeline {
    ChannelTimeline::from_windows(
        tags.iter()
            .enumerate()
            .map(|(i, &t)| {
                let s = (t0 + i as f64 * 0.6) * 1000.0;
                (s, s + 600.0, t)
            })
            .collect(),
    )
}

fn stamps(words: &[(&str, f32, f32)]) -> Vec<WordStamp> {
    words
        .iter()
        .map(|&(w, s, e)| WordStamp {
            text: w.to_string(),
            start: s,
            end: e,
        })
        .collect()
}

#[test]
fn remote_tail_then_owner_onset_cuts_and_tags_the_owner() {
    // 0–2.4 s remote (mixed, the common tag), 2.4–4.8 s the owner.
    let tl = timeline(0.0, &[X, X, X, X, M, M, M, M]);
    let words = stamps(&[
        ("that's", 0.2, 0.6),
        ("the", 0.7, 0.9),
        ("plan.", 1.0, 1.8),
        ("I", 2.5, 2.6),
        ("disagree", 2.7, 3.3),
        ("though.", 3.4, 4.2),
    ]);
    let text = "that's the plan. I disagree though.";
    let parts = plan_channel_split(0.0, 4.8, text, &tl, Some(&words)).expect("split");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].text, "that's the plan.");
    assert_eq!(parts[0].channel.as_deref(), Some("mixed"));
    assert_eq!(parts[1].text, "I disagree though.");
    assert_eq!(parts[1].channel.as_deref(), Some("microphone"));
    assert!((parts[0].end - 2.4).abs() < 1e-4 && parts[1].start == parts[0].end);
    // Each part keeps its own word stamps, so the turn split after this stays exact.
    assert_eq!(parts[0].words.as_ref().unwrap().len(), 3);
    assert_eq!(parts[1].words.as_ref().unwrap()[0].text, "I");
}

#[test]
fn owner_then_remote_onset_never_tags_the_remote_part_microphone() {
    let tl = timeline(0.0, &[M, M, M, S, S, S]);
    let parts =
        plan_channel_split(0.0, 3.6, "yes go ahead sure so the numbers", &tl, None).expect("split");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].channel.as_deref(), Some("microphone"));
    assert_eq!(parts[1].channel.as_deref(), Some("system"));
}

#[test]
fn brief_owner_blip_inside_remote_speech_keeps_the_row_whole() {
    // One mic-classified window (bleed during a dip) in the middle of remote speech.
    let tl = timeline(0.0, &[X, X, X, M, X, X, X]);
    assert!(plan_channel_split(0.0, 4.2, "a b c d e f g h", &tl, None).is_none());
}

#[test]
fn brief_remote_between_owner_speech_is_folded_into_the_owner() {
    // A 600 ms backchannel/overlap window inside the owner's own row must not hand the
    // owner's surrounding words to a remote speaker (specs/0055 Stage 2 review).
    let tl = timeline(0.0, &[M, M, M, S, M, M, M]);
    assert!(plan_channel_split(
        0.0,
        4.2,
        "one two three four five six seven eight",
        &tl,
        None
    )
    .is_none());
    assert!(is_single_side(0.0, 4.2, &tl));
}

#[test]
fn brief_remote_onset_at_the_row_end_still_cuts() {
    // Not interior: the next speaker's first 600 ms at the end of the owner's row is
    // exactly the handoff this split exists for, so it is NOT demoted.
    let tl = timeline(0.0, &[M, M, M, M, M, S]);
    let parts = plan_channel_split(0.0, 3.6, "one two three four five six seven", &tl, None)
        .expect("split");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[1].channel.as_deref(), Some("system"));
    assert!(!is_single_side(0.0, 3.6, &tl));
}

#[test]
fn track_shortfall_is_measured_from_the_shorter_track() {
    let sr = 16_000usize;
    let tl = ChannelTimeline::from_tracks(&vec![0.1; sr * 3], &vec![0.0; sr * 2], sr).unwrap();
    assert!((tl.duration_secs - 2.0).abs() < 1e-9);
}

#[test]
fn single_side_row_stays_whole() {
    let tl = timeline(0.0, &[X, S, X, X]);
    assert!(plan_channel_split(0.0, 2.4, "all remote speech here", &tl, None).is_none());
    let tl = timeline(0.0, &[M, M, M]);
    assert!(plan_channel_split(0.0, 1.8, "all the owner here", &tl, None).is_none());
}

#[test]
fn edge_sliver_in_the_pad_is_folded_not_cut() {
    // A 300 ms remote window at the row's start (the previous speaker's last syllable
    // in the VAD pre-pad) must not take the owner's first word.
    let tl = ChannelTimeline::from_windows(vec![
        (0.0, 300.0, X),
        (300.0, 900.0, M),
        (900.0, 1500.0, M),
        (1500.0, 2100.0, M),
    ]);
    assert!(plan_channel_split(0.0, 2.1, "the owner speaks here", &tl, None).is_none());
}

#[test]
fn row_with_no_classified_windows_stays_whole() {
    let tl = timeline(100.0, &[M, X]);
    assert!(plan_channel_split(0.0, 3.0, "far from any window", &tl, None).is_none());
}

#[test]
fn from_tracks_classifies_windows_and_needs_both_tracks() {
    let sr = 16_000usize;
    let mut mic = vec![0.0f32; sr * 2];
    let mut sys = vec![0.0f32; sr * 2];
    mic[..sr].iter_mut().for_each(|x| *x = 0.1); // owner, first second
    sys[sr..].iter_mut().for_each(|x| *x = 0.1); // remote, second second
    let tl = ChannelTimeline::from_tracks(&mic, &sys, sr).expect("timeline");
    let tags: Vec<ChannelTag> = tl.windows.iter().map(|w| w.2).collect();
    assert_eq!(tags.first(), Some(&M));
    assert_eq!(tags.last(), Some(&S));
    assert!(ChannelTimeline::from_tracks(&mic, &[], sr).is_none());
    assert!(ChannelTimeline::from_tracks(&[], &sys, sr).is_none());
}
