//! specs/0078 W0 task 2 — room detection on REAL recordings (local, `#[ignore]`).
//!
//! Discovers samples STRUCTURALLY: every subdirectory of the eval dir (the same
//! resolution as `diarization_tuning.rs`: `NIXON_DIARIZATION_EVAL_DIR`, else a
//! `zoom-samples/` directory beside the repo) that holds both `mic.wav` and `system.wav`.
//! Folder names can be meeting titles and this repo is public, so none is hard-coded
//! here. For each sample it prints the channel activity and the verdict and, for a room
//! verdict, the speaker count a room pass predicts by clustering the mic with the pass's
//! own count logic (Auto → room ceiling → consolidation + the 0050 audio seed). No
//! reference transcript is needed. Nothing is cached: every run diarizes from the audio.
//!
//! Run with:
//!   NIXON_DIARIZATION_PROVIDER=cpu cargo test --features metal --test diarization_room_eval \
//!     -- --ignored --nocapture

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use app_lib::audio::retranscription_channels::ChannelRmsProfile;
use app_lib::diarization::models::{EMBEDDING_MODEL_FILE, SEGMENTATION_MODEL_FILE};
use app_lib::diarization::room::{detect_room, resolve_setup, room_speaker_ceiling};
use app_lib::diarization::room_types::{AudioSetup, AudioSetupOverride};
use app_lib::diarization::{Diarizer, SherpaDiarizer, SpeakerCount, UNKNOWN_SPEAKER_KEY};

/// `NIXON_DIARIZATION_EVAL_DIR`, else `zoom-samples/` next to the repo root or its parent.
fn eval_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("NIXON_DIARIZATION_EVAL_DIR") {
        let p = PathBuf::from(dir);
        return p.is_dir().then_some(p);
    }
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    [
        repo_root.join("..").join("zoom-samples"),
        repo_root.join("..").join("..").join("zoom-samples"),
    ]
    .into_iter()
    .filter_map(|p| p.canonicalize().ok())
    .find(|p| p.is_dir())
}

/// Every subdirectory holding both channel files, sorted for stable output.
fn discover_channel_samples(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .filter(|p| p.join("mic.wav").is_file() && p.join("system.wav").is_file())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
#[ignore = "local eval over real recordings; run with --ignored"]
fn room_detection_on_real_samples() {
    let Some(dir) = eval_dir() else {
        eprintln!("SKIP: no eval dir (set NIXON_DIARIZATION_EVAL_DIR)");
        return;
    };
    let samples = discover_channel_samples(&dir);
    if samples.is_empty() {
        eprintln!(
            "SKIP: no <sample>/{{mic.wav + system.wav}} under {}",
            dir.display()
        );
        return;
    }
    let models = common::diarization_models_dir();

    for sample in samples {
        let label = sample
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let t0 = Instant::now();
        let Some(profile) = ChannelRmsProfile::load_for_detection(&sample) else {
            eprintln!("[{label}] channels unreadable; a pass would run as a call");
            continue;
        };
        let activity = profile.activity();
        let setup = resolve_setup(AudioSetupOverride::Auto, Some(&activity));
        eprintln!(
            "[{label}] {activity} | detect_room={} → {} ({:.1}s)",
            detect_room(&activity),
            setup.as_str(),
            t0.elapsed().as_secs_f64()
        );
        if setup != AudioSetup::Room {
            continue;
        }
        let Some(models) = models.as_ref() else {
            eprintln!("[{label}] SKIP clustering: diarization models not found");
            continue;
        };

        let count = room_speaker_ceiling(SpeakerCount::Auto, setup);
        let diarizer = SherpaDiarizer::with_speaker_count(
            &models.join(SEGMENTATION_MODEL_FILE),
            &models.join(EMBEDDING_MODEL_FILE),
            count,
        )
        .expect("init diarizer");
        let decoded =
            app_lib::audio::decoder::decode_audio_file(&sample.join("mic.wav")).expect("decode");
        let mic = decoded.to_whisper_format();
        let t0 = Instant::now();
        let (turns, embeddings) = diarizer
            .diarize_with_embeddings(&mic, 16_000)
            .expect("diarize mic");
        let mut talk: BTreeMap<String, f32> = BTreeMap::new();
        for t in &turns {
            *talk.entry(t.speaker.clone()).or_default() += t.duration();
        }
        let predicted = talk.keys().filter(|k| *k != UNKNOWN_SPEAKER_KEY).count();
        eprintln!(
            "[{label}] room pass ({count:?}): predicted {predicted} speaker(s), {} embedded, \
             in {:.0}s; talk time {:?}",
            embeddings.len(),
            t0.elapsed().as_secs_f64(),
            talk.iter()
                .map(|(k, v)| format!("{k}={v:.0}s"))
                .collect::<Vec<_>>()
        );
    }
}
