//! Diarization attribution + persistence test (specs/0010 P1-B2).
//!
//! Covers the DB side of the diarization pipeline with NO models and NO audio:
//! given synthetic system-channel speaker turns and transcript segments, the
//! correct speaker KEYS land in `transcripts.speaker` and the `speakers` rows are
//! upserted with the right display names. This exercises exactly the repository +
//! alignment calls `pipeline::run` makes after `diarize()` returns, so it validates
//! the persistence contract P1-C consumes without needing the ONNX runtime.
//!
//! Run with:
//!   cargo test --features metal --test diarization_persistence -- --nocapture

mod common;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::speaker::SpeakersRepository;
use app_lib::database::repositories::transcript::TranscriptsRepository;
use app_lib::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository;
use app_lib::diarization::align::align_system_turns_to_segments;
use app_lib::diarization::pipeline::display_name_for_key;
use app_lib::diarization::SpeakerTurn;
use common::{fresh_db, segment};

use sqlx::Acquire;

fn turn(start: f32, end: f32, speaker: &str) -> SpeakerTurn {
    SpeakerTurn {
        start,
        end,
        speaker: speaker.to_string(),
    }
}

/// Replicates the persistence the pipeline performs after alignment: clear prior
/// keys, set per-segment keys, upsert speakers. (The pipeline's own `persist` is a
/// private fn requiring an AppHandle; this mirrors it through the public repo API.)
async fn persist(pool: &sqlx::SqlitePool, meeting_id: &str, assignments: &[(String, String)]) {
    use std::collections::BTreeSet;

    let mut conn = pool.acquire().await.unwrap();

    // Mirror specs/0019 WS2.3: union override target keys so corrected lines resolve to
    // a name, and re-apply the overrides after re-stamping so they survive the re-run.
    let override_keys =
        TranscriptSpeakerOverridesRepository::override_keys_for_meeting(&mut conn, meeting_id)
            .await
            .unwrap();
    let mut keys: BTreeSet<String> = assignments.iter().map(|(_, k)| k.clone()).collect();
    keys.extend(override_keys);

    let mut tx = conn.begin().await.unwrap();
    SpeakersRepository::clear_meeting_speakers(&mut tx, meeting_id)
        .await
        .unwrap();
    SpeakersRepository::set_segment_speakers(&mut tx, assignments)
        .await
        .unwrap();
    TranscriptSpeakerOverridesRepository::reapply(&mut tx, meeting_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    for key in &keys {
        SpeakersRepository::upsert(
            pool,
            meeting_id,
            key,
            &display_name_for_key(key),
            key.as_str() == "local",
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }
}

/// The stored `transcripts.speaker` key for one segment.
async fn speaker_of(pool: &sqlx::SqlitePool, transcript_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT speaker FROM transcripts WHERE id = ?")
        .bind(transcript_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Fetch (id, audio window) for a meeting, start-ordered — matches the pipeline.
async fn load_windows(pool: &sqlx::SqlitePool, meeting_id: &str) -> Vec<(String, f32, f32)> {
    sqlx::query_as::<_, (String, Option<f64>, Option<f64>)>(
        "SELECT id, audio_start_time, audio_end_time FROM transcripts
         WHERE meeting_id = ? ORDER BY audio_start_time IS NULL, audio_start_time, timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(id, s, e)| (id, s.unwrap() as f32, e.unwrap() as f32))
    .collect()
}

#[tokio::test]
async fn attribution_persists_keys_and_speakers() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // A meeting with 4 segments. Two remote speakers (spk_0, spk_1) cover parts of
    // the timeline; the 3rd segment falls in a gap with no remote speech and more
    // than 1.0s from every turn, so the live alignment rule (specs/0043 W1.3)
    // attributes it to the "unknown" bucket — never "You" without a mic channel tag.
    let meeting_id =
        MeetingsRepository::create_meeting(pool, Some("Diar".into()), None, None, None, None)
            .await
            .unwrap();

    let segs = vec![
        segment("remote opening", 0.0, 3.0),     // -> spk_0
        segment("remote question", 6.0, 9.0),    // -> spk_1
        segment("my reply", 12.0, 14.0),         // gap, >1.0s from turns -> unknown
        segment("remote follow up", 18.0, 20.0), // -> spk_0
    ];
    let saved =
        TranscriptsRepository::save_transcripts_for_meeting(pool, &meeting_id, "Diar", &segs, None)
            .await
            .unwrap();
    assert!(saved, "segments attached to existing meeting");

    // System-channel turns (the remote speakers only).
    let turns = vec![
        turn(0.0, 3.5, "spk_0"),
        turn(5.5, 9.5, "spk_1"),
        turn(17.5, 20.5, "spk_0"),
    ];

    // Align exactly as the pipeline does, then persist.
    let windows = load_windows(pool, &meeting_id).await;
    let win_pairs: Vec<(f32, f32)> = windows.iter().map(|(_, s, e)| (*s, *e)).collect();
    let keys = align_system_turns_to_segments(&turns, &win_pairs);
    assert_eq!(keys, vec!["spk_0", "spk_1", "unknown", "spk_0"]);

    let assignments: Vec<(String, String)> =
        windows.into_iter().map(|(id, _, _)| id).zip(keys).collect();
    persist(pool, &meeting_id, &assignments).await;

    // 1. transcripts.speaker is written per segment (start-ordered).
    let stored: Vec<Option<String>> = sqlx::query_as::<_, (Option<String>,)>(
        "SELECT speaker FROM transcripts WHERE meeting_id = ?
         ORDER BY audio_start_time IS NULL, audio_start_time, timestamp",
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(s,)| s)
    .collect();
    assert_eq!(
        stored,
        vec![
            Some("spk_0".to_string()),
            Some("spk_1".to_string()),
            Some("unknown".to_string()),
            Some("spk_0".to_string()),
        ]
    );

    // 2. speakers rows upserted with the right display names + is_local flag.
    let speakers = SpeakersRepository::get_by_meeting(pool, &meeting_id)
        .await
        .unwrap();
    assert_eq!(speakers.len(), 3, "unknown + spk_0 + spk_1");
    let unknown = speakers
        .iter()
        .find(|s| s.speaker_key == "unknown")
        .unwrap();
    assert_eq!(unknown.display_name, "Unknown speaker");
    assert_eq!(unknown.is_local, 0);
    let s0 = speakers.iter().find(|s| s.speaker_key == "spk_0").unwrap();
    assert_eq!(s0.display_name, "Speaker 1");
    assert_eq!(s0.is_local, 0);
    let s1 = speakers.iter().find(|s| s.speaker_key == "spk_1").unwrap();
    assert_eq!(s1.display_name, "Speaker 2");

    // 3. The transcript read JOINs the display name back through.
    let (paginated, total) =
        MeetingsRepository::get_meeting_transcripts_paginated(pool, &meeting_id, 100, 0)
            .await
            .unwrap();
    assert_eq!(total, 4);
    assert_eq!(paginated[0].speaker.as_deref(), Some("spk_0"));
    assert_eq!(paginated[0].speaker_name.as_deref(), Some("Speaker 1"));
    assert_eq!(paginated[2].speaker.as_deref(), Some("unknown"));
    assert_eq!(
        paginated[2].speaker_name.as_deref(),
        Some("Unknown speaker")
    );
}

#[tokio::test]
async fn rerun_is_idempotent_and_resets() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id =
        MeetingsRepository::create_meeting(pool, Some("Re".into()), None, None, None, None)
            .await
            .unwrap();
    let segs = vec![segment("a", 0.0, 2.0), segment("b", 5.0, 7.0)];
    TranscriptsRepository::save_transcripts_for_meeting(pool, &meeting_id, "Re", &segs, None)
        .await
        .unwrap();

    let windows = load_windows(pool, &meeting_id).await;
    let ids: Vec<String> = windows.iter().map(|(id, _, _)| id.clone()).collect();

    // First run: both segments map to spk_0.
    let first: Vec<(String, String)> = ids
        .iter()
        .map(|id| (id.clone(), "spk_0".to_string()))
        .collect();
    persist(pool, &meeting_id, &first).await;
    assert_eq!(
        SpeakersRepository::get_by_meeting(pool, &meeting_id)
            .await
            .unwrap()
            .len(),
        1
    );

    // Re-run with a different clustering: spk_0 + local. The prior spk_0-only row
    // must be cleared, not accumulated.
    let second: Vec<(String, String)> = vec![
        (ids[0].clone(), "spk_0".to_string()),
        (ids[1].clone(), "local".to_string()),
    ];
    persist(pool, &meeting_id, &second).await;

    let speakers = SpeakersRepository::get_by_meeting(pool, &meeting_id)
        .await
        .unwrap();
    assert_eq!(speakers.len(), 2, "re-run resets, no stale rows");
    assert!(speakers.iter().any(|s| s.speaker_key == "local"));

    // Delete the meeting -> speakers rows are cleared too (no orphans).
    MeetingsRepository::delete_meeting(pool, &meeting_id)
        .await
        .unwrap();
    assert!(SpeakersRepository::get_by_meeting(pool, &meeting_id)
        .await
        .unwrap()
        .is_empty());
}

/// specs/0019 WS2.3 — a manual per-segment speaker correction must SURVIVE a later
/// offline re-diarization (which clears + rebuilds all keys). The override is keyed by
/// the stable transcript id and re-applied at the end of `persist`.
#[tokio::test]
async fn segment_speaker_override_survives_rerun() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id =
        MeetingsRepository::create_meeting(pool, Some("Split".into()), None, None, None, None)
            .await
            .unwrap();
    let segs = vec![segment("line one", 0.0, 2.0), segment("line two", 5.0, 7.0)];
    TranscriptsRepository::save_transcripts_for_meeting(pool, &meeting_id, "Split", &segs, None)
        .await
        .unwrap();

    let ids: Vec<String> = load_windows(pool, &meeting_id)
        .await
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    let both_spk0: Vec<(String, String)> = ids
        .iter()
        .map(|id| (id.clone(), "spk_0".to_string()))
        .collect();

    // First diarization lumps both lines under spk_0.
    persist(pool, &meeting_id, &both_spk0).await;
    assert_eq!(speaker_of(pool, &ids[0]).await.as_deref(), Some("spk_0"));
    assert_eq!(speaker_of(pool, &ids[1]).await.as_deref(), Some("spk_0"));

    // User corrects the SECOND line to spk_1 (the per-segment split).
    let applied = TranscriptSpeakerOverridesRepository::set(pool, &meeting_id, &ids[1], "spk_1")
        .await
        .unwrap();
    assert!(
        applied,
        "override applies to a line that belongs to the meeting"
    );
    assert_eq!(speaker_of(pool, &ids[1]).await.as_deref(), Some("spk_1"));

    // Re-diarize: this run AGAIN lumps both under spk_0 — but the correction must hold.
    persist(pool, &meeting_id, &both_spk0).await;
    assert_eq!(
        speaker_of(pool, &ids[0]).await.as_deref(),
        Some("spk_0"),
        "uncorrected line follows the new pass"
    );
    assert_eq!(
        speaker_of(pool, &ids[1]).await.as_deref(),
        Some("spk_1"),
        "corrected line survived re-diarization"
    );

    // spk_1 was materialized (via the override-key union) so its name resolves.
    let speakers = SpeakersRepository::get_by_meeting(pool, &meeting_id)
        .await
        .unwrap();
    assert!(
        speakers.iter().any(|s| s.speaker_key == "spk_1"),
        "override target speaker row exists"
    );

    // Setting an override for a line in another meeting must NOT touch this one.
    let other =
        MeetingsRepository::create_meeting(pool, Some("Other".into()), None, None, None, None)
            .await
            .unwrap();
    let bogus = TranscriptSpeakerOverridesRepository::set(pool, &other, &ids[1], "spk_9")
        .await
        .unwrap();
    assert!(!bogus, "a line from a different meeting is rejected");
    assert_eq!(
        speaker_of(pool, &ids[1]).await.as_deref(),
        Some("spk_1"),
        "still spk_1"
    );

    // Clearing the override lets the next pass revert the line.
    assert!(
        TranscriptSpeakerOverridesRepository::clear(pool, &meeting_id, &ids[1])
            .await
            .unwrap()
    );
    persist(pool, &meeting_id, &both_spk0).await;
    assert_eq!(
        speaker_of(pool, &ids[1]).await.as_deref(),
        Some("spk_0"),
        "reverts after the override is cleared"
    );
}

/// specs/0039 WS2 — a SPAN reassigned to a brand-new (manually-minted, embedding-less)
/// speaker must survive a later offline re-diarization. Exercises the exact backend
/// contract task 4 delivers: `SpeakersRepository::create_manual` +
/// `TranscriptSpeakerOverridesRepository::set_many` + `reapply`/`override_keys_for_meeting`.
///
/// Note: this asserts the SPAN keys survive (the override guarantee). The manual
/// speaker's *display name* carry-over across a re-run is the pipeline's
/// `restore_user_identities` (WS3.2) job — a private fn requiring an AppHandle, so the
/// `persist` helper here doesn't mirror it; name restoration is covered by the real
/// pipeline path.
#[tokio::test]
async fn span_reassign_to_new_speaker_survives_rerun() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id =
        MeetingsRepository::create_meeting(pool, Some("Span".into()), None, None, None, None)
            .await
            .unwrap();
    // Four lines; the first stretch (lines 0,1) is mis-attributed to spk_0 by clustering.
    let segs = vec![
        segment("first stretch a", 0.0, 2.0),
        segment("first stretch b", 3.0, 5.0),
        segment("later c", 8.0, 10.0),
        segment("later d", 12.0, 14.0),
    ];
    TranscriptsRepository::save_transcripts_for_meeting(pool, &meeting_id, "Span", &segs, None)
        .await
        .unwrap();

    let ids: Vec<String> = load_windows(pool, &meeting_id)
        .await
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();

    // First diarization lumps everything under spk_0.
    let all_spk0: Vec<(String, String)> = ids
        .iter()
        .map(|id| (id.clone(), "spk_0".to_string()))
        .collect();
    persist(pool, &meeting_id, &all_spk0).await;

    // User mints a NEW speaker the clusterer never produced, with a NULL embedding.
    let manual_key = SpeakersRepository::create_manual(pool, &meeting_id, "Priya")
        .await
        .unwrap();
    assert!(manual_key.starts_with("manual_"), "manual key shape");

    // Reassign the first-stretch SPAN (lines 0,1) to the new speaker — plus a bogus id
    // from no meeting, which must be skipped (not counted, no override written).
    let span = vec![ids[0].clone(), ids[1].clone(), "not-a-real-id".to_string()];
    let applied =
        TranscriptSpeakerOverridesRepository::set_many(pool, &meeting_id, &span, &manual_key)
            .await
            .unwrap();
    assert_eq!(
        applied, 2,
        "only the 2 real lines are reassigned; the bogus id is skipped"
    );
    assert_eq!(
        speaker_of(pool, &ids[0]).await.as_deref(),
        Some(manual_key.as_str())
    );
    assert_eq!(
        speaker_of(pool, &ids[1]).await.as_deref(),
        Some(manual_key.as_str())
    );
    assert_eq!(speaker_of(pool, &ids[2]).await.as_deref(), Some("spk_0"));

    // The manual key is materialized as a speaker row (via the override-key union).
    let speakers = SpeakersRepository::get_by_meeting(pool, &meeting_id)
        .await
        .unwrap();
    assert!(
        speakers.iter().any(|s| s.speaker_key == manual_key),
        "new-speaker row exists after set_many"
    );

    // Re-diarize: the clusterer AGAIN lumps everything under spk_0 and can NEVER
    // re-derive the embedding-less manual key — but the span override must hold.
    persist(pool, &meeting_id, &all_spk0).await;
    assert_eq!(
        speaker_of(pool, &ids[0]).await.as_deref(),
        Some(manual_key.as_str()),
        "span line 0 survives re-diarization on the new speaker"
    );
    assert_eq!(
        speaker_of(pool, &ids[1]).await.as_deref(),
        Some(manual_key.as_str()),
        "span line 1 survives re-diarization on the new speaker"
    );
    assert_eq!(
        speaker_of(pool, &ids[2]).await.as_deref(),
        Some("spk_0"),
        "uncorrected line follows the fresh clustering"
    );
    // The manual speaker row is re-materialized from the override-key union.
    assert!(
        SpeakersRepository::get_by_meeting(pool, &meeting_id)
            .await
            .unwrap()
            .iter()
            .any(|s| s.speaker_key == manual_key),
        "new-speaker row re-materialized after the re-run"
    );

    // Clearing the span's overrides reverts it to the clusterer (the manual speaker,
    // having no embedding, then has nothing pointing at it).
    for id in [&ids[0], &ids[1]] {
        TranscriptSpeakerOverridesRepository::clear(pool, &meeting_id, id)
            .await
            .unwrap();
    }
    persist(pool, &meeting_id, &all_spk0).await;
    assert_eq!(
        speaker_of(pool, &ids[0]).await.as_deref(),
        Some("spk_0"),
        "span reverts to the clusterer once its overrides are cleared"
    );
    assert!(
        !SpeakersRepository::get_by_meeting(pool, &meeting_id)
            .await
            .unwrap()
            .iter()
            .any(|s| s.speaker_key == manual_key),
        "manual speaker drops once no override references it"
    );
}

