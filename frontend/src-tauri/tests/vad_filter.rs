//! VAD filtering tests (specs/0009, priority #2).
//!
//! Guards the Silero VAD stage (`audio/vad.rs`) against two opposite failures:
//!   - silence leaking through (would feed Whisper/Parakeet garbage), and
//!   - the specs/0004 OVER-GATING regression, where real speech is dropped and
//!     "transcribing produces nothing".
//!
//! These tests use NO model and NO microphone. The speech case uses a `say`
//! fixture (skipped if `say` is unavailable); the silence case is pure synthetic
//! zeros and always runs.
//!
//! Run with:
//!   cargo test --features metal --test vad_filter -- --nocapture

mod common;

use app_lib::audio::vad::{get_speech_chunks, ContinuousVadProcessor};

/// The pre-fix "strict" Silero config that shipped with meetily (specs/0004
/// over-gating repro). Kept here so the comparison test documents exactly what
/// dropped speech and proves the new tuned defaults are strictly better.
const OLD_POSITIVE: f32 = 0.50;
const OLD_NEGATIVE: f32 = 0.35;
const OLD_MIN_SPEECH_MS: u64 = 250;

/// Live-pipeline redemption (hangover) used by `audio/pipeline.rs`.
const LIVE_REDEMPTION_MS: u32 = 400;

/// Run a fixture through a VAD configured with explicit thresholds, streaming it
/// in small (512-sample) chunks the way the live pipeline does. Returns the
/// fraction of input samples retained as speech (0.0 == fully dropped).
fn retained_ratio(samples: &[f32], positive: f32, negative: f32, min_speech_ms: u64) -> f32 {
    let mut processor = ContinuousVadProcessor::new_with_thresholds(
        16_000,
        LIVE_REDEMPTION_MS,
        positive,
        negative,
        min_speech_ms,
    )
    .expect("vad processor");
    let mut all = Vec::new();
    for chunk in samples.chunks(512) {
        all.extend(processor.process_audio(chunk).expect("process chunk"));
    }
    all.extend(processor.flush().expect("flush"));
    let detected: usize = all.iter().map(|s| s.samples.len()).sum();
    detected as f32 / samples.len().max(1) as f32
}

/// Pure silence must yield no speech segments (no false positives).
#[test]
fn silence_yields_no_speech_segments() {
    let silence = common::silence(3.0, 16_000); // 3s of zeros at 16 kHz
    let segments = get_speech_chunks(&silence, 400).expect("vad on silence");
    assert!(
        segments.is_empty(),
        "VAD reported {} speech segment(s) for pure silence",
        segments.len()
    );
}

/// Real synthesized speech must be detected (guards specs/0004 over-gating).
#[test]
fn speech_fixture_is_detected() {
    let Some(samples) = common::speech_samples_16k() else {
        eprintln!("SKIP speech_fixture_is_detected: `say` unavailable");
        return;
    };

    let segments = get_speech_chunks(&samples, 400).expect("vad on speech");
    assert!(
        !segments.is_empty(),
        "VAD detected NO speech in a real spoken sentence (specs/0004 over-gating regression)"
    );

    // The detected speech should account for a meaningful fraction of the clip,
    // not a single 40ms fragment. Sum segment samples and compare to input.
    let detected: usize = segments.iter().map(|s| s.samples.len()).sum();
    let ratio = detected as f32 / samples.len() as f32;
    eprintln!(
        "VAD found {} segment(s), {} / {} samples retained ({:.0}%)",
        segments.len(),
        detected,
        samples.len(),
        ratio * 100.0
    );
    assert!(
        ratio > 0.20,
        "VAD retained only {:.0}% of a spoken sentence — likely over-gating",
        ratio * 100.0
    );
}

/// Streaming the same speech through the processor in small (30ms-ish) chunks —
/// the way the live pipeline feeds it — must still detect speech. This guards
/// against state being lost across chunk boundaries during real-time capture.
#[test]
fn speech_detected_when_streamed_in_small_chunks() {
    let Some(samples) = common::speech_samples_16k() else {
        eprintln!("SKIP speech_detected_when_streamed_in_small_chunks: `say` unavailable");
        return;
    };

    let mut processor = ContinuousVadProcessor::new(16_000, 400).expect("vad processor");
    let mut all = Vec::new();
    for chunk in samples.chunks(512) {
        all.extend(processor.process_audio(chunk).expect("process_audio chunk"));
    }
    all.extend(processor.flush().expect("flush"));

    assert!(
        !all.is_empty(),
        "VAD detected no speech when fed real speech in small streaming chunks"
    );
}

