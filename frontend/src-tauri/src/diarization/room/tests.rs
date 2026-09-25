//! Unit tests for `room.rs`: detection, setup resolution, the ceiling, and the owner rules.

use super::*;

/// The in-person sample's measured profile (see the module table).
fn in_person() -> ChannelActivity {
    ChannelActivity {
        duration_secs: 3_259.0,
        system_active_secs: 4.8,
        system_longest_run_secs: 1.2,
        mic_active_secs: 2_800.8,
        system_present: true,
    }
}

/// The two-channel call sample's measured profile.
fn call() -> ChannelActivity {
    ChannelActivity {
        duration_secs: 3_068.0,
        system_active_secs: 2_136.6,
        system_longest_run_secs: 28.2,
        mic_active_secs: 2_661.6,
        system_present: true,
    }
}

#[test]
fn the_in_person_profile_is_a_room() {
    assert!(detect_room(&in_person()));
}

#[test]
fn the_call_profile_is_a_call() {
    assert!(!detect_room(&call()));
}

#[test]
fn notification_dings_alone_are_still_a_room() {
    // 7 dings of up to 1.2 s each: 8.4 s total, never a run of 3 s.
    let a = ChannelActivity {
        system_active_secs: 7.0 * 1.2,
        system_longest_run_secs: 1.2,
        ..in_person()
    };
    assert!(detect_room(&a));
}

#[test]
fn one_sustained_system_run_makes_it_a_call() {
    let a = ChannelActivity {
        system_active_secs: 3.5,
        system_longest_run_secs: 3.5,
        ..in_person()
    };
    assert!(!detect_room(&a), "a 3.5 s system run is remote speech");
}

#[test]
fn many_short_blips_in_a_short_meeting_make_it_a_call() {
    // 10 minutes, 20 s of 1.2 s blips: under 30 s but over 2% of the duration.
    let a = ChannelActivity {
        duration_secs: 600.0,
        system_active_secs: 20.0,
        system_longest_run_secs: 1.2,
        mic_active_secs: 400.0,
        system_present: true,
    };
    assert!(!detect_room(&a));
}

#[test]
fn too_little_mic_speech_stays_a_call() {
    let a = ChannelActivity {
        mic_active_secs: 29.4,
        ..in_person()
    };
    assert!(
        !detect_room(&a),
        "under 30 s of mic speech is not worth diarizing"
    );
}

#[test]
fn a_missing_system_channel_with_mic_speech_is_a_room() {
    let a = ChannelActivity {
        system_present: false,
        // A missing track can't have activity, but even junk numbers must not matter.
        system_active_secs: 999.0,
        system_longest_run_secs: 999.0,
        ..in_person()
    };
    assert!(system_is_quiet(&a));
    assert!(detect_room(&a));
}

#[test]
fn every_override_combination_resolves() {
    use AudioSetupOverride as O;
    let room = in_person();
    let call = call();
    // Call override always wins.
    assert_eq!(resolve_setup(O::Call, Some(&room)), AudioSetup::Call);
    assert_eq!(resolve_setup(O::Call, Some(&call)), AudioSetup::Call);
    assert_eq!(resolve_setup(O::Call, None), AudioSetup::Call);
    // Room override forces room (hybrid is not built yet).
    assert_eq!(resolve_setup(O::Room, Some(&room)), AudioSetup::Room);
    assert_eq!(resolve_setup(O::Room, Some(&call)), AudioSetup::Room);
    assert_eq!(resolve_setup(O::Room, None), AudioSetup::Room);
    // Auto detects, never picks hybrid, and falls back to call without channels.
    assert_eq!(resolve_setup(O::Auto, Some(&room)), AudioSetup::Room);
    assert_eq!(resolve_setup(O::Auto, Some(&call)), AudioSetup::Call);
    assert_eq!(resolve_setup(O::Auto, None), AudioSetup::Call);

    assert_eq!(SetupSource::of(O::Auto), SetupSource::Detected);
    assert_eq!(SetupSource::of(O::Room), SetupSource::Override);
    assert_eq!(SetupSource::of(O::Call).as_str(), "override");
}

#[test]
fn the_ceiling_counts_the_owner_only_when_the_owner_is_clustered() {
    use SpeakerCount::*;
    assert_eq!(room_speaker_ceiling(AtMost(3), AudioSetup::Room), AtMost(4));
    assert_eq!(
        room_speaker_ceiling(AtMost(3), AudioSetup::Hybrid),
        AtMost(4)
    );
    assert_eq!(room_speaker_ceiling(AtMost(3), AudioSetup::Call), AtMost(3));
    assert_eq!(room_speaker_ceiling(Auto, AudioSetup::Room), Auto);
    assert_eq!(room_speaker_ceiling(Fixed(2), AudioSetup::Room), Fixed(2));
    assert_eq!(
        room_speaker_ceiling(AtMost(0), AudioSetup::Room),
        AtMost(0),
        "0 means no cap and stays no cap"
    );
}

fn write(path: std::path::PathBuf) {
    std::fs::write(path, b"RIFF").unwrap();
}