// ---------------------------------------------------------------------------
// specs/0078: room recordings (everyone on the mic), model-free routing
// ---------------------------------------------------------------------------

use app_lib::database::repositories::meeting_audio_setup::MeetingAudioSetupRepository;
use app_lib::database::repositories::voiceprints::VoiceprintsRepository;
use app_lib::diarization::embedding::EMBEDDING_MODEL_ID;
use app_lib::diarization::pipeline::attribute_and_persist;
use app_lib::diarization::room::{label_owner_cluster, OwnerRule};
use app_lib::diarization::room_types::AudioSetup;
use app_lib::people::enroll::{ensure_owner_person, OWNER_PERSON_ID};
use std::collections::HashMap;

/// A mic-tagged row, as capture writes every row of a room recording.
fn mic_segment(text: &str, start: f64, end: f64) -> app_lib::transcripts::TranscriptSegment {
    app_lib::transcripts::TranscriptSegment {
        channel: Some("microphone".to_string()),
        ..segment(text, start, end)
    }
}

/// A room meeting: two voices taking turns on one mic, every row tagged `microphone`,
/// one row gluing a fast handoff between them.
async fn seed_room_meeting(pool: &sqlx::SqlitePool) -> String {
    let meeting_id =
        MeetingsRepository::create_meeting(pool, Some("Room".into()), None, None, None, None)
            .await
            .unwrap();
    let segs = vec![
        mic_segment("first voice opens the meeting", 0.0, 3.0),
        mic_segment("second voice answers the question", 6.0, 9.0),
        mic_segment(
            "so that works for me. Great lets ship it tomorrow then",
            12.0,
            18.7,
        ),
        mic_segment("first voice wraps things up", 22.0, 24.0),
    ];
    TranscriptsRepository::save_transcripts_for_meeting(pool, &meeting_id, "Room", &segs, None)
        .await
        .unwrap();
    meeting_id
}