// ---------------------------------------------------------------------------
// specs/0004 over-gating fixtures — quiet, short, and paused speech.
// These mimic the live "whole phrases missing" symptom. Each asserts that real
// speech is NOT dropped under the (now tuned) default config.
// ---------------------------------------------------------------------------

/// QUIET speech (soft talker / low mic gain). Scale the fixture down to ~-35
/// dBFS peak and require the VAD still keeps a meaningful fraction. The old
/// strict config (0.50/0.35/250ms) dropped this entirely — see the comparison
/// test below.
#[test]
fn quiet_speech_is_not_dropped() {
    let Some(samples) = common::speech_samples_16k() else {
        eprintln!("SKIP quiet_speech_is_not_dropped: `say` unavailable");
        return;
    };
    let quiet = common::scale_to_peak_dbfs(&samples, -35.0);
    eprintln!(
        "quiet fixture: peak={:.1}dBFS rms={:.1}dBFS",
        common::peak_dbfs(&quiet),
        common::rms_dbfs(&quiet)
    );

    let ratio = retained_ratio(
        &quiet,
        app_lib::audio::vad::POSITIVE_SPEECH_THRESHOLD,
        app_lib::audio::vad::NEGATIVE_SPEECH_THRESHOLD,
        app_lib::audio::vad::MIN_SPEECH_MS,
    );
    eprintln!(
        "quiet speech retained {:.0}% under tuned config",
        ratio * 100.0
    );
    assert!(
        ratio > 0.20,
        "quiet speech retained only {:.0}% — VAD over-gating soft speech",
        ratio * 100.0
    );
}

/// SHORT utterance (1-2 words, ~300-500ms). The strict 250ms min_speech_time
/// could discard these before `SpeechStart` ever fired. The tuned config must
/// keep them.
#[test]
fn short_utterance_is_not_dropped() {
    // A one-word reply (~400ms). Under the OLD strict config (250ms min_speech)
    // words like this were dropped to 0% — see `probe_old_config_drops`.
    let Some(samples) = common::speech_samples_16k_text("okay") else {
        eprintln!("SKIP short_utterance_is_not_dropped: `say` unavailable");
        return;
    };
    eprintln!(
        "short fixture: {} samples ({:.0}ms), peak={:.1}dBFS",
        samples.len(),
        samples.len() as f32 / 16_000.0 * 1000.0,
        common::peak_dbfs(&samples)
    );

    let ratio = retained_ratio(
        &samples,
        app_lib::audio::vad::POSITIVE_SPEECH_THRESHOLD,
        app_lib::audio::vad::NEGATIVE_SPEECH_THRESHOLD,
        app_lib::audio::vad::MIN_SPEECH_MS,
    );
    eprintln!(
        "short utterance retained {:.0}% under tuned config",
        ratio * 100.0
    );
    assert!(
        ratio > 0.20,
        "short utterance retained only {:.0}% — VAD dropped a brief phrase",
        ratio * 100.0
    );
}

/// PAUSED speech: two phrases with a ~700ms gap, both at slightly reduced level.
/// Redemption must bridge the pause (or at least surface both phrases) so neither
/// half of the sentence is lost.
#[test]
fn paused_speech_keeps_both_phrases() {
    let Some(a) = common::speech_samples_16k_text("the quarterly numbers") else {
        eprintln!("SKIP paused_speech_keeps_both_phrases: `say` unavailable");
        return;
    };
    let Some(b) = common::speech_samples_16k_text("look very strong this year") else {
        eprintln!("SKIP paused_speech_keeps_both_phrases: `say` unavailable");
        return;
    };
    // Slightly reduce level too, so this stresses recall (quiet + paused).
    let a = common::scale_to_peak_dbfs(&a, -28.0);
    let b = common::scale_to_peak_dbfs(&b, -28.0);
    let joined = common::join_with_pause(&a, &b, 700, 16_000);
    let voiced = a.len() + b.len();

    let mut processor =
        ContinuousVadProcessor::new(16_000, LIVE_REDEMPTION_MS).expect("vad processor");
    let mut all = Vec::new();
    for chunk in joined.chunks(512) {
        all.extend(processor.process_audio(chunk).expect("process chunk"));
    }
    all.extend(processor.flush().expect("flush"));

    let detected: usize = all.iter().map(|s| s.samples.len()).sum();
    let ratio = detected as f32 / voiced as f32;
    eprintln!(
        "paused speech: {} segment(s), {:.0}% of voiced audio retained",
        all.len(),
        ratio * 100.0
    );
    assert!(
        !all.is_empty(),
        "paused speech produced NO segments — both phrases lost"
    );
    // Should recover a good chunk of BOTH phrases, not just the first.
    assert!(
        ratio > 0.30,
        "paused speech retained only {:.0}% of voiced audio — a phrase was dropped",
        ratio * 100.0
    );
}

