//! specs/0059: the seeder writes the embedded dataset through the repositories.
#![cfg(debug_assertions)]
mod common;

use app_lib::dev_fixtures::{dataset, seed};
use std::collections::HashMap;

#[tokio::test]
async fn seeds_embedded_dataset_and_is_idempotent() {
    let (_dir, db) = common::fresh_db().await;
    let ds = dataset::load_embedded().unwrap();
    let now = chrono::Utc::now();

    let r1 = seed::seed_all(db.pool(), &ds, &HashMap::new(), now)
        .await
        .unwrap();
    assert_eq!(r1.meetings, 5);
    assert_eq!(r1.people, 7);
    // specs/0059 Ruling 1: the embedded dataset has 846 segments.
    assert!(
        r1.segments > 700,
        "expected ~846 segments, got {}",
        r1.segments
    );

    let (n_meet,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM meetings")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(n_meet, 5);
    let (id,): (String,) =
        sqlx::query_as("SELECT id FROM meetings WHERE title LIKE 'Product sync%'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(id, "demo-01");

    // speakers for the steerco incl. the local row, linked to people where the fixture says so
    let (n_spk,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM speakers WHERE meeting_id = 'demo-03'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(n_spk, 6);
    let (is_local,): (i64,) = sqlx::query_as(
        "SELECT is_local FROM speakers WHERE meeting_id='demo-03' AND speaker_key='local'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(is_local, 1);
    let (linked,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM speakers WHERE meeting_id='demo-01' AND person_id IS NOT NULL",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(linked, 3);

    // summary completed for 01, absent for 05
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM summary_processes WHERE meeting_id='demo-01'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(status, "completed");
    let (n_sum05,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM summary_processes WHERE meeting_id='demo-05'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(n_sum05, 0);

    // action items carry a content_key and self-assignment
    let (n_self,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM action_items WHERE meeting_id='demo-01' AND assignee_is_self=1",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(n_self, 2);

    // transcripts kept channel + speaker
    let (n_mic,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transcripts WHERE meeting_id='demo-01' AND channel='microphone' AND speaker='local'").fetch_one(db.pool()).await.unwrap();
    assert!(n_mic > 10);

    // dates re-based: demo-01 is today at 14:00 local
    let (created,): (String,) =
        sqlx::query_as("SELECT created_at FROM meetings WHERE id='demo-01'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    let expected = seed::started_at(&ds.meetings[0], now);
    assert!(
        created.starts_with(&expected.format("%Y-%m-%dT%H:%M").to_string()),
        "{created} vs {expected}"
    );

    // idempotent
    let r2 = seed::seed_all(db.pool(), &ds, &HashMap::new(), now)
        .await
        .unwrap();
    assert_eq!(r2.meetings, 5);
    let (n_meet2,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM meetings")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(n_meet2, 5);
    let (n_people,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM people")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(n_people, 7);
}

#[tokio::test]
async fn wipe_leaves_settings_alone() {
    let (_dir, db) = common::fresh_db().await;
    sqlx::query("INSERT OR REPLACE INTO settings (id, provider, model, whisperModel) VALUES ('settings', 'ollama', 'x', 'y')")
        .execute(db.pool()).await.unwrap();
    let (before,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM settings")
        .fetch_one(db.pool())
        .await
        .unwrap();
    seed::wipe(db.pool()).await.unwrap();
    let (after,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM settings")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(before, after);
}