fn room_turns() -> Vec<SpeakerTurn> {
    vec![
        turn(0.0, 3.5, "spk_0"),
        turn(5.5, 9.5, "spk_1"),
        turn(12.0, 15.0, "spk_0"),
        turn(15.4, 18.3, "spk_1"),
        turn(21.5, 24.5, "spk_0"),
    ]
}

fn room_embeddings() -> HashMap<String, Vec<f32>> {
    HashMap::from([
        ("spk_0".to_string(), vec![1.0, 0.0, 0.0, 0.0]),
        ("spk_1".to_string(), vec![0.0, 1.0, 0.0, 0.0]),
    ])
}

async fn stored_rows(pool: &sqlx::SqlitePool, meeting_id: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT speaker, channel FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(s, c)| (s.unwrap_or_default(), c))
    .collect()
}

/// specs/0078 task 11 / acceptance 4: two clusters over mic-tagged rows persist as two
/// non-`local` speakers when nobody is identified as the owner, the straddling row is
/// split between them, and no `transcripts.channel` changes. The same rows through the
/// call path all come out "You".
#[tokio::test]
async fn room_routing_keeps_two_mic_voices_apart_and_never_rewrites_channels() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let room = seed_room_meeting(pool).await;
    let (persisted, segments) = attribute_and_persist(
        pool,
        &room,
        &room_turns(),
        &room_embeddings(),
        AudioSetup::Room,
    )
    .await
    .expect("room pass");
    assert_eq!(segments, 5, "the glued handoff row was split in two");
    assert_eq!(persisted, 2);

    let rows = stored_rows(pool, &room).await;
    let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, vec!["spk_0", "spk_1", "spk_0", "spk_1", "spk_0"]);
    assert!(
        rows.iter().all(|(_, c)| c.as_deref() == Some("microphone")),
        "room mode never rewrites transcripts.channel: {rows:?}"
    );
    let speakers = SpeakersRepository::get_by_meeting(pool, &room)
        .await
        .unwrap();
    let mut speaker_keys: Vec<&str> = speakers.iter().map(|s| s.speaker_key.as_str()).collect();
    speaker_keys.sort();
    assert_eq!(
        speaker_keys,
        vec!["spk_0", "spk_1"],
        "no local without an owner"
    );

    // Call mode over the same kind of meeting: every mic row is "You", nothing splits.
    let call = seed_room_meeting(pool).await;
    let (persisted, segments) = attribute_and_persist(
        pool,
        &call,
        &room_turns(),
        &room_embeddings(),
        AudioSetup::Call,
    )
    .await
    .expect("call pass");
    assert_eq!((persisted, segments), (1, 4));
    let rows = stored_rows(pool, &call).await;
    assert!(rows.iter().all(|(k, _)| k == "local"), "{rows:?}");
}