#[test]
fn choose_input_clusters_the_mic_in_a_room_and_the_system_track_in_a_call() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().to_path_buf();
    write(folder.join("mic.wav"));
    write(folder.join("system.wav"));

    let room = choose_input(folder.clone(), AudioSetupOverride::Auto, Some(in_person()));
    assert_eq!(room.setup, AudioSetup::Room);
    assert_eq!(room.cluster_wav, folder.join("mic.wav"));
    assert_eq!(room.source, SetupSource::Detected);

    let call_input = choose_input(folder.clone(), AudioSetupOverride::Auto, Some(call()));
    assert_eq!(call_input.setup, AudioSetup::Call);
    assert_eq!(call_input.cluster_wav, folder.join("system.wav"));
}

#[test]
fn choose_input_without_channels_is_a_call_exactly_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().to_path_buf();
    // Even a Room override can't cluster a mic that isn't there.
    let input = choose_input(folder.clone(), AudioSetupOverride::Room, None);
    assert_eq!(input.setup, AudioSetup::Call);
    assert_eq!(input.cluster_wav, system_channel_wav(&folder));
}

// ----- owner rules -----

fn turn(start: f32, end: f32, speaker: &str) -> SpeakerTurn {
    SpeakerTurn {
        start,
        end,
        speaker: speaker.to_string(),
    }
}

fn two_clusters() -> (Vec<SpeakerTurn>, HashMap<String, Vec<f32>>) {
    (
        vec![
            turn(0.0, 5.0, "spk_0"),
            turn(5.0, 9.0, "spk_1"),
            turn(9.0, 9.5, UNKNOWN_SPEAKER_KEY),
        ],
        HashMap::from([
            ("spk_0".to_string(), vec![1.0, 0.0, 0.0]),
            ("spk_1".to_string(), vec![0.0, 1.0, 0.0]),
        ]),
    )
}

#[test]
fn a_single_cluster_is_the_owner() {
    let turns = vec![turn(0.0, 5.0, "spk_0"), turn(6.0, 7.0, UNKNOWN_SPEAKER_KEY)];
    let label = choose_owner_cluster(&turns, &HashMap::new(), None, None).unwrap();
    assert_eq!(label.cluster, "spk_0");
    assert_eq!(label.rule, OwnerRule::SingleCluster);
}

#[test]
fn two_clusters_without_owner_evidence_stay_anonymous() {
    let (turns, emb) = two_clusters();
    assert_eq!(choose_owner_cluster(&turns, &emb, None, None), None);
}

#[test]
fn the_owner_voiceprint_picks_its_cluster() {
    let (turns, emb) = two_clusters();
    let label =
        choose_owner_cluster(&turns, &emb, Some(("spk_1".to_string(), 0.83)), None).unwrap();
    assert_eq!(label.cluster, "spk_1");
    assert_eq!(label.rule, OwnerRule::Voiceprint);
    // A voiceprint pick naming a cluster this pass didn't produce is ignored.
    assert_eq!(
        choose_owner_cluster(&turns, &emb, Some(("spk_7".to_string(), 0.9)), None),
        None
    );
}

#[test]
fn the_previous_local_embedding_carries_over_at_tau_match() {
    let (turns, emb) = two_clusters();
    // Close to spk_0 (cosine ≈ 0.95).
    let prior = [0.95f32, 0.3, 0.0];
    let label = choose_owner_cluster(&turns, &emb, None, Some(&prior)).unwrap();
    assert_eq!(label.cluster, "spk_0");
    assert_eq!(label.rule, OwnerRule::CarryOver);
    // Unlike every cluster (cosine 0 to both): nobody.
    let far = [0.0f32, 0.0, 1.0];
    assert_eq!(choose_owner_cluster(&turns, &emb, None, Some(&far)), None);
}

#[test]
fn rules_apply_in_order() {
    let (turns, emb) = two_clusters();
    // The voiceprint (rule 2) beats the carry-over (rule 3).
    let prior = [1.0f32, 0.0, 0.0];
    let label =
        choose_owner_cluster(&turns, &emb, Some(("spk_1".to_string(), 0.8)), Some(&prior)).unwrap();
    assert_eq!(label.rule, OwnerRule::Voiceprint);
    assert_eq!(label.cluster, "spk_1");
}

#[test]
fn renaming_moves_turns_and_embedding_to_local() {
    let (mut turns, mut emb) = two_clusters();
    rename_cluster_to_local(&mut turns, &mut emb, "spk_1");
    let keys: Vec<&str> = turns.iter().map(|t| t.speaker.as_str()).collect();
    assert_eq!(keys, vec!["spk_0", LOCAL_SPEAKER_KEY, UNKNOWN_SPEAKER_KEY]);
    assert!(emb.contains_key(LOCAL_SPEAKER_KEY));
    assert!(!emb.contains_key("spk_1"));
    assert_eq!(emb[LOCAL_SPEAKER_KEY], vec![0.0, 1.0, 0.0]);
}
