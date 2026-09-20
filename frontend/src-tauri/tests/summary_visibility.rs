//! A saved summary is readable regardless of how it got there (specs/0066).
//!
//! `get_summary_data_for_meeting` used to `JOIN transcript_chunks`, a table written in
//! exactly one place — the summary *generation* entry points. Any summary that arrived by
//! another route therefore read back as "no summary at all", with its markdown sitting
//! untouched in `summary_processes`. The demo dataset seeds summaries directly, so every
//! seeded meeting showed "No Summary Generated Yet", and a screenshot of that empty tab
//! reached the README twice before anyone questioned it.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

use app_lib::database::repositories::summary::SummaryProcessesRepository;

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
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','now','now')",
    )
    .execute(&pool)
    .await
    .expect("seed meeting");
    pool
}

/// Write a completed summary the way the fixture seeder does: straight into
/// `summary_processes`, with nothing in `transcript_chunks`.
async fn save_summary(pool: &SqlitePool, markdown: &str) {
    SummaryProcessesRepository::create_or_reset_process(pool, "m1")
        .await
        .expect("create process");
    let result = serde_json::json!({
        "markdown": markdown,
        "summary_status": { "complete": true, "total_chunks": 1, "processed_chunks": 1, "failed_chunks": 0 }
    });
    SummaryProcessesRepository::update_process_completed(
        pool,
        "m1",
        result,
        1,
        0.0,
        false,
        "fingerprint",
        "",
    )
    .await
    .expect("complete process");
}

#[tokio::test]
async fn a_summary_saved_without_transcript_chunks_is_still_readable() {
    let pool = test_pool().await;
    save_summary(&pool, "## Key decisions\n\n- Ship it").await;

    let chunks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transcript_chunks")
        .fetch_one(&pool)
        .await
        .expect("count chunks");
    assert_eq!(chunks, 0, "the point of this test is that there are none");

    let process = SummaryProcessesRepository::get_summary_data_for_meeting(&pool, "m1")
        .await
        .expect("query summary")
        .expect("a saved summary must be readable without a transcript_chunks row");

    assert_eq!(process.status.to_lowercase(), "completed");
    let result = process
        .result
        .expect("completed process carries its result");
    assert!(
        result.contains("Key decisions"),
        "the stored markdown should come back intact, got: {result}"
    );
}

#[tokio::test]
async fn a_meeting_with_no_summary_still_reads_as_none() {
    let pool = test_pool().await;
    let process = SummaryProcessesRepository::get_summary_data_for_meeting(&pool, "m1")
        .await
        .expect("query summary");
    assert!(
        process.is_none(),
        "dropping the join must not invent a summary for a meeting that has none"
    );
}