/// In a room pass the owner's cluster, re-keyed to `local`, keeps its embedding (the
/// carry-over and "This is me" read it); a call's `local` stays NULL (ADR-0007 §3).
#[tokio::test]
async fn only_a_room_pass_keeps_an_embedding_on_local() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let mut embeddings = room_embeddings();
    embeddings.insert("local".to_string(), vec![0.0, 0.0, 1.0, 0.0]);
    let with_owner: Vec<SpeakerTurn> = room_turns()
        .into_iter()
        .map(|mut t| {
            if t.speaker == "spk_1" {
                t.speaker = "local".to_string();
            }
            t
        })
        .collect();

    for (setup, expect_embedding) in [(AudioSetup::Room, true), (AudioSetup::Call, false)] {
        let meeting = seed_room_meeting(pool).await;
        attribute_and_persist(pool, &meeting, &with_owner, &embeddings, setup)
            .await
            .unwrap();
        let has: bool = sqlx::query_scalar(
            "SELECT embedding IS NOT NULL FROM speakers WHERE meeting_id = ? AND speaker_key = 'local'",
        )
        .bind(&meeting)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(has, expect_embedding, "{setup:?}");
    }
}

/// Give the owner `n` gallery samples near `voice`.
async fn enroll_owner(pool: &sqlx::SqlitePool, voice: &[f32], n: usize) {
    ensure_owner_person(pool).await.unwrap();
    for _ in 0..n {
        VoiceprintsRepository::add_sample(
            pool,
            OWNER_PERSON_ID,
            voice,
            EMBEDDING_MODEL_ID,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }
}

/// specs/0078 owner decision 3: in a room the owner is an ordinary gallery candidate. With
/// a well-trained voiceprint (TRUSTED_GALLERY_MIN_SAMPLES) and a match above the voice-only
/// bar, the owner's cluster becomes `local`; with a thin one (one sample), the bar is the
/// same as for anyone and nothing is labeled.
#[tokio::test]
async fn a_room_owner_is_labeled_by_the_same_rules_as_anyone() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting = seed_room_meeting(pool).await;
    let owner_voice = [0.95f32, 0.31, 0.0, 0.0]; // cosine ≈ 0.95 to spk_0

    enroll_owner(pool, &owner_voice, 1).await;
    let mut turns = room_turns();
    let mut emb = room_embeddings();
    let thin = label_owner_cluster(pool, &meeting, &mut turns, &mut emb, &[]).await;
    assert_eq!(
        thin, None,
        "one owner sample is a thin gallery: no auto-label"
    );
    assert!(turns.iter().all(|t| t.speaker != "local"));

    enroll_owner(pool, &owner_voice, 2).await; // now 3 samples
    let label = label_owner_cluster(pool, &meeting, &mut turns, &mut emb, &[])
        .await
        .expect("a well-trained owner voiceprint wins its cluster");
    assert_eq!(label.rule, OwnerRule::Voiceprint);
    assert_eq!(label.cluster, "spk_0");
    assert!(turns
        .iter()
        .all(|t| t.speaker == "local" || t.speaker == "spk_1"));
    assert!(emb.contains_key("local") && !emb.contains_key("spk_0"));
}

