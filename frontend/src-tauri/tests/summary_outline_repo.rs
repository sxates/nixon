//! specs/0053 W3: the derived Auto outline must survive across runs, so a
//! regenerated summary keeps its shape and the summary cache can fingerprint it.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

use app_lib::database::repositories::summary_outline::SummaryOutlineRepository;

async fn test_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations");
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','now','now')")
        .execute(&pool)
        .await
        .expect("seed meeting");
    pool
}

#[tokio::test]
async fn absent_outline_reads_as_none() {
    let pool = test_pool().await;
    assert!(SummaryOutlineRepository::get(&pool, "m1").await.unwrap().is_none());
}

#[tokio::test]
async fn upsert_then_get_round_trips() {
    let pool = test_pool().await;
    SummaryOutlineRepository::upsert(&pool, "m1", r#"{"sections":[]}"#, true)
        .await
        .unwrap();

    let stored = SummaryOutlineRepository::get(&pool, "m1").await.unwrap().unwrap();
    assert_eq!(stored.outline_json, r#"{"sections":[]}"#);
    assert!(stored.has_commitments);
    assert!(!stored.derived_at.is_empty());
}

/// Re-deriving must replace, not accumulate — the meeting_id is the primary key.
#[tokio::test]
async fn upsert_replaces_an_existing_outline() {
    let pool = test_pool().await;
    SummaryOutlineRepository::upsert(&pool, "m1", r#"{"v":1}"#, true).await.unwrap();
    SummaryOutlineRepository::upsert(&pool, "m1", r#"{"v":2}"#, false).await.unwrap();

    let stored = SummaryOutlineRepository::get(&pool, "m1").await.unwrap().unwrap();
    assert_eq!(stored.outline_json, r#"{"v":2}"#);
    assert!(!stored.has_commitments);
}

/// "Re-think structure" clears the row so the next run derives fresh.
#[tokio::test]
async fn delete_removes_the_outline() {
    let pool = test_pool().await;
    SummaryOutlineRepository::upsert(&pool, "m1", "{}", false).await.unwrap();
    SummaryOutlineRepository::delete(&pool, "m1").await.unwrap();
    assert!(SummaryOutlineRepository::get(&pool, "m1").await.unwrap().is_none());
}

/// Deleting a meeting must not strand its outline.
#[tokio::test]
async fn outline_is_cascade_deleted_with_its_meeting() {
    let pool = test_pool().await;
    sqlx::query("PRAGMA foreign_keys = ON").execute(&pool).await.unwrap();
    SummaryOutlineRepository::upsert(&pool, "m1", "{}", false).await.unwrap();
    sqlx::query("DELETE FROM meetings WHERE id = 'm1'").execute(&pool).await.unwrap();
    assert!(SummaryOutlineRepository::get(&pool, "m1").await.unwrap().is_none());
}