#[test]
#[ignore]
fn probe_old_config_drops() {
    let words = ["yes", "okay", "no", "right", "got it", "sure thing"];
    for w in words {
        let Some(s) = common::speech_samples_16k_text(w) else {
            return;
        };
        let old = retained_ratio(&s, OLD_POSITIVE, OLD_NEGATIVE, OLD_MIN_SPEECH_MS);
        let new = retained_ratio(
            &s,
            app_lib::audio::vad::POSITIVE_SPEECH_THRESHOLD,
            app_lib::audio::vad::NEGATIVE_SPEECH_THRESHOLD,
            app_lib::audio::vad::MIN_SPEECH_MS,
        );
        eprintln!(
            "[{w}] {} samples ({:.0}ms) OLD={:.0}% NEW={:.0}%",
            s.len(),
            s.len() as f32 / 16.0,
            old * 100.0,
            new * 100.0
        );
    }
    // Quiet + light white noise (mimics soft talker in a noisy room).
    let Some(s) = common::speech_samples_16k() else {
        return;
    };
    for db in [-30.0f32, -38.0, -45.0] {
        let q = common::scale_to_peak_dbfs(&s, db);
        // add -55 dBFS noise
        let noise_amp = 10f32.powf(-55.0 / 20.0);
        let mut idx = 0u32;
        let noisy: Vec<f32> = q
            .iter()
            .map(|x| {
                idx = idx.wrapping_mul(1664525).wrapping_add(1013904223);
                let n = (idx as f32 / u32::MAX as f32 - 0.5) * 2.0 * noise_amp;
                x + n
            })
            .collect();
        let old = retained_ratio(&noisy, OLD_POSITIVE, OLD_NEGATIVE, OLD_MIN_SPEECH_MS);
        let new = retained_ratio(
            &noisy,
            app_lib::audio::vad::POSITIVE_SPEECH_THRESHOLD,
            app_lib::audio::vad::NEGATIVE_SPEECH_THRESHOLD,
            app_lib::audio::vad::MIN_SPEECH_MS,
        );
        eprintln!(
            "[noisy quiet {db}dBFS] OLD={:.0}% NEW={:.0}%",
            old * 100.0,
            new * 100.0
        );
    }
}

/// Documents the regression and proves the fix: the OLD strict config dropped
/// (or badly under-retained) the quiet/short fixtures, while the tuned config
/// keeps them. Printed numbers are the specs/0004 repro evidence.
#[test]
fn tuned_config_beats_old_strict_config_on_adversarial_fixtures() {
    let Some(sentence) = common::speech_samples_16k() else {
        eprintln!("SKIP tuned_config_beats_old_strict_config: `say` unavailable");
        return;
    };
    let Some(short) = common::speech_samples_16k_text("okay") else {
        eprintln!("SKIP tuned_config_beats_old_strict_config: `say` unavailable");
        return;
    };
    let quiet = common::scale_to_peak_dbfs(&sentence, -35.0);

    let cases: [(&str, &[f32]); 3] = [
        ("normal sentence", &sentence),
        ("quiet (-35 dBFS) sentence", &quiet),
        ("short one-word utterance", &short),
    ];

    // Track that AT LEAST ONE adversarial case was fully dropped by the old
    // config but recovered by the tuned one — that is the specs/0004 repro.
    let mut recovered_a_dropped_case = false;

    for (name, samples) in cases {
        let old = retained_ratio(samples, OLD_POSITIVE, OLD_NEGATIVE, OLD_MIN_SPEECH_MS);
        let new = retained_ratio(
            samples,
            app_lib::audio::vad::POSITIVE_SPEECH_THRESHOLD,
            app_lib::audio::vad::NEGATIVE_SPEECH_THRESHOLD,
            app_lib::audio::vad::MIN_SPEECH_MS,
        );
        eprintln!(
            "[{name}] OLD strict retained {:.0}%, tuned retained {:.0}%",
            old * 100.0,
            new * 100.0
        );
        // The tuned config must never retain LESS than the old strict config.
        assert!(
            new + 0.001 >= old,
            "tuned config regressed on '{name}': old {:.0}% > tuned {:.0}%",
            old * 100.0,
            new * 100.0
        );
        if old <= 0.0001 && new > 0.20 {
            recovered_a_dropped_case = true;
        }
    }

    assert!(
        recovered_a_dropped_case,
        "expected at least one fixture the OLD config dropped entirely to be \
         recovered by the tuned config (specs/0004 repro)"
    );
}