/// The refetch path (`api_get_speaker_suggestions` → `auto_label::apply`) labels an older
/// room meeting once the owner's voiceprint is well trained, by re-keying the cluster to
/// `local`. A call meeting never offers the owner at all.
#[tokio::test]
async fn the_refetch_turns_a_room_owner_match_into_you() {
    use app_lib::diarization::auto_label;
    use app_lib::diarization::pipeline::compute_suggestions;

    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let owner_voice = [0.95f32, 0.31, 0.0, 0.0];
    enroll_owner(pool, &owner_voice, 3).await;

    for setup in [AudioSetup::Call, AudioSetup::Room] {
        let meeting = seed_room_meeting(pool).await;
        attribute_and_persist(
            pool,
            &meeting,
            &room_turns(),
            &room_embeddings(),
            AudioSetup::Room,
        )
        .await
        .unwrap();
        // The persist recorded `room`; pretend the last pass was `setup`.
        MeetingAudioSetupRepository::set_resolved(pool, &meeting, setup)
            .await
            .unwrap();
        let suggestions = compute_suggestions(pool, &meeting).await.unwrap();
        let owner = suggestions
            .iter()
            .find(|s| s.suggested_person_id.as_deref() == Some(OWNER_PERSON_ID));
        if setup == AudioSetup::Call {
            assert!(owner.is_none(), "the owner never competes in a call");
            continue;
        }
        let owner = owner.expect("the owner competes in a room with no You yet");
        assert!(owner.auto_label);
        assert_eq!(owner.speaker_key, "spk_0");

        assert_eq!(auto_label::apply(pool, &meeting, &suggestions).await, 1);
        let rows = stored_rows(pool, &meeting).await;
        let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["local", "spk_1", "local", "spk_1", "local"]);
        // With a You in place, the owner stops competing.
        let again = compute_suggestions(pool, &meeting).await.unwrap();
        assert!(again
            .iter()
            .all(|s| s.suggested_person_id.as_deref() != Some(OWNER_PERSON_ID)));
    }
}

