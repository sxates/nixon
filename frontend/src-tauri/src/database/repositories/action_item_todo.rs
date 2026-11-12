//! To-do list / suggestion inbox queries and transitions (specs/0088).
//!
//! Kept apart from `action_item.rs` (file-size budget). "Latest source meeting" means the
//! newest meeting among the item's `action_item_sources` rows plus its own `meeting_id`:
//! the union keeps items that predate source-row writing ordered correctly.

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use super::action_item::{ActionItem, ActionItemsRepository};

type SqlxError = sqlx::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoView {
    List,
    Inbox,
    Done,
    Dismissed,
}

impl TodoView {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "list" => Some(Self::List),
            "inbox" => Some(Self::Inbox),
            "done" => Some(Self::Done),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoRow {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub item: ActionItem,
    pub meeting_title: Option<String>,
    pub meeting_created_at: Option<String>,
    pub source_count: i64,
    pub latest_source_at: Option<String>,
    pub latest_context: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRow {
    pub meeting_id: String,
    pub meeting_title: Option<String>,
    pub meeting_created_at: Option<String>,
    pub context: Option<String>,
    pub phrasing: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemDetail {
    pub item: ActionItem,
    pub sources: Vec<SourceRow>,
    pub person_ids: Vec<String>,
}

/// Meeting ids an item was mentioned in (sources plus its own meeting).
const SOURCE_IDS: &str = "SELECT s.meeting_id FROM action_item_sources s \
     WHERE s.action_item_id = ai.id UNION SELECT ai.meeting_id";

const COLS: &str = "ai.id, ai.meeting_id, ai.description, ai.assignee_person_id, \
     ai.assignee_is_self, ai.assignee_raw, ai.due_hint, ai.due_date, ai.status, ai.source, \
     ai.user_edited, ai.content_key, ai.created_at, ai.updated_at, ai.completed_at, \
     ai.sort_order, ai.triage, ai.adopted_at, ai.notes_markdown";

fn latest_join() -> String {
    format!(
        "LEFT JOIN meetings lm ON lm.id = (SELECT m2.id FROM meetings m2 \
         WHERE m2.id IN ({SOURCE_IDS}) ORDER BY m2.created_at DESC, m2.id LIMIT 1)"
    )
}

pub struct ActionItemTodoRepository;

impl ActionItemTodoRepository {
    pub async fn list_view(pool: &SqlitePool, view: TodoView) -> Result<Vec<TodoRow>, SqlxError> {
        let (filter, order) = match view {
            TodoView::List => (
                "ai.triage = 'listed' AND ai.status = 'open'",
                "ai.sort_order IS NULL, ai.sort_order ASC, ai.created_at ASC",
            ),
            TodoView::Inbox => (
                "ai.triage = 'suggested' AND ai.status = 'open'",
                "lm.created_at DESC, ai.created_at DESC",
            ),
            TodoView::Done => (
                "ai.triage = 'listed' AND ai.status = 'completed'",
                "ai.completed_at DESC",
            ),
            TodoView::Dismissed => (
                "ai.status = 'dismissed' AND ai.triage != 'record'",
                "ai.updated_at DESC",
            ),
        };
        let sql = format!(
            "SELECT {COLS}, m.title AS meeting_title, m.created_at AS meeting_created_at, \
             (SELECT COUNT(*) FROM ({SOURCE_IDS}) WHERE meeting_id IS NOT NULL) AS source_count, \
             lm.created_at AS latest_source_at, \
             (SELECT s.context FROM action_item_sources s \
               WHERE s.action_item_id = ai.id AND s.meeting_id = lm.id) AS latest_context \
             FROM action_items ai \
             LEFT JOIN meetings m ON m.id = ai.meeting_id {} \
             WHERE {filter} ORDER BY {order}",
            latest_join()
        );
        sqlx::query_as::<_, TodoRow>(&sql).fetch_all(pool).await
    }

    pub async fn inbox_count(pool: &SqlitePool) -> Result<i64, SqlxError> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM action_items WHERE triage = 'suggested' AND status = 'open'",
        )
        .fetch_one(pool)
        .await
    }

    /// "Add to my list" / "Adopt": the assignee is left alone.
    pub async fn add_to_list(pool: &SqlitePool, id: &str) -> Result<Option<ActionItem>, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE action_items SET triage = 'listed', adopted_at = ?, user_edited = 1, \
             status = 'open', completed_at = NULL, updated_at = ?, \
             sort_order = (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM action_items \
                           WHERE triage = 'listed' AND id != ?) \
             WHERE id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(id)
        .bind(id)
        .execute(pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        ActionItemsRepository::get(pool, id).await
    }

    pub async fn restore(pool: &SqlitePool, id: &str) -> Result<Option<ActionItem>, SqlxError> {
        let res = sqlx::query(
            "UPDATE action_items SET status = 'open', completed_at = NULL, user_edited = 1, \
             updated_at = ? WHERE id = ?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        ActionItemsRepository::get(pool, id).await
    }

    /// A suggestion that now belongs to someone else is part of the record, not the inbox.
    pub async fn reconcile_triage_after_assignee_change(
        pool: &SqlitePool,
        id: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE action_items SET triage = 'record' WHERE id = ? AND triage = 'suggested' \
             AND assignee_is_self = 0 \
             AND (assignee_person_id IS NOT NULL OR assignee_raw IS NOT NULL)",
        )
        .bind(id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn set_notes(
        pool: &SqlitePool,
        id: &str,
        markdown: Option<&str>,
    ) -> Result<bool, SqlxError> {
        let markdown = markdown.map(str::trim).filter(|m| !m.is_empty());
        let res =
            sqlx::query("UPDATE action_items SET notes_markdown = ?, updated_at = ? WHERE id = ?")
                .bind(markdown)
                .bind(Utc::now().to_rfc3339())
                .bind(id)
                .execute(pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_detail(
        pool: &SqlitePool,
        id: &str,
    ) -> Result<Option<ActionItemDetail>, SqlxError> {
        let Some(item) = ActionItemsRepository::get(pool, id).await? else {
            return Ok(None);
        };
        let sources = sqlx::query_as::<_, SourceRow>(
            "SELECT s.meeting_id, m.title AS meeting_title, m.created_at AS meeting_created_at, \
             s.context, s.phrasing FROM action_item_sources s \
             LEFT JOIN meetings m ON m.id = s.meeting_id WHERE s.action_item_id = ? \
             ORDER BY m.created_at ASC, s.meeting_id ASC",
        )
        .bind(id)
        .fetch_all(pool)
        .await?;
        let person_ids = sqlx::query_scalar::<_, String>(
            "SELECT person_id FROM action_item_people WHERE action_item_id = ? ORDER BY person_id",
        )
        .bind(id)
        .fetch_all(pool)
        .await?;
        Ok(Some(ActionItemDetail {
            item,
            sources,
            person_ids,
        }))
    }

    /// Dismiss open suggestions whose newest source meeting is older than `days`.
    pub async fn dismiss_suggestions_older_than(
        pool: &SqlitePool,
        days: i64,
    ) -> Result<Vec<String>, SqlxError> {
        let cutoff = (Utc::now() - Duration::days(days)).to_rfc3339();
        let now = Utc::now().to_rfc3339();
        sqlx::query_scalar::<_, String>(&format!(
            "UPDATE action_items SET status = 'dismissed', completed_at = NULL, \
             user_edited = 1, updated_at = ? WHERE id IN (\
               SELECT ai.id FROM action_items ai {} \
               WHERE ai.triage = 'suggested' AND ai.status = 'open' \
                 AND lm.created_at IS NOT NULL AND julianday(lm.created_at) < julianday(?)) \
             RETURNING id",
            latest_join()
        ))
        .bind(&now)
        .bind(&cutoff)
        .fetch_all(pool)
        .await
    }

    /// Items shown on a meeting page: born there, or mentioned there.
    pub async fn list_for_meeting_page(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<ActionItem>, SqlxError> {
        sqlx::query_as::<_, ActionItem>(
            "SELECT id, meeting_id, description, assignee_person_id, assignee_is_self, \
             assignee_raw, due_hint, due_date, status, source, user_edited, content_key, \
             created_at, updated_at, completed_at, sort_order, triage, adopted_at, \
             notes_markdown FROM action_items WHERE meeting_id = ?1 OR EXISTS (\
               SELECT 1 FROM action_item_sources s WHERE s.action_item_id = action_items.id \
               AND s.meeting_id = ?1) ORDER BY created_at ASC, id ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }
}
