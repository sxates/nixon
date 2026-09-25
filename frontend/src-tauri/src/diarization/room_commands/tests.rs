//! DB tests for the room commands' cores (specs/0078).

use super::*;
use crate::diarization::embedding::{embedding_to_bytes, l2_normalize, EMBEDDING_MODEL_ID};

async fn pool_with_meeting(id: &str) -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, 't', ?, ?)")
        .bind(id)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
    pool
}

/// A clustered speaker with a real embedding and `lines` transcript rows.
async fn cluster(pool: &SqlitePool, m: &str, key: &str, v: &[f32], lines: usize) {
    let emb = embedding_to_bytes(&l2_normalize(v));
    SpeakersRepository::upsert(
        pool,
        m,
        key,
        &crate::diarization::pipeline::display_name_for_key(key),
        false,
        Some(&emb),
        Some(v.len() as i64),
        Some(EMBEDDING_MODEL_ID),
    )
    .await
    .unwrap();
    for i in 0..lines {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker)
             VALUES (?, ?, 'hi', ?, ?)",
        )
        .bind(format!("{key}-{i}"))
        .bind(m)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(key)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// `(live, quarantined)` owner samples back-linked to this meeting's `local`.
async fn owner_samples(pool: &SqlitePool, m: &str) -> (i64, i64) {
    sqlx::query_as(
        "SELECT COALESCE(SUM(quarantined_at IS NULL), 0), COALESCE(SUM(quarantined_at IS NOT NULL), 0)
         FROM voiceprints WHERE person_id = ? AND source_meeting_id = ? AND source_speaker_key = 'local'",
    )
    .bind(OWNER_PERSON_ID)
    .bind(m)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn this_is_me_enrolls_one_owner_sample_when_voiceprints_are_on() {
    let pool = pool_with_meeting("m1").await;
    cluster(&pool, "m1", "spk_0", &[1.0, 0.0, 0.0], 3).await;
    cluster(&pool, "m1", "spk_1", &[0.0, 1.0, 0.0], 2).await;

    let out = mark_speaker_as_me(&pool, "m1", "spk_0", true)
        .await
        .unwrap();
    assert!(out.enrolled);
    assert_eq!(out.rekey.moved_lines, 3);
    assert_eq!(owner_samples(&pool, "m1").await, (1, 0));
    let speakers = SpeakersRepository::get_by_meeting(&pool, "m1")
        .await
        .unwrap();
    let you = speakers.iter().find(|s| s.speaker_key == "local").unwrap();
    assert_eq!(you.display_name, "You");
    assert_eq!(you.person_id.as_deref(), Some(OWNER_PERSON_ID));

    // A second cluster folds into the same `local`; no second sample for this meeting.
    let out = mark_speaker_as_me(&pool, "m1", "spk_1", true)
        .await
        .unwrap();
    assert!(out.rekey.merged_into_existing);
    assert!(!out.enrolled);
    assert_eq!(owner_samples(&pool, "m1").await, (1, 0));
}

#[tokio::test]
async fn this_is_me_writes_no_voiceprint_when_voiceprints_are_off() {
    let pool = pool_with_meeting("m1").await;
    cluster(&pool, "m1", "spk_0", &[1.0, 0.0, 0.0], 2).await;

    let out = mark_speaker_as_me(&pool, "m1", "spk_0", false)
        .await
        .unwrap();
    assert!(!out.enrolled);
    assert_eq!(out.rekey.moved_lines, 2, "the label still moves");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voiceprints")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn this_isnt_me_quarantines_the_sample_and_restores_a_speaker() {
    let pool = pool_with_meeting("m1").await;
    cluster(&pool, "m1", "spk_0", &[1.0, 0.0, 0.0], 2).await;
    cluster(&pool, "m1", "spk_1", &[0.0, 1.0, 0.0], 1).await;
    mark_speaker_as_me(&pool, "m1", "spk_0", true)
        .await
        .unwrap();
    assert_eq!(owner_samples(&pool, "m1").await, (1, 0));

    let out = unmark_speaker_as_me(&pool, "m1").await.unwrap();
    assert_eq!(out.new_key, "spk_2");
    assert_eq!(out.quarantined_owner_samples, 1);
    assert_eq!(owner_samples(&pool, "m1").await, (0, 1));
    let keys: Vec<String> = SpeakersRepository::get_by_meeting(&pool, "m1")
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.speaker_key)
        .collect();
    assert_eq!(keys, vec!["spk_1".to_string(), "spk_2".to_string()]);

    // Marking again enrolls a fresh sample: the quarantined one isn't live.
    let out = mark_speaker_as_me(&pool, "m1", "spk_2", true)
        .await
        .unwrap();
    assert!(out.enrolled);
    assert_eq!(owner_samples(&pool, "m1").await, (1, 1));
}

