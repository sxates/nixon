//! specs/0085 acceptance 6: a meeting summarized with Claude Code while signed out ends
//! with the summary process failed and the sign-in copy, no summary written, and a queue
//! row that offers Retry. With the fake switched to success, that Retry writes a summary.
//!
//! Real DB (migrated), `tauri::test::mock_app`, the production summary entry point
//! (`start_summary_generation_for_meeting`, which the queue's Retry also calls) and the
//! queue's own `retry_task`. The CLI is the fake, through the wrapper in `common_cli`.
//!
//! Run with:
//!   cargo test --features metal --test cli_summary_flow

mod common_cli;

use app_lib::database::manager::DatabaseManager;
use app_lib::database::models::SummaryProcess;
use app_lib::database::repositories::setting::SettingsRepository;
use app_lib::database::repositories::summary::SummaryProcessesRepository;
use app_lib::llm_activity::registry::TaskOutcome;
use app_lib::llm_activity::retry::{is_retryable, retry_task};
use app_lib::llm_activity::{LlmActivityState, LlmTaskRegistry, TaskKind};
use app_lib::state::AppState;
use app_lib::summary::commands::start_summary_generation_for_meeting;
use common_cli::FakeClaude;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;

const SIGN_IN_COPY: &str = "Nixon couldn't reach Claude with your account. Make sure you're signed in to Claude Code on this Mac, then press Retry.";

async fn demo_meeting(pool: &SqlitePool) -> String {
    let id = "meeting-cli-flow-demo".to_string();
    let at = "2026-11-06T10:00:00Z";
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, origin) \
         VALUES (?, 'Fictional demo planning sync', ?, ?, 'recorded')",
    )
    .bind(&id)
    .bind(at)
    .bind(at)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, ?, ?)",
    )
    .bind("t-cli-flow-1")
    .bind(&id)
    .bind("We agreed to ship the launch checklist on Friday and review the demo next week.")
    .bind(at)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// Polls the process row until it leaves `processing`, or panics after 30 s.
async fn settled(pool: &SqlitePool, meeting_id: &str, want: &str) -> SummaryProcess {
    let start = Instant::now();
    loop {
        let row = SummaryProcessesRepository::get_summary_data(pool, meeting_id)
            .await
            .unwrap();
        if let Some(row) = row {
            if row.status == want {
                return row;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "summary never reached {want}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn signed_out_summary_fails_with_the_copy_and_retry_succeeds() {
    let fake = FakeClaude::install().await;
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new(
        dir.path().join("t.sqlite").to_str().unwrap(),
        dir.path().join("none.db").to_str().unwrap(),
    )
    .await
    .unwrap();
    let pool = db.pool().clone();
    let registry = Arc::new(LlmTaskRegistry::new());
    let app = tauri::test::mock_app();
    app.manage(AppState { db_manager: db });
    app.manage(LlmActivityState(Arc::clone(&registry)));
    SettingsRepository::save_model_config(&pool, "claude-code", "sonnet", "large-v3", None)
        .await
        .unwrap();
    let meeting = demo_meeting(&pool).await;

    // Signed out: the process fails with the sign-in copy and nothing is written.
    fake.set_mode("not-logged-in");
    start_summary_generation_for_meeting(app.handle().clone(), pool.clone(), meeting.clone())
        .await
        .unwrap();
    let failed = settled(&pool, &meeting, "failed").await;
    assert_eq!(failed.error.as_deref(), Some(SIGN_IN_COPY));
    assert_eq!(failed.result, None, "no summary row on failure");
    assert_eq!(fake.calls(), 1, "terminal: one attempt");

    // The queue row carries the same copy and offers Retry.
    let start = Instant::now();
    let record = loop {
        let found = registry
            .view()
            .history
            .into_iter()
            .find(|r| r.kind == TaskKind::MeetingSummary);
        if let Some(r) = found {
            break r;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "no queue row");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    match &record.outcome {
        TaskOutcome::Failed { error } => assert_eq!(error, SIGN_IN_COPY),
        other => panic!("queue row is not a failure: {other:?}"),
    }
    assert!(is_retryable(record.kind), "the row offers Retry");

    // Signed back in: the queue's Retry produces a summary.
    fake.set_mode("success");
    retry_task(app.handle(), record.id).await.unwrap();
    let done = settled(&pool, &meeting, "completed").await;
    assert_eq!(done.error, None);
    let result = done.result.expect("a summary is written");
    assert!(result.contains("Exponential backoff"), "the fake's summary");
}
