//! Startup transcript recovery's defence in depth (specs/0083 W2b).
//!
//! The webview keeps an IndexedDB copy of the live transcript so a crash mid-recording can be
//! recovered. Rust now saves the meeting itself, possibly while the webview is asleep, so the
//! webview may never hear `saved` and never mark its copy. The next launch would then offer to
//! "recover" a meeting that is already in the database. The recovery check asks this first.

use tauri::{AppHandle, Runtime};

use crate::state::AppState;

/// True when a meeting recorded into `folder_path` already has transcript rows saved.
#[tauri::command]
pub async fn api_folder_has_saved_transcript<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    folder_path: String,
) -> Result<bool, String> {
    folder_has_saved_transcript(state.db_manager.pool(), &folder_path)
        .await
        .map_err(|e| format!("Couldn't check the saved meeting for this recording: {e}"))
}

pub async fn folder_has_saved_transcript(
    pool: &sqlx::SqlitePool,
    folder_path: &str,
) -> Result<bool, sqlx::Error> {
    let folder = folder_path.trim();
    if folder.is_empty() {
        return Ok(false);
    }
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM meetings m JOIN transcripts t ON t.meeting_id = m.id
             WHERE m.folder_path = ?
         )",
    )
    .bind(folder)
    .fetch_one(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn meeting(pool: &SqlitePool, id: &str, folder: &str) {
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) \
             VALUES (?, 'M', '2026-11-05T10:00:00Z', '2026-11-05T10:00:00Z', ?)",
        )
        .bind(id)
        .bind(folder)
        .execute(pool)
        .await
        .expect("insert meeting");
    }

    async fn segment(pool: &SqlitePool, meeting_id: &str) {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp) \
             VALUES ('t1', ?, 'hello', '2026-11-05T10:00:01Z')",
        )
        .bind(meeting_id)
        .execute(pool)
        .await
        .expect("insert transcript");
    }

    #[tokio::test]
    async fn a_folder_whose_meeting_has_rows_is_saved() {
        let pool = test_pool().await;
        meeting(&pool, "m1", "/rec/a").await;
        segment(&pool, "m1").await;
        assert!(folder_has_saved_transcript(&pool, "/rec/a").await.unwrap());
    }

    #[tokio::test]
    async fn a_meeting_row_without_transcript_rows_is_not_saved() {
        let pool = test_pool().await;
        meeting(&pool, "m1", "/rec/a").await;
        assert!(!folder_has_saved_transcript(&pool, "/rec/a").await.unwrap());
    }

    #[tokio::test]
    async fn another_folder_or_an_empty_path_is_not_saved() {
        let pool = test_pool().await;
        meeting(&pool, "m1", "/rec/a").await;
        segment(&pool, "m1").await;
        assert!(!folder_has_saved_transcript(&pool, "/rec/b").await.unwrap());
        assert!(!folder_has_saved_transcript(&pool, "  ").await.unwrap());
    }
}
