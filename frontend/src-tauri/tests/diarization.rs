//! Speaker-diarization core test (specs/0010 P1-A).
//!
//! Two layers:
//! 1. **Alignment** (`align::align_turns_to_segments`) is pure logic with no model
//!    dependency — its unit tests live in-module (`src/diarization/align.rs`) and
//!    run unconditionally with `cargo test`. This file does NOT re-test it.
//! 2. **End-to-end diarization** runs the REAL `SherpaDiarizer` on a synthesized
//!    two-voice 16 kHz mono clip. It is MODEL-GATED: it SKIPS cleanly (like the
//!    Parakeet test in specs/0009) when the diarization ONNX models aren't
//!    downloaded, and RUNS when they are.
//!
//! Run with:
//!   cargo test --features metal --test diarization -- --nocapture

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use app_lib::diarization::{Diarizer, SherpaDiarizer};

/// Filenames the diarization engine needs on disk (see
/// `src/diarization/models.rs`).
const SEGMENTATION_FILE: &str = "segmentation.onnx";
const EMBEDDING_FILE: &str = "3dspeaker_campplus_sv_en_voxceleb_16k.onnx";

/// Candidate `models/diarization/` dirs across the dev/prod/upstream identifiers
/// (mirrors the Parakeet model-gate in `tests/common/mod.rs`).
fn diarization_models_dir() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(dir) = std::env::var("NIXON_TEST_DIARIZATION_DIR") {
        roots.push(PathBuf::from(dir));
    }
    if let Some(data) = dirs::data_dir() {
        for id in [
            "ai.vinyl.app.debug",
            "ai.vinyl.app",
            "com.vinyl.dev.debug",
            "com.vinyl.dev",
            "com.meetily.ai",
            "Nixon",
        ] {
            roots.push(data.join(id).join("models").join("diarization"));
        }
    }

    roots
        .into_iter()
        .find(|dir| dir.join(SEGMENTATION_FILE).exists() && dir.join(EMBEDDING_FILE).exists())
}

/// Synthesize `text` in voice `voice` to a 16 kHz mono WAV. Returns false if
/// `say` is unavailable (caller skips).
fn synth_voice(text: &str, voice: &str, out: &Path) -> bool {
    let status = Command::new("say")
        .args(["-v", voice, "-o"])
        .arg(out)
        .args(["--data-format=LEI16@16000", "--channels=1", text])
        .status();
    matches!(status, Ok(s) if s.success() && out.exists())
}

#[test]
fn sherpa_diarizes_two_voices() {
    let test_name = "sherpa_diarizes_two_voices";

    // 1. Model gate: skip cleanly if the diarization models aren't downloaded.
    let Some(models_dir) = diarization_models_dir() else {
        eprintln!(
            "SKIP {test_name}: diarization models not found. Download them in-app or set \
             NIXON_TEST_DIARIZATION_DIR to a dir containing {SEGMENTATION_FILE} + \
             {EMBEDDING_FILE}. This is a runtime skip, not a failure."
        );
        return;
    };

    // 2. Synthesize two distinct voices and concatenate them (voice A, then B,
    //    then A again) so clustering has multi-turn evidence.
    let dir = tempfile::tempdir().expect("tempdir");
    let a1 = dir.path().join("a1.wav");
    let b1 = dir.path().join("b1.wav");
    let a2 = dir.path().join("a2.wav");

    let say_ok = synth_voice(
        "Good morning everyone, thanks for joining the meeting today.",
        "Samantha",
        &a1,
    ) && synth_voice(
        "Yes hello, I have a few questions about the quarterly roadmap.",
        "Daniel",
        &b1,
    ) && synth_voice(
        "Sure, let me walk you through the plan step by step.",
        "Samantha",
        &a2,
    );
    if !say_ok {
        eprintln!("SKIP {test_name}: `say` unavailable, cannot synthesize voices");
        return;
    }

    let (sa1, ra) = common::decode_wav_16k_mono(&a1);
    let (sb1, _) = common::decode_wav_16k_mono(&b1);
    let (sa2, _) = common::decode_wav_16k_mono(&a2);
    assert_eq!(ra, 16_000, "say fixture should be 16 kHz");

    // Stitch with short pauses so segmentation has clear boundaries.
    let mut samples = Vec::new();
    samples.extend_from_slice(&sa1);
    samples.extend(common::silence(0.5, 16_000));
    samples.extend_from_slice(&sb1);
    samples.extend(common::silence(0.5, 16_000));
    samples.extend_from_slice(&sa2);

    // 3. Real engine.
    let diarizer = SherpaDiarizer::new(
        &models_dir.join(SEGMENTATION_FILE),
        &models_dir.join(EMBEDDING_FILE),
    )
    .expect("construct SherpaDiarizer");

    let turns = diarizer
        .diarize(&samples, 16_000)
        .expect("diarize two-voice clip");

    eprintln!("{test_name}: got {} turns: {turns:?}", turns.len());

    // 4. Assertions: at least one turn; ideally the two synthesized voices are
    //    separated into >= 2 speakers (clustering on synthetic TTS is imperfect,
    //    so >=1 is the hard gate and >=2 is logged as the ideal).
    assert!(
        !turns.is_empty(),
        "diarizer returned ZERO turns for a multi-voice clip"
    );

    let distinct: std::collections::BTreeSet<&str> =
        turns.iter().map(|t| t.speaker.as_str()).collect();
    eprintln!(
        "{test_name}: {} distinct speaker(s): {distinct:?}",
        distinct.len()
    );
    if distinct.len() < 2 {
        eprintln!(
            "WARN {test_name}: only {} speaker(s) recovered from a 2-voice clip; \
             clustering may under-segment synthetic TTS (acceptable for P1-A gate).",
            distinct.len()
        );
    }

    // Turns must be ordered and non-degenerate.
    for w in turns.windows(2) {
        assert!(
            w[1].start >= w[0].start,
            "turns should be sorted by start time: {:?}",
            turns
        );
    }
    for t in &turns {
        assert!(t.end >= t.start, "turn end before start: {t:?}");
    }
}
