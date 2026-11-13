//! Topic persistence: CRUD, the single notes writer, and delete cascades (specs/0089).

use anyhow::{anyhow, Result};
use chrono::Utc;
use sqlx::{SqliteConnection, SqlitePool};
use tauri::{AppHandle, Emitter, Runtime};

use super::model::{Topic, TopicSummary};

type SqlxError = sqlx::Error;

/// Rust -> frontend event after any topic write. Payload `{ "topic_id": string | null }`
/// (`null` = several topics changed).
pub const TOPICS_UPDATED_EVENT: &str = "topics-updated";

pub fn emit_updated<R: Runtime>(app: &AppHandle<R>, topic_id: Option<&str>) {
    let _ = app.emit(
        TOPICS_UPDATED_EVENT,
        serde_json::json!({ "topic_id": topic_id }),
    );
}

const SELECT_TOPIC: &str = "SELECT id, name, description, status, notes_markdown, notes_json, \
                            created_at, updated_at, closed_at FROM topics";

pub struct TopicsRepository;

impl TopicsRepository {
    pub async fn create(pool: &SqlitePool, name: &str, description: &str) -> Result<Topic> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("Give the topic a name."));
        }
        let id = format!("topic-{}", uuid::Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO topics (id, name, description, status, created_at, updated_at)
             VALUES (?, ?, ?, 'open', ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(description.trim())
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
        Self::get(pool, &id)
            .await?
            .ok_or_else(|| anyhow!("Topic vanished right after it was created."))
    }

    pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Topic>> {
        Ok(
            sqlx::query_as::<_, Topic>(&format!("{SELECT_TOPIC} WHERE id = ?"))
                .bind(id)
                .fetch_optional(pool)
                .await?,
        )
    }

    /// Open topics first, then by name. `status` filters when given.
    pub async fn list(pool: &SqlitePool, status: Option<&str>) -> Result<Vec<TopicSummary>> {
        Ok(sqlx::query_as::<_, TopicSummary>(
            "SELECT t.id, t.name, t.description, t.status,
                    (SELECT MAX(e.occurred_at) FROM topic_entries e WHERE e.topic_id = t.id)
                        AS last_activity,
                    (SELECT COUNT(*) FROM topic_suggestions s
                      WHERE s.topic_id = t.id AND s.status = 'pending') AS pending_count,
                    (SELECT COUNT(DISTINCT e.meeting_id) FROM topic_entries e
                      WHERE e.topic_id = t.id AND e.meeting_id IS NOT NULL) AS meeting_count
             FROM topics t
             WHERE (?1 IS NULL OR t.status = ?1)
             ORDER BY CASE t.status WHEN 'open' THEN 0 ELSE 1 END, t.name COLLATE NOCASE, t.id",
        )
        .bind(status)
        .fetch_all(pool)
        .await?)
    }

    pub async fn update(
        pool: &SqlitePool,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        status: Option<&str>,
    ) -> Result<Option<Topic>> {
        if let Some(n) = name {
            if n.trim().is_empty() {
                return Err(anyhow!("Give the topic a name."));
            }
        }
        if let Some(s) = status {
            if s != "open" && s != "closed" {
                return Err(anyhow!("A topic is either open or closed."));
            }
        }
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE topics SET
                 name = COALESCE(?2, name),
                 description = COALESCE(?3, description),
                 status = COALESCE(?4, status),
                 closed_at = CASE WHEN ?4 = 'closed' THEN ?5
                                  WHEN ?4 = 'open' THEN NULL ELSE closed_at END,
                 updated_at = ?5
             WHERE id = ?1",
        )
        .bind(id)
        .bind(name.map(str::trim))
        .bind(description.map(str::trim))
        .bind(status)
        .bind(&now)
        .execute(pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Self::get(pool, id).await
    }

    /// The only writer of `notes_markdown` / `notes_json` ("My notes").
    pub async fn set_notes(
        pool: &SqlitePool,
        id: &str,
        markdown: &str,
        json: Option<&str>,
    ) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE topics SET notes_markdown = ?2, notes_json = ?3, updated_at = ?4 WHERE id = ?1",
        )
        .bind(id)
        .bind(markdown)
        .bind(json)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Delete a topic and everything under it; tagged to-dos are kept, untagged.
    pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool> {
        let mut tx = pool.begin().await?;
        sqlx::query(
            "DELETE FROM topic_revisions WHERE entry_id IN
                 (SELECT id FROM topic_entries WHERE topic_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        for table in ["topic_entries", "topic_suggestions", "topic_links"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE topic_id = ?"))
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE action_items SET topic_id = NULL WHERE topic_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let res = sqlx::query("DELETE FROM topics WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }
}

/// Meeting-delete hook. Entries and revisions are kept (marked); pending suggestions,
/// meeting links and the scan ledger row go.
pub async fn cascade_meeting_delete(
    tx: &mut SqliteConnection,
    meeting_id: &str,
) -> Result<(), SqlxError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE topic_entries SET source_deleted_at = ?2
         WHERE meeting_id = ?1 AND source_deleted_at IS NULL",
    )
    .bind(meeting_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE topic_revisions SET source_deleted_at = ?2
         WHERE source_meeting_id = ?1 AND source_deleted_at IS NULL",
    )
    .bind(meeting_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM topic_suggestions WHERE meeting_id = ? AND status = 'pending'")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM topic_links WHERE kind = 'meeting' AND ref = ?")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM topic_mention_scans WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}