/// A room meeting persisted by a room pass, with the owner's voiceprint trained on spk_0.
async fn room_meeting_with_trained_owner(pool: &sqlx::SqlitePool) -> String {
    enroll_owner(pool, &[0.95f32, 0.31, 0.0, 0.0], 3).await;
    let meeting = seed_room_meeting(pool).await;
    attribute_and_persist(
        pool,
        &meeting,
        &room_turns(),
        &room_embeddings(),
        AudioSetup::Room,
    )
    .await
    .unwrap();
    meeting
}

async fn has_local(pool: &sqlx::SqlitePool, meeting: &str) -> bool {
    SpeakersRepository::get_by_meeting(pool, meeting)
        .await
        .unwrap()
        .iter()
        .any(|s| s.speaker_key == "local")
}

/// specs/0078 review: "This isn't me" used to undo itself. The refetch after it
/// (`api_get_speaker_suggestions` → `auto_label::apply`) saw a room meeting with no "You"
/// and re-keyed the same cluster straight back. The rejection is now sticky, and "This is
/// me" lifts it.
#[tokio::test]
async fn this_isnt_me_survives_the_suggestion_refetch() {
    use app_lib::diarization::auto_label;
    use app_lib::diarization::pipeline::compute_suggestions;

    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting = room_meeting_with_trained_owner(pool).await;
    let suggestions = compute_suggestions(pool, &meeting).await.unwrap();
    assert_eq!(auto_label::apply(pool, &meeting, &suggestions).await, 1);
    assert!(
        has_local(pool, &meeting).await,
        "the owner auto-labeled spk_0"
    );

    let out = SpeakersRepository::rekey_from_local(pool, &meeting, OWNER_PERSON_ID)
        .await
        .unwrap()
        .expect("a You to unmark");
    assert!(!has_local(pool, &meeting).await);

    let suggestions = compute_suggestions(pool, &meeting).await.unwrap();
    assert!(
        suggestions
            .iter()
            .all(|s| s.suggested_person_id.as_deref() != Some(OWNER_PERSON_ID)),
        "the owner no longer competes here: {suggestions:?}"
    );
    auto_label::apply(pool, &meeting, &suggestions).await;
    assert!(
        !has_local(pool, &meeting).await,
        "the refetch must not re-key it back"
    );
    let rows = stored_rows(pool, &meeting).await;
    assert!(rows.iter().all(|(k, _)| k != "local"), "{rows:?}");

    // "This is me" on the same voice lifts the rejection.
    SpeakersRepository::rekey_to_local(pool, &meeting, &out.new_key, OWNER_PERSON_ID, true)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !MeetingAudioSetupRepository::owner_label_rejected(pool, &meeting)
            .await
            .unwrap()
    );
}

