//! `saved_questions` table access (specs/0038 WS2.b) — reusable "starred" Ask-AI questions.
//!
//! A saved question is a label + question text + the scope it should run over. The frontend
//! re-runs one by re-invoking the existing `api_ask_ai_run` with the stored `(question,
//! scope_json)`, so it re-gathers and re-answers against whatever data now exists. `last_run_at`
//! (bumped by [`touch_last_run`](SavedQuestionsRepository::touch_last_run)) is the seam a future
//! scheduled/digest feature would build on.
//!
//! Mirrors the directory's house pattern (`action_item.rs`, `meeting_brief.rs`): a stateless
//! struct whose methods take `&SqlitePool` and return `SqlxError`; the command layer maps to
//! user-facing strings.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, FromRow, SqlitePool};
use uuid::Uuid;

/// One saved question, serialized camelCase for the frontend.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuestion {
    /// `"saq-<uuid>"`.
    pub id: String,
    pub label: String,
    pub question: String,
    /// Serialized `AggregationScope` (camelCase) — re-run round-trips it verbatim.
    pub scope_json: String,
    pub created_at: String,
    pub updated_at: String,
    /// ISO-8601 of the last run; `None` until first run. Stamped by `update_answer` when a run
    /// completes (previously only the `touch_last_run` seam).
    pub last_run_at: Option<String>,
    /// Cached answer markdown from the last completed run; `None` until first run (specs/0038 #4).
    pub last_answer_markdown: Option<String>,
    /// Cached JSON array of `SourceMeeting` for the last answer; `None` until first run.
    pub last_sources_json: Option<String>,
}

const SELECT: &str = "SELECT id, label, question, scope_json, created_at, updated_at, \
     last_run_at, last_answer_markdown, last_sources_json FROM saved_questions";

pub struct SavedQuestionsRepository;

impl SavedQuestionsRepository {
    /// Star the current question + scope. Returns the created row.
    pub async fn create(
        pool: &SqlitePool,
        label: &str,
        question: &str,
        scope_json: &str,
    ) -> Result<SavedQuestion, SqlxError> {
        let id = format!("saq-{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO saved_questions \
               (id, label, question, scope_json, created_at, updated_at, last_run_at) \
             VALUES (?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(&id)
        .bind(label)
        .bind(question)
        .bind(scope_json)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        Self::get(pool, &id).await?.ok_or(SqlxError::RowNotFound)
    }

    /// All saved questions, most-recently-created first.
    pub async fn list(pool: &SqlitePool) -> Result<Vec<SavedQuestion>, SqlxError> {
        sqlx::query_as::<_, SavedQuestion>(&format!("{SELECT} ORDER BY created_at DESC, id DESC"))
            .fetch_all(pool)
            .await
    }

    /// One saved question by id.
    pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<SavedQuestion>, SqlxError> {
        sqlx::query_as::<_, SavedQuestion>(&format!("{SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    /// Hard delete. Returns whether a row was removed.
    pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool, SqlxError> {
        let res = sqlx::query("DELETE FROM saved_questions WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Stamps `last_run_at` (and `updated_at`) with now — called when a saved question is
    /// re-run. Returns whether the row still existed. WS2.b re-run itself re-invokes the
    /// existing `api_ask_ai_run`; this is the optional bookkeeping hook.
    pub async fn touch_last_run(pool: &SqlitePool, id: &str) -> Result<bool, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res =
            sqlx::query("UPDATE saved_questions SET last_run_at = ?, updated_at = ? WHERE id = ?")
                .bind(&now)
                .bind(&now)
                .bind(id)
                .execute(pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Caches the answer from a completed run (specs/0038 #4): stores the answer markdown +
    /// sources JSON and stamps `last_run_at` (and `updated_at`) with now. The sub-page renders
    /// this cache on open — it does NOT re-answer until the user presses Rerun, which calls this.
    /// Returns whether the row still existed.
    pub async fn update_answer(
        pool: &SqlitePool,
        id: &str,
        answer_markdown: &str,
        sources_json: &str,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE saved_questions \
               SET last_answer_markdown = ?, last_sources_json = ?, last_run_at = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(answer_markdown)
        .bind(sources_json)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

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

    #[tokio::test]
    async fn create_list_get_touch_delete_roundtrip() {
        let pool = test_pool().await;

        let q = SavedQuestionsRepository::create(
            &pool,
            "Decisions today",
            "What were the key decisions today?",
            r#"{"dateFrom":"2026-07-07T00:00:00Z"}"#,
        )
        .await
        .unwrap();
        assert!(q.id.starts_with("saq-"));
        assert_eq!(q.label, "Decisions today");
        assert!(q.last_run_at.is_none(), "never run yet");

        let list = SavedQuestionsRepository::list(&pool).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].scope_json, r#"{"dateFrom":"2026-07-07T00:00:00Z"}"#);

        // touch_last_run stamps last_run_at.
        assert!(SavedQuestionsRepository::touch_last_run(&pool, &q.id)
            .await
            .unwrap());
        let after = SavedQuestionsRepository::get(&pool, &q.id)
            .await
            .unwrap()
            .expect("exists");
        assert!(after.last_run_at.is_some(), "re-run stamps last_run_at");

        // touch on a missing id is a no-op (returns false).
        assert!(
            !SavedQuestionsRepository::touch_last_run(&pool, "saq-missing")
                .await
                .unwrap()
        );

        // update_answer caches the last answer markdown + sources and stamps last_run_at.
        assert!(SavedQuestionsRepository::update_answer(
            &pool,
            &q.id,
            "The key decision was to ship.",
            r#"[{"meetingId":"m1","title":"Standup","createdAt":"2026-07-07T00:00:00Z","cited":true}]"#,
        )
        .await
        .unwrap());
        let cached = SavedQuestionsRepository::get(&pool, &q.id)
            .await
            .unwrap()
            .expect("exists");
        assert_eq!(
            cached.last_answer_markdown.as_deref(),
            Some("The key decision was to ship.")
        );
        assert!(cached.last_sources_json.is_some(), "sources cached");
        assert!(cached.last_run_at.is_some(), "run stamps last_run_at");

        // update_answer on a missing id is a no-op (returns false).
        assert!(
            !SavedQuestionsRepository::update_answer(&pool, "saq-missing", "x", "[]")
                .await
                .unwrap()
        );

        // delete removes exactly one; second delete is a no-op.
        assert!(SavedQuestionsRepository::delete(&pool, &q.id)
            .await
            .unwrap());
        assert!(!SavedQuestionsRepository::delete(&pool, &q.id)
            .await
            .unwrap());
        assert!(SavedQuestionsRepository::list(&pool)
            .await
            .unwrap()
            .is_empty());
    }
}
