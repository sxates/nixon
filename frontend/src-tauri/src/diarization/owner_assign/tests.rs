//! DB tests for assigning a speaker to yourself (specs/0078 follow-up).

use super::*;
use crate::diarization::embedding::{embedding_to_bytes, l2_normalize, EMBEDDING_MODEL_ID};

const ALIAS: &str = "me@example.com";

/// A meeting whose last pass ran as `resolved` (`None`: no pass recorded yet).
async fn pool_with_meeting(id: &str, resolved: Option<&str>) -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, audio_setup_resolved)
         VALUES (?, 't', ?, ?, ?)",
    )
    .bind(id)
    .bind(&now)
    .bind(&now)
    .bind(resolved)
    .execute(&pool)
    .await
    .unwrap();
    enroll::ensure_owner_person(&pool).await.unwrap();
    pool
}

/// A speaker row with a real embedding and `lines` transcript rows.
async fn speaker(pool: &SqlitePool, m: &str, key: &str, v: &[f32], lines: usize) {
    let emb = embedding_to_bytes(&l2_normalize(v));
    SpeakersRepository::upsert(
        pool,
        m,
        key,
        &crate::diarization::pipeline::display_name_for_key(key),
        key == LOCAL_SPEAKER_KEY,
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

/// `(speaker_key, is_local, person_id)` for every row, by key.
async fn rows(pool: &SqlitePool, m: &str) -> Vec<(String, i64, Option<String>)> {
    sqlx::query_as(
        "SELECT speaker_key, is_local, person_id FROM speakers WHERE meeting_id = ?
         ORDER BY speaker_key",
    )
    .bind(m)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn lines_on(pool: &SqlitePool, m: &str, key: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ? AND speaker = ?")
        .bind(m)
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Owner voiceprint samples as `source_speaker_key`s, sorted.
async fn owner_sample_keys(pool: &SqlitePool) -> Vec<Option<String>> {
    sqlx::query_scalar(
        "SELECT source_speaker_key FROM voiceprints WHERE person_id = ?
         ORDER BY source_speaker_key",
    )
    .bind(OWNER_PERSON_ID)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn owner_label(pool: &SqlitePool, m: &str) -> Option<String> {
    sqlx::query_scalar("SELECT owner_label FROM meetings WHERE id = ?")
        .bind(m)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn two_clusters(resolved: Option<&str>) -> SqlitePool {
    let pool = pool_with_meeting("m1", resolved).await;
    speaker(&pool, "m1", "spk_0", &[1.0, 0.0, 0.0], 3).await;
    speaker(&pool, "m1", "spk_1", &[0.0, 1.0, 0.0], 2).await;
    pool
}

/// The room outcome of every route: `spk_0` became "You", confirmed, enrolled ONCE.
async fn assert_became_you(pool: &SqlitePool) {
    assert_eq!(
        rows(pool, "m1").await,
        vec![
            ("local".to_string(), 1, Some(OWNER_PERSON_ID.to_string())),
            ("spk_1".to_string(), 0, None),
        ]
    );
    assert_eq!(lines_on(pool, "m1", "local").await, 3);
    assert_eq!(owner_label(pool, "m1").await.as_deref(), Some("confirmed"));
    assert_eq!(
        owner_sample_keys(pool).await,
        vec![Some("local".to_string())],
        "exactly one owner sample, back-linked to You"
    );
}

/// The call (or unresolved) outcome: today's link-to-owner, owner-path enrollment.
async fn assert_linked_not_rekeyed(pool: &SqlitePool) {
    assert_eq!(
        rows(pool, "m1").await,
        vec![
            ("spk_0".to_string(), 0, Some(OWNER_PERSON_ID.to_string())),
            ("spk_1".to_string(), 0, None),
        ]
    );
    assert_eq!(lines_on(pool, "m1", "local").await, 0);
    assert_eq!(owner_label(pool, "m1").await, None);
    assert_eq!(
        owner_sample_keys(pool).await,
        vec![Some("spk_0".to_string())]
    );
}

// ----- fix 1: every route, room vs call -----

#[tokio::test]
async fn assigning_a_room_cluster_to_the_owner_person_is_this_is_me() {
    for resolved in ["room", "hybrid"] {
        let pool = two_clusters(Some(resolved)).await;
        assign_speaker_to_person(&pool, "m1", "spk_0", OWNER_PERSON_ID, true)
            .await
            .unwrap();
        assert_became_you(&pool).await;
    }
}

#[tokio::test]
async fn assigning_a_call_cluster_to_the_owner_person_only_links_it() {
    for resolved in [Some("call"), None] {
        let pool = two_clusters(resolved).await;
        assign_speaker_to_person(&pool, "m1", "spk_0", OWNER_PERSON_ID, true)
            .await
            .unwrap();
        assert_linked_not_rekeyed(&pool).await;
    }
}

#[tokio::test]
async fn assigning_a_room_cluster_to_an_owner_email_attendee_is_this_is_me() {
    let pool = two_clusters(Some("room")).await;
    OwnerEmailsRepository::add(&pool, ALIAS).await.unwrap();
    assign_speaker_to_attendee(&pool, "m1", "spk_0", "Me", ALIAS, true)
        .await
        .unwrap();
    assert_became_you(&pool).await;
}

#[tokio::test]
async fn assigning_a_call_cluster_to_an_owner_email_attendee_only_links_it() {
    for resolved in [Some("call"), None] {
        let pool = two_clusters(resolved).await;
        OwnerEmailsRepository::add(&pool, ALIAS).await.unwrap();
        assign_speaker_to_attendee(&pool, "m1", "spk_0", "Me", ALIAS, true)
            .await
            .unwrap();
        assert_linked_not_rekeyed(&pool).await;
    }
}

/// A person created after the address was declared as the owner's is not folded into the
/// owner, but assigning a cluster to them still says "this is me".
#[tokio::test]
async fn assigning_a_room_cluster_to_a_person_with_an_owner_email_is_this_is_me() {
    let pool = two_clusters(Some("room")).await;
    OwnerEmailsRepository::add(&pool, ALIAS).await.unwrap();
    let alias = PeopleRepository::create(&pool, "Me (work)", Some(ALIAS), None, None)
        .await
        .unwrap();
    assert_ne!(alias.id, OWNER_PERSON_ID);
    assign_speaker_to_person(&pool, "m1", "spk_0", &alias.id, true)
        .await
        .unwrap();
    assert_became_you(&pool).await;
}

#[tokio::test]
async fn assigning_a_room_cluster_to_someone_else_is_unchanged() {
    let pool = two_clusters(Some("room")).await;
    let priya = PeopleRepository::create(&pool, "Priya", Some("p@example.com"), None, None)
        .await
        .unwrap();
    assign_speaker_to_person(&pool, "m1", "spk_0", &priya.id, true)
        .await
        .unwrap();
    assert_eq!(
        rows(&pool, "m1").await[0],
        ("spk_0".to_string(), 0, Some(priya.id))
    );
    assert_eq!(owner_label(&pool, "m1").await, None);
}

/// Fix 2 through a fix 1 route: an automatic "You" is displaced, not merged into.
#[tokio::test]
async fn assigning_to_yourself_displaces_an_automatic_you_in_a_room() {
    let pool = two_clusters(Some("room")).await;
    speaker(&pool, "m1", "local", &[0.0, 0.0, 1.0], 4).await;
    assign_speaker_to_person(&pool, "m1", "spk_0", OWNER_PERSON_ID, true)
        .await
        .unwrap();
    assert_eq!(
        lines_on(&pool, "m1", "local").await,
        3,
        "only the claimed lines"
    );
    assert_eq!(
        lines_on(&pool, "m1", "spk_2").await,
        4,
        "the guess moved out"
    );
    assert_eq!(owner_label(&pool, "m1").await.as_deref(), Some("confirmed"));
}

/// Fix 2 on the direct "This is me": room swaps, call merges.
#[tokio::test]
async fn this_is_me_swaps_an_automatic_you_only_in_a_room() {
    for (resolved, you_lines, displaced) in [("room", 3, Some("spk_2")), ("call", 7, None)] {
        let pool = two_clusters(Some(resolved)).await;
        speaker(&pool, "m1", "local", &[0.0, 0.0, 1.0], 4).await;
        let out = room_commands::mark_speaker_as_me(&pool, "m1", "spk_0", true)
            .await
            .unwrap();
        assert_eq!(out.rekey.displaced_to.as_deref(), displaced, "{resolved}");
        assert_eq!(
            lines_on(&pool, "m1", "local").await,
            you_lines,
            "{resolved}"
        );
    }
}

// ----- fix 3: an existing assign-to-self becomes "You" on read -----

/// The state an owner assignment left behind before this fix: the cluster linked to the
/// owner person, its sample back-linked to the cluster.
async fn link_to_owner_the_old_way(pool: &SqlitePool, key: &str) {
    PeopleRepository::assign_speaker_to_person(pool, "m1", key, OWNER_PERSON_ID)
        .await
        .unwrap();
    enroll::enroll_speaker_gated(
        pool,
        "m1",
        key,
        OWNER_PERSON_ID,
        EnrollConfidence::UserConfirmed,
        false,
        true,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_owner_linked_room_cluster_becomes_you_on_read_without_enrolling() {
    let pool = two_clusters(Some("room")).await;
    link_to_owner_the_old_way(&pool, "spk_0").await;
    assert_eq!(
        owner_sample_keys(&pool).await,
        vec![Some("spk_0".to_string())]
    );

    assert_eq!(convert_owner_links(&pool, "m1").await.unwrap(), 1);
    // `assert_became_you` checks exactly one sample: the old one, now on `local`.
    assert_became_you(&pool).await;
    assert_eq!(
        convert_owner_links(&pool, "m1").await.unwrap(),
        0,
        "idempotent"
    );
}

#[tokio::test]
async fn several_owner_linked_room_clusters_all_merge_into_you() {
    let pool = pool_with_meeting("m1", Some("room")).await;
    speaker(&pool, "m1", "spk_0", &[1.0, 0.0, 0.0], 1).await;
    speaker(&pool, "m1", "spk_1", &[0.0, 1.0, 0.0], 2).await;
    speaker(&pool, "m1", "spk_2", &[0.0, 0.0, 1.0], 4).await;
    link_to_owner_the_old_way(&pool, "spk_0").await;
    link_to_owner_the_old_way(&pool, "spk_2").await;

    assert_eq!(convert_owner_links(&pool, "m1").await.unwrap(), 2);
    assert_eq!(
        rows(&pool, "m1").await,
        vec![
            ("local".to_string(), 1, Some(OWNER_PERSON_ID.to_string())),
            ("spk_1".to_string(), 0, None),
        ]
    );
    assert_eq!(lines_on(&pool, "m1", "local").await, 5);
    // The biggest cluster's voice is You's.
    let (_, bytes, _) = SpeakersRepository::get_speaker_embedding(&pool, "m1", "local")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, embedding_to_bytes(&l2_normalize(&[0.0, 0.0, 1.0])));
    assert_eq!(
        owner_sample_keys(&pool).await,
        vec![Some("local".to_string()), Some("local".to_string())],
        "no new samples; both old ones follow their cluster"
    );
}

#[tokio::test]
async fn owner_linked_clusters_in_a_call_are_left_alone() {
    for resolved in [Some("call"), None] {
        let pool = two_clusters(resolved).await;
        link_to_owner_the_old_way(&pool, "spk_0").await;
        assert_eq!(convert_owner_links(&pool, "m1").await.unwrap(), 0);
        assert_linked_not_rekeyed(&pool).await;
    }
}