/// The same rejection holds in a re-run pass: neither the owner's voiceprint (rule 2) nor
/// the previous "You" (rule 3) labels a cluster. A single voice is still "You" (rule 1).
#[tokio::test]
async fn this_isnt_me_turns_off_the_voiceprint_and_carry_over_rules_in_a_pass() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting = room_meeting_with_trained_owner(pool).await;
    let mut turns = room_turns();
    let mut emb = room_embeddings();
    assert!(
        label_owner_cluster(pool, &meeting, &mut turns, &mut emb, &[])
            .await
            .is_some(),
        "precondition: the trained voiceprint wins spk_0"
    );

    // A "You" whose embedding matches spk_0, then "This isn't me".
    SpeakersRepository::rekey_to_local(pool, &meeting, "spk_0", OWNER_PERSON_ID, false)
        .await
        .unwrap()
        .unwrap();
    SpeakersRepository::rekey_from_local(pool, &meeting, OWNER_PERSON_ID)
        .await
        .unwrap()
        .unwrap();

    let mut turns = room_turns();
    let mut emb = room_embeddings();
    assert_eq!(
        label_owner_cluster(pool, &meeting, &mut turns, &mut emb, &[]).await,
        None
    );
    assert!(turns.iter().all(|t| t.speaker != "local"));

    let mut solo: Vec<SpeakerTurn> = vec![turn(0.0, 3.0, "spk_0")];
    let mut solo_emb = HashMap::from([("spk_0".to_string(), vec![1.0, 0.0, 0.0, 0.0])]);
    let label = label_owner_cluster(pool, &meeting, &mut solo, &mut solo_emb, &[])
        .await
        .expect("a single voice is still the owner");
    assert_eq!(label.rule, OwnerRule::SingleCluster);
}

/// specs/0078 review: `audio_setup_resolved` is written with the pass's rows, so a pass
/// that fails before persisting leaves the previous value, and resolving a pass's input
/// writes nothing.
#[tokio::test]
async fn a_failed_pass_leaves_the_previous_resolved_setup() {
    use app_lib::database::repositories::meeting_audio_setup::MeetingAudioSetup;
    use app_lib::diarization::room::resolve_diarization_input;
    use app_lib::diarization::room_types::AudioSetupOverride;

    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting = seed_room_meeting(pool).await;
    MeetingAudioSetupRepository::set_resolved(pool, &meeting, AudioSetup::Call)
        .await
        .unwrap();
    MeetingAudioSetupRepository::set_override(pool, &meeting, AudioSetupOverride::Room)
        .await
        .unwrap();
    let folder = tempfile::tempdir().unwrap();
    // A decodable mic track, so detection would produce an activity if it ran.
    common::write_wav_16k(
        &folder.path().join("mic.wav"),
        &common::silence(2.0, 16_000),
    );
    MeetingsRepository::update_folder_path(pool, &meeting, folder.path().to_str().unwrap())
        .await
        .unwrap();
    let resolved = |m: Option<MeetingAudioSetup>| m.unwrap().resolved;

    let input = resolve_diarization_input(pool, &meeting).await.unwrap();
    assert_eq!(input.setup, AudioSetup::Room);
    assert_eq!(
        input.activity, None,
        "an override skips detection (it would decode both tracks for nothing)"
    );
    assert_eq!(
        resolved(
            MeetingAudioSetupRepository::get(pool, &meeting)
                .await
                .unwrap()
        ),
        Some(AudioSetup::Call),
        "resolving the input records nothing"
    );

    // The pass fails before it persists: the meeting lost its timed rows.
    sqlx::query("UPDATE transcripts SET audio_start_time = NULL WHERE meeting_id = ?")
        .bind(&meeting)
        .execute(pool)
        .await
        .unwrap();
    assert!(attribute_and_persist(
        pool,
        &meeting,
        &room_turns(),
        &room_embeddings(),
        AudioSetup::Room
    )
    .await
    .is_err());
    assert_eq!(
        resolved(
            MeetingAudioSetupRepository::get(pool, &meeting)
                .await
                .unwrap()
        ),
        Some(AudioSetup::Call)
    );

    // A pass that persists records its setup.
    let ok = seed_room_meeting(pool).await;
    attribute_and_persist(
        pool,
        &ok,
        &room_turns(),
        &room_embeddings(),
        AudioSetup::Room,
    )
    .await
    .unwrap();
    assert_eq!(
        resolved(MeetingAudioSetupRepository::get(pool, &ok).await.unwrap()),
        Some(AudioSetup::Room)
    );
}