#[tokio::test]
async fn marking_refuses_local_unknown_and_missing_speakers() {
    let pool = pool_with_meeting("m1").await;
    // The mixed overflow bucket exists as a row, so only the explicit guard refuses it.
    cluster(&pool, "m1", "unknown", &[0.0, 0.0, 1.0], 1).await;
    cluster(&pool, "m1", "local", &[0.0, 1.0, 1.0], 1).await;
    for key in ["local", "unknown", "spk_7", " "] {
        assert!(
            mark_speaker_as_me(&pool, "m1", key, true).await.is_err(),
            "{key:?} must be refused"
        );
    }
    let empty = pool_with_meeting("m2").await;
    assert!(
        unmark_speaker_as_me(&empty, "m2").await.is_err(),
        "no You to unmark"
    );
}

#[tokio::test]
async fn the_override_round_trips_and_bad_input_is_refused() {
    let pool = pool_with_meeting("m1").await;
    assert_eq!(
        get_audio_setup(&pool, "m1").await.unwrap(),
        MeetingAudioSetupDto {
            override_setup: "auto",
            resolved: None
        }
    );
    set_audio_setup(&pool, "m1", "room").await.unwrap();
    assert_eq!(
        get_audio_setup(&pool, "m1").await.unwrap().override_setup,
        "room"
    );
    let json = serde_json::to_value(get_audio_setup(&pool, "m1").await.unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "override": "room", "resolved": null })
    );

    assert!(set_audio_setup(&pool, "m1", "hybrid").await.is_err());
    assert!(set_audio_setup(&pool, "missing", "call").await.is_err());
    assert!(get_audio_setup(&pool, "missing").await.is_err());
}

/// specs/0078 review: "This is me" folding a cluster into an automatic "You" used to
/// enroll `local`'s old embedding (possibly the wrong voice) instead of the voice the user
/// just named. Both the sample and `local` now carry the confirmed cluster's embedding.
#[tokio::test]
async fn this_is_me_into_an_automatic_you_enrolls_the_confirmed_voice() {
    let pool = pool_with_meeting("m1").await;
    let wrong = [0.0, 1.0, 0.0];
    let right = [1.0, 0.0, 0.0];
    cluster(&pool, "m1", "local", &wrong, 2).await; // an automatic "You", never enrolled
    cluster(&pool, "m1", "spk_0", &right, 3).await;

    let out = mark_speaker_as_me(&pool, "m1", "spk_0", true)
        .await
        .unwrap();
    assert!(out.rekey.merged_into_existing);
    assert!(out.enrolled);

    let right_bytes = embedding_to_bytes(&l2_normalize(&right));
    let sample: Vec<u8> = sqlx::query_scalar(
        "SELECT embedding FROM voiceprints WHERE person_id = ? AND source_meeting_id = 'm1'",
    )
    .bind(OWNER_PERSON_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sample, right_bytes, "the confirmed voice is enrolled");
    let (_, local_bytes, _) = SpeakersRepository::get_speaker_embedding(&pool, "m1", "local")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(local_bytes, right_bytes, "and becomes local's voice");
}
