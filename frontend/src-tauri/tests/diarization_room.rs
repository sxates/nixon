//! specs/0078 task 12 — room recordings end to end on synthetic audio (model-gated).
//!
//! Builds the W0 synthetic meetings (`common::synth_room_meeting`: two `say` voices, a
//! silent system track with two notification dings) and runs them through the real
//! detection and the real diarizer, configured the way a room pass configures it:
//!
//! - room → detected as a room → clustering the mic gives exactly 2 speakers;
//! - solo → 1 cluster → labeled `local` ("You");
//! - call (voice B on the system track) → not a room.
//!
//! SKIPS cleanly without the diarization models or `say`. Run with:
//!   cargo test --features metal --test diarization_room -- --nocapture

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use app_lib::audio::retranscription_channels::ChannelRmsProfile;
use app_lib::diarization::models::{EMBEDDING_MODEL_FILE, SEGMENTATION_MODEL_FILE};
use app_lib::diarization::room::{
    detect_room, label_owner_cluster, resolve_setup, room_speaker_ceiling, ChannelActivity,
    OwnerRule,
};
use app_lib::diarization::room_types::{AudioSetup, AudioSetupOverride};
use app_lib::diarization::{
    Diarizer, SherpaDiarizer, SpeakerCount, TurnsWithEmbeddings, LOCAL_SPEAKER_KEY,
    UNKNOWN_SPEAKER_KEY,
};
use common::{RoomFixture, ROOM_DING_AT_SECS};

/// Build a fixture folder, or `None` (skip) without models or `say`.
fn fixture(name: &str, variant: RoomFixture) -> Option<(tempfile::TempDir, std::path::PathBuf)> {
    let Some(models) = common::diarization_models_dir() else {
        eprintln!("SKIP {name}: diarization models not found (runtime skip, not a failure)");
        return None;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    if common::synth_room_meeting(dir.path(), variant).is_none() {
        eprintln!("SKIP {name}: `say` or its voices are unavailable");
        return None;
    }
    Some((dir, models))
}

fn activity(folder: &Path) -> ChannelActivity {
    ChannelRmsProfile::load_for_detection(folder)
        .expect("fixture channels load")
        .activity()
}

/// Cluster the mic the way a room pass does: Auto count raised by the room ceiling, the
/// shipped models, consolidation and the 0050 audio seed on.
fn cluster_mic(folder: &Path, models: &Path) -> TurnsWithEmbeddings {
    let diarizer = SherpaDiarizer::with_speaker_count(
        &models.join(SEGMENTATION_MODEL_FILE),
        &models.join(EMBEDDING_MODEL_FILE),
        room_speaker_ceiling(SpeakerCount::Auto, AudioSetup::Room),
    )
    .expect("init diarizer");
    let (mic, rate) = common::decode_wav_16k_mono(&folder.join("mic.wav"));
    diarizer
        .diarize_with_embeddings(&mic, rate)
        .expect("diarize mic")
}

fn clusters(turns: &[app_lib::diarization::SpeakerTurn]) -> BTreeSet<String> {
    turns
        .iter()
        .map(|t| t.speaker.clone())
        .filter(|k| k != UNKNOWN_SPEAKER_KEY)
        .collect()
}

#[test]
fn a_two_voice_room_is_detected_and_clusters_into_two_speakers() {
    let name = "a_two_voice_room_is_detected_and_clusters_into_two_speakers";
    let Some((dir, models)) = fixture(name, RoomFixture::Room) else {
        return;
    };
    let a = activity(dir.path());
    eprintln!("{name}: {a}");
    assert!(
        a.system_active_secs > 0.0,
        "the dings are on the system track"
    );
    assert!(
        a.system_active_secs <= ROOM_DING_AT_SECS.len() as f32 * 0.6 + 1e-3,
        "each ding fills one 600 ms window: {a}"
    );
    assert!(detect_room(&a), "dings alone must not make it a call: {a}");
    assert_eq!(
        resolve_setup(AudioSetupOverride::Auto, Some(&a)),
        AudioSetup::Room
    );

    let (turns, _) = cluster_mic(dir.path(), &models);
    let found = clusters(&turns);
    eprintln!("{name}: clusters {found:?}");
    assert_eq!(
        found.len(),
        2,
        "two voices on one mic → two speakers: {found:?}"
    );
}

#[tokio::test]
async fn a_solo_recording_is_one_cluster_labeled_you() {
    let name = "a_solo_recording_is_one_cluster_labeled_you";
    let Some((dir, models)) = fixture(name, RoomFixture::Solo) else {
        return;
    };
    let a = activity(dir.path());
    eprintln!("{name}: {a}");
    assert!(detect_room(&a), "{a}");

    let (mut turns, mut embeddings) = cluster_mic(dir.path(), &models);
    assert_eq!(clusters(&turns).len(), 1, "{:?}", clusters(&turns));

    let (_db_dir, db) = common::fresh_db().await;
    let label = label_owner_cluster(db.pool(), "solo", &mut turns, &mut embeddings, &[])
        .await
        .expect("the single voice is the owner");
    assert_eq!(label.rule, OwnerRule::SingleCluster);
    assert_eq!(
        clusters(&turns),
        BTreeSet::from([LOCAL_SPEAKER_KEY.to_string()])
    );
}

#[test]
fn a_call_is_not_a_room() {
    let name = "a_call_is_not_a_room";
    let Some((dir, _models)) = fixture(name, RoomFixture::Call) else {
        return;
    };
    let a = activity(dir.path());
    eprintln!("{name}: {a}");
    assert!(
        !detect_room(&a),
        "remote speech on the system track is a call: {a}"
    );
    assert_eq!(
        resolve_setup(AudioSetupOverride::Auto, Some(&a)),
        AudioSetup::Call
    );
}