/// specs/0078 review: in a room pass `local` carries the owner's embedding, so a renamed
/// speaker whose key didn't survive the re-run could have its name restored onto "You"
/// by centroid similarity. The owner's identity is fixed: `local` is never a target.
#[tokio::test]
async fn a_rename_is_never_restored_onto_you() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting = seed_room_meeting(pool).await;
    attribute_and_persist(
        pool,
        &meeting,
        &room_turns(),
        &room_embeddings(),
        AudioSetup::Room,
    )
    .await
    .unwrap();
    SpeakersRepository::rename(pool, &meeting, "spk_0", "Priya")
        .await
        .unwrap();

    // The re-run: spk_0's voice is now the owner's cluster, and no other cluster is close.
    let turns: Vec<SpeakerTurn> = room_turns()
        .into_iter()
        .map(|mut t| {
            if t.speaker == "spk_0" {
                t.speaker = "local".to_string();
            }
            t
        })
        .collect();
    let emb = HashMap::from([
        ("local".to_string(), vec![1.0, 0.0, 0.0, 0.0]),
        ("spk_1".to_string(), vec![0.0, 1.0, 0.0, 0.0]),
    ]);
    attribute_and_persist(pool, &meeting, &turns, &emb, AudioSetup::Room)
        .await
        .unwrap();
    let speakers = SpeakersRepository::get_by_meeting(pool, &meeting)
        .await
        .unwrap();
    let you = speakers.iter().find(|s| s.speaker_key == "local").unwrap();
    assert_eq!(you.display_name, "You", "{speakers:?}");
    assert!(speakers.iter().all(|s| s.display_name != "Priya"));
}

/// specs/0078 follow-up (fix 3): a room cluster the user assigned to themself before that
/// meant "This is me" is carried onto its key by the re-run's identity restore; the pass
/// then re-keys it to "You" instead of leaving the owner as a stranger `spk_N`. A call
/// re-run keeps today's link.
#[tokio::test]
async fn a_rerun_turns_a_room_cluster_assigned_to_you_into_you() {
    use app_lib::database::repositories::people::PeopleRepository;

    for (setup, expect_you) in [(AudioSetup::Room, true), (AudioSetup::Call, false)] {
        let (_dir, db) = fresh_db().await;
        let pool = db.pool();
        ensure_owner_person(pool).await.unwrap();
        let meeting = seed_room_meeting(pool).await;
        if setup == AudioSetup::Call {
            // In a call only system rows are clustered: make the second voice remote.
            sqlx::query(
                "UPDATE transcripts SET channel = 'system'
                 WHERE meeting_id = ? AND transcript LIKE 'second%'",
            )
            .bind(&meeting)
            .execute(pool)
            .await
            .unwrap();
        }
        // The first pass identified nobody as the owner...
        let first = room_turns();
        attribute_and_persist(pool, &meeting, &first, &room_embeddings(), setup)
            .await
            .unwrap();
        let cluster = if setup == AudioSetup::Room {
            "spk_0"
        } else {
            "spk_1"
        };
        // ...then the user linked a cluster to the owner person the old way.
        assert!(PeopleRepository::assign_speaker_to_person(
            pool,
            &meeting,
            cluster,
            OWNER_PERSON_ID
        )
        .await
        .unwrap());

        attribute_and_persist(pool, &meeting, &first, &room_embeddings(), setup)
            .await
            .unwrap();
        let speakers = SpeakersRepository::get_by_meeting(pool, &meeting)
            .await
            .unwrap();
        let owner_linked: Vec<&str> = speakers
            .iter()
            .filter(|s| s.person_id.as_deref() == Some(OWNER_PERSON_ID))
            .map(|s| s.speaker_key.as_str())
            .collect();
        if expect_you {
            assert_eq!(owner_linked, vec!["local"], "{setup:?}: {speakers:?}");
            let rows = stored_rows(pool, &meeting).await;
            let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(keys, vec!["local", "spk_1", "local", "spk_1", "local"]);
        } else {
            assert_eq!(owner_linked, vec![cluster], "{setup:?}: {speakers:?}");
        }
    }
}
