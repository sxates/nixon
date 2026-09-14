//! `ask_ai_history` table access (specs/0038 WS2.a) — one row per completed Ask-AI run.
//!
//! A thin persistence layer over the 0035 Ask-AI engine: `execute_ask_ai` stays ephemeral,
//! and `aggregation::commands::api_ask_ai_run` records each success here fire-and-forget (the
//! write must never block or fail the run). The `/ask` page lists these most-recent-first and
//! can reload a prior Q&A read-only; the stored `scope_json` lets a re-run round-trip the exact
//! scope.
//!
//! Mirrors the directory's house pattern (`action_item.rs`, `meeting_brief.rs`): a stateless
//! struct whose methods take `&SqlitePool` and return `SqlxError`; the command layer maps to
//! user-facing strings.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, FromRow, SqlitePool};
use uuid::Uuid;

/// One persisted Ask-AI run, serialized camelCase for the frontend.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiHistoryEntry {
    /// `"aah-<uuid>"`.
    pub id: String,
    pub question: String,
    /// Serialized `AggregationScope` (camelCase) — re-run round-trips it verbatim.
    pub scope_json: String,
    pub answer_markdown: String,
    /// JSON array of `SourceMeeting`.
    pub sources_json: String,
    /// The summary provider/model the run followed (`None` when unknown).
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
}

const SELECT: &str = "SELECT id, question, scope_json, answer_markdown, sources_json, \
     provider, model, created_at FROM ask_ai_history";

/// Default page size for [`AskAiHistoryRepository::list`] when no explicit limit is given.
const DEFAULT_LIST_LIMIT: u32 = 50;

pub struct AskAiHistoryRepository;

impl AskAiHistoryRepository {
    /// Records a completed run. Called fire-and-forget from the Ask-AI success arm, so it
    /// returns the inserted row for symmetry with the directory's `create` methods but the
    /// caller typically ignores it.
    pub async fn insert(
        pool: &SqlitePool,
        question: &str,
        scope_json: &str,
        answer_markdown: &str,
        sources_json: &str,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> Result<AskAiHistoryEntry, SqlxError> {
        let id = format!("aah-{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO ask_ai_history \
               (id, question, scope_json, answer_markdown, sources_json, provider, model, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(question)
        .bind(scope_json)
        .bind(answer_markdown)
        .bind(sources_json)
        .bind(provider)
        .bind(model)
        .bind(&now)
        .execute(pool)
        .await?;

        Self::get(pool, &id).await?.ok_or(SqlxError::RowNotFound)
    }

    /// Most-recent-first history. `limit` caps the page (defaults to
    /// [`DEFAULT_LIST_LIMIT`]); the `created_at DESC` index backs the ordering.
    pub async fn list(
        pool: &SqlitePool,
        limit: Option<u32>,
    ) -> Result<Vec<AskAiHistoryEntry>, SqlxError> {
        let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT);
        sqlx::query_as::<_, AskAiHistoryEntry>(&format!(
            "{SELECT} ORDER BY created_at DESC, id DESC LIMIT ?"
        ))
        .bind(limit)
        .fetch_all(pool)
        .await
    }

    /// One entry by id (the read-only reload of a prior Q&A).
    pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<AskAiHistoryEntry>, SqlxError> {
        sqlx::query_as::<_, AskAiHistoryEntry>(&format!("{SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    /// Hard delete. Returns whether a row was removed.
    pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool, SqlxError> {
        let res = sqlx::query("DELETE FROM ask_ai_history WHERE id = ?")
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

    /// Fresh in-memory SQLite brought up through the app's real migration set (mirrors
    /// `action_item.rs` tests). One connection max — each in-memory connection is a separate db.
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
    async fn insert_list_get_delete_roundtrip_newest_first() {
        let pool = test_pool().await;

        let a = AskAiHistoryRepository::insert(
            &pool,
            "What did we decide?",
            "{}",
            "We decided X [M1]",
            "[]",
            Some("ollama"),
            Some("llama3.2:latest"),
        )
        .await
        .unwrap();
        assert!(a.id.starts_with("aah-"));
        assert_eq!(a.provider.as_deref(), Some("ollama"));

        // A later row sorts first (created_at DESC). rfc3339 has sub-second precision, and even
        // at equal timestamps the id tie-breaker keeps ordering deterministic.
        let b = AskAiHistoryRepository::insert(
            &pool,
            "Any open risks?",
            r#"{"personId":"p1"}"#,
            "Risk R [M2]",
            r#"[{"meetingId":"m1"}]"#,
            None,
            None,
        )
        .await
        .unwrap();

        let list = AskAiHistoryRepository::list(&pool, None).await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|e| e.id == a.id));
        assert!(list.iter().any(|e| e.id == b.id));

        // get round-trips the stored scope/sources JSON verbatim.
        let got = AskAiHistoryRepository::get(&pool, &b.id)
            .await
            .unwrap()
            .expect("entry exists");
        assert_eq!(got.scope_json, r#"{"personId":"p1"}"#);
        assert_eq!(got.sources_json, r#"[{"meetingId":"m1"}]"#);
        assert!(got.provider.is_none());

        // limit caps the page.
        let one = AskAiHistoryRepository::list(&pool, Some(1)).await.unwrap();
        assert_eq!(one.len(), 1);

        // delete removes exactly one; second delete is a no-op.
        assert!(AskAiHistoryRepository::delete(&pool, &a.id).await.unwrap());
        assert!(!AskAiHistoryRepository::delete(&pool, &a.id).await.unwrap());
        assert!(AskAiHistoryRepository::get(&pool, &a.id)
            .await
            .unwrap()
            .is_none());
    }
}
