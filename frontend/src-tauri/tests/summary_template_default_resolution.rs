//! specs/0053 C1 regression: a brand-new meeting's `template_id` is NULL until
//! the user explicitly picks a template (`persistSelection` only fires on an
//! explicit choice). Both NULL-resolution sites in `summary::commands`
//! (`api_process_transcript` and `start_summary_generation_for_meeting`) must
//! fall back to Auto — never to a fixed template — or the offline-diarization
//! refresh path silently overwrites an Auto-shaped summary with
//! `standard_meeting`'s forced four-section shape.
//!
//! This exercises the same two building blocks those call sites compose
//! (`MeetingsRepository::get_meeting_template` + the shared default constant)
//! rather than the `#[tauri::command]` functions themselves, which need a
//! live `AppHandle<R: Runtime>` and are out of reach for a unit-style test.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::summary::outline::AUTO_TEMPLATE_ID;
use app_lib::summary::templates::{DEFAULT_SUMMARY_TEMPLATE_ID, DEFAULT_TEMPLATE_ID};

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
    pool
}

/// The constant itself must point at Auto, not at the old fixed default —
/// this is the crux of the regression.
#[test]
fn default_summary_template_id_is_auto() {
    assert_eq!(DEFAULT_SUMMARY_TEMPLATE_ID, AUTO_TEMPLATE_ID);
    assert_ne!(
        DEFAULT_SUMMARY_TEMPLATE_ID, DEFAULT_TEMPLATE_ID,
        "the summary default must NOT be the fixed-template default any longer"
    );
}

/// A brand-new meeting row (no explicit template choice ever made) must read
/// back as NULL from the DB, and the two commands.rs call sites resolve that
/// NULL via `.unwrap_or_else(|| DEFAULT_SUMMARY_TEMPLATE_ID.to_string())`.
/// Reproduce that exact resolution here and confirm it lands on "auto".
#[tokio::test]
async fn a_meeting_with_no_persisted_template_resolves_to_auto() {
    let pool = test_pool().await;
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','now','now')",
    )
    .execute(&pool)
    .await
    .expect("seed meeting with NULL template_id");

    let persisted = MeetingsRepository::get_meeting_template(&pool, "m1")
        .await
        .expect("query must succeed");
    assert!(
        persisted.is_none(),
        "a fresh meeting must have no explicit template choice"
    );

    let resolved = persisted.unwrap_or_else(|| DEFAULT_SUMMARY_TEMPLATE_ID.to_string());
    assert_eq!(
        resolved, AUTO_TEMPLATE_ID,
        "NULL template_id must resolve to Auto, not the old fixed default"
    );
}
