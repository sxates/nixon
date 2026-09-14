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
