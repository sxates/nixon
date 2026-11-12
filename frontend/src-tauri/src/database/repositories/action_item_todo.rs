//! To-do list / suggestion inbox queries and transitions (specs/0088).
//!
//! Kept apart from `action_item.rs` (file-size budget). "Latest source meeting" means the
//! newest meeting among the item's `action_item_sources` rows plus its own `meeting_id`:
//! the union keeps items that predate source-row writing ordered correctly.

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::action_items::extractor::ALREADY_TRACKED_CAP;
use crate::action_items::mentions::{person_ids_in, plain_description};
use crate::action_items::merge::{SourceAttach, TrackedItem};
use crate::action_items::triage::ABSORB_COMPLETED_WITHIN_DAYS;

use super::action_item::{ActionItem, ActionItemsRepository, SELECT_ITEM};

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
    /// Prep carryover (specs/0088): YOUR listed items open in the series (any source meeting
    /// in `prior`), then only the PREVIOUS occurrence's (`prior[0]`, newest-first) open
    /// `record` items. Mine-first within the result; empty `prior` -> empty.
    pub async fn prep_items(
        pool: &SqlitePool,
        prior: &[String],
    ) -> Result<Vec<ActionItem>, SqlxError> {
        let Some(previous) = prior.first() else {
            return Ok(Vec::new());
        };
        let ph = std::iter::repeat_n("?", prior.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "{SELECT_ITEM} WHERE status = 'open' AND ( \
               (triage = 'listed' AND (meeting_id IN ({ph}) OR id IN ( \
                  SELECT action_item_id FROM action_item_sources WHERE meeting_id IN ({ph})))) \
               OR (triage = 'record' AND meeting_id = ?)) \
             ORDER BY (triage = 'listed') DESC, assignee_is_self DESC, created_at ASC, id ASC"
        );
        let mut q = sqlx::query_as::<_, ActionItem>(&sql);
        for id in prior.iter().chain(prior.iter()) {
            q = q.bind(id);
        }
        q.bind(previous).fetch_all(pool).await
    }

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

    /// "Adopt": take someone else's item onto the list, assignee left alone.
    pub async fn adopt(pool: &SqlitePool, id: &str) -> Result<Option<ActionItem>, SqlxError> {
        Self::list_item(pool, id, false).await
    }

    /// "Add to my list": the item becomes mine (a suggestion is mine or unassigned).
    pub async fn add_to_list(pool: &SqlitePool, id: &str) -> Result<Option<ActionItem>, SqlxError> {
        Self::list_item(pool, id, true).await
    }

    async fn list_item(
        pool: &SqlitePool,
        id: &str,
        make_mine: bool,
    ) -> Result<Option<ActionItem>, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE action_items SET triage = 'listed', adopted_at = ?2, user_edited = 1, \
             status = 'open', completed_at = NULL, updated_at = ?2, \
             assignee_is_self = CASE WHEN ?3 THEN 1 ELSE assignee_is_self END, \
             assignee_person_id = CASE WHEN ?3 THEN NULL ELSE assignee_person_id END, \
             assignee_raw = CASE WHEN ?3 THEN NULL ELSE assignee_raw END, \
             sort_order = (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM action_items \
                           WHERE triage = 'listed' AND id != ?1) \
             WHERE id = ?1",
        )
        .bind(id)
        .bind(&now)
        .bind(make_mine)
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

    /// Keeps triage in step with the assignee: a suggestion that now belongs to someone else is
    /// part of the record; an open record item that is mine or nobody's is a suggestion again.
    pub async fn reconcile_triage_after_assignee_change(
        pool: &SqlitePool,
        id: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE action_items SET triage = CASE \
               WHEN assignee_is_self = 0 AND (assignee_person_id IS NOT NULL \
                    OR assignee_raw IS NOT NULL) THEN 'record' ELSE 'suggested' END \
             WHERE id = ?1 AND ( \
               (triage = 'suggested' AND assignee_is_self = 0 \
                AND (assignee_person_id IS NOT NULL OR assignee_raw IS NOT NULL)) \
               OR (triage = 'record' AND status = 'open' AND (assignee_is_self = 1 \
                OR (assignee_person_id IS NULL AND assignee_raw IS NULL))))",
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

    /// The TRACKED list for cross-meeting merging (specs/0088), in prompt order: listed+open,
    /// suggested+open, dismissed (not `record`), then completed within
    /// [`ABSORB_COMPLETED_WITHIN_DAYS`]; each group newest-updated first, capped at
    /// [`ALREADY_TRACKED_CAP`]. Descriptions go through `plain_description`, so mention
    /// tokens never reach the model. The caller must route against this same order.
    pub async fn tracked_for_merge(pool: &SqlitePool) -> Result<Vec<TrackedItem>, SqlxError> {
        let cutoff = (Utc::now() - Duration::days(ABSORB_COMPLETED_WITHIN_DAYS)).to_rfc3339();
        let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, meeting_id, description FROM action_items WHERE \
               (triage IN ('listed', 'suggested') AND status = 'open') \
               OR (status = 'dismissed' AND triage != 'record') \
               OR (status = 'completed' AND triage != 'record' AND completed_at IS NOT NULL \
                   AND julianday(completed_at) >= julianday(?)) \
             ORDER BY CASE WHEN status = 'open' AND triage = 'listed' THEN 0 \
                           WHEN status = 'open' THEN 1 \
                           WHEN status = 'dismissed' THEN 2 ELSE 3 END, \
                      updated_at DESC, id LIMIT ?",
        )
        .bind(&cutoff)
        .bind(ALREADY_TRACKED_CAP as i64)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, meeting_id, description)| TrackedItem {
                id,
                meeting_id,
                description: plain_description(&description),
            })
            .collect())
    }

    /// Replace the item's `action_item_people` rows with the people mentioned in
    /// `description` (specs/0088). Mentions of people that no longer exist are skipped.
    pub async fn sync_mentions(
        pool: &SqlitePool,
        item_id: &str,
        description: &str,
    ) -> Result<(), SqlxError> {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM action_item_people WHERE action_item_id = ?")
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
        for pid in person_ids_in(description) {
            sqlx::query(
                "INSERT OR IGNORE INTO action_item_people (action_item_id, person_id)
                 SELECT ?, id FROM people WHERE id = ?",
            )
            .bind(item_id)
            .bind(&pid)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
}

/// Source-aware meeting delete (specs/0088), run inside `delete_meeting`'s transaction.
///
/// 1. Items of `meeting_id` with another source meeting move to the oldest such meeting
///    (`UPDATE OR IGNORE`; if the unique index refuses, fall through to 2/3).
/// 2. Remaining listed items become standalone (`meeting_id = NULL`).
/// 3. Everything else is deleted with its mention and source rows.
/// 4. This meeting's source rows go.
pub async fn cascade_meeting_delete(
    tx: &mut SqliteConnection,
    meeting_id: &str,
) -> Result<(), SqlxError> {
    let multi: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM action_items
         WHERE meeting_id = ?1
           AND EXISTS (SELECT 1 FROM action_item_sources s
                       JOIN meetings m ON m.id = s.meeting_id
                       WHERE s.action_item_id = action_items.id AND s.meeting_id != ?1)",
    )
    .bind(meeting_id)
    .fetch_all(&mut *tx)
    .await?;
    for id in multi {
        sqlx::query(
            "UPDATE OR IGNORE action_items SET meeting_id = (
                 SELECT s.meeting_id FROM action_item_sources s
                 JOIN meetings m ON m.id = s.meeting_id
                 WHERE s.action_item_id = ?1 AND s.meeting_id != ?2
                 ORDER BY m.created_at ASC, s.meeting_id ASC LIMIT 1)
             WHERE id = ?1",
        )
        .bind(&id)
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE action_items SET meeting_id = NULL WHERE meeting_id = ? AND triage = 'listed'",
    )
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?;

    for table in ["action_item_people", "action_item_sources"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE action_item_id IN
             (SELECT id FROM action_items WHERE meeting_id = ?)"
        ))
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("DELETE FROM action_items WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM action_item_sources WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// (item id, context, phrasing) of a primary source row.
pub(crate) type PrimarySource = (String, Option<String>, String);

/// Persist the source rows of one extraction run, inside its transaction (specs/0088).
///
/// 1. Sweep this meeting's source rows on items that no longer exist, and its earlier
///    ATTACHES (rows on items another meeting or nobody owns, `NOT IN` the owned set so
///    standalone to-dos count) that this run did not reproduce. Reproduced attaches are
///    kept and upserted below, so `added_at` stays at its first-seen value.
/// 2. Upsert the primary row of every item this run inserted, rewrote or matched, only
///    while this meeting still owns the item. Only context and phrasing change.
/// 3. Upsert each attach, only while its target still exists: the routing was computed
///    before a slow LLM call and the target may have been deleted since. An attach never
///    changes the target item itself (a dismissed or completed target stays as it is).
pub(crate) async fn apply_sources_in_tx(
    tx: &mut SqliteConnection,
    meeting_id: &str,
    primaries: &[PrimarySource],
    attaches: &[SourceAttach],
) -> Result<(), SqlxError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "DELETE FROM action_item_sources WHERE meeting_id = ? \
         AND action_item_id NOT IN (SELECT id FROM action_items)",
    )
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?;
    let attached: Vec<String> = sqlx::query_scalar(
        "SELECT action_item_id FROM action_item_sources WHERE meeting_id = ?1 \
         AND action_item_id NOT IN (SELECT id FROM action_items WHERE meeting_id = ?1)",
    )
    .bind(meeting_id)
    .fetch_all(&mut *tx)
    .await?;
    for id in attached
        .iter()
        .filter(|id| !attaches.iter().any(|a| &a.item_id == *id))
    {
        sqlx::query("DELETE FROM action_item_sources WHERE action_item_id = ? AND meeting_id = ?")
            .bind(id)
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
    }
    for (item_id, context, phrasing) in primaries {
        sqlx::query(
            "INSERT INTO action_item_sources \
                (action_item_id, meeting_id, context, phrasing, added_at) \
             SELECT id, ?2, ?3, ?4, ?5 FROM action_items WHERE id = ?1 AND meeting_id = ?2 \
             ON CONFLICT(action_item_id, meeting_id) DO UPDATE SET \
                context = excluded.context, phrasing = excluded.phrasing",
        )
        .bind(item_id)
        .bind(meeting_id)
        .bind(context.as_deref())
        .bind(phrasing)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    }
    for a in attaches {
        sqlx::query(
            "INSERT INTO action_item_sources \
                (action_item_id, meeting_id, context, phrasing, added_at) \
             SELECT id, ?2, ?3, ?4, ?5 FROM action_items WHERE id = ?1 \
             ON CONFLICT(action_item_id, meeting_id) DO UPDATE SET \
                context = excluded.context, phrasing = excluded.phrasing",
        )
        .bind(&a.item_id)
        .bind(meeting_id)
        .bind(a.context.as_deref())
        .bind(&a.phrasing)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Hard delete of one item together with its source and mention rows, in one transaction.
pub(crate) async fn delete_item_with_links(pool: &SqlitePool, id: &str) -> Result<bool, SqlxError> {
    let mut tx = pool.begin().await?;
    for table in ["action_item_sources", "action_item_people"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE action_item_id = ?"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let res = sqlx::query("DELETE FROM action_items WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Whether `meeting_id` currently has attach rows: sources on items that another meeting
/// (or nobody) owns. Feeds the zero-candidate safety valve.
pub async fn meeting_has_attaches(pool: &SqlitePool, meeting_id: &str) -> Result<bool, SqlxError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM action_item_sources s WHERE s.meeting_id = ?1 \
         AND s.action_item_id IN (SELECT id FROM action_items WHERE meeting_id IS NOT ?1))",
    )
    .bind(meeting_id)
    .fetch_one(pool)
    .await
}

/// A pristine extracted row that the meeting's new summary no longer supports.
///
/// - Mentioned in another meeting too: hand it to the OLDEST other source meeting
///   (`UPDATE OR IGNORE`, so a `(meeting_id, content_key)` clash there is a no-op) and drop
///   this meeting's source row. On a refused repoint, or when the user touched the row
///   since the diff snapshot, leave it fully intact, source row included.
/// - Otherwise: the ordinary pristine-guarded delete, plus its source and mention rows.
pub(crate) async fn delete_or_detach_in_tx(
    tx: &mut SqliteConnection,
    meeting_id: &str,
    item_id: &str,
) -> Result<(), SqlxError> {
    const PRISTINE: &str = "source = 'extracted' AND status = 'open' AND user_edited = 0";
    let other: Option<String> = sqlx::query_scalar(
        "SELECT s.meeting_id FROM action_item_sources s \
         JOIN meetings m ON m.id = s.meeting_id \
         WHERE s.action_item_id = ?1 AND s.meeting_id != ?2 \
         ORDER BY m.created_at ASC, s.meeting_id ASC LIMIT 1",
    )
    .bind(item_id)
    .bind(meeting_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(other) = other {
        let moved = sqlx::query(&format!(
            "UPDATE OR IGNORE action_items SET meeting_id = ?1 \
             WHERE id = ?2 AND meeting_id = ?3 AND {PRISTINE}"
        ))
        .bind(&other)
        .bind(item_id)
        .bind(meeting_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if moved == 1 {
            sqlx::query(
                "DELETE FROM action_item_sources WHERE action_item_id = ? AND meeting_id = ?",
            )
            .bind(item_id)
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
        }
        return Ok(());
    }
    let deleted = sqlx::query(&format!(
        "DELETE FROM action_items WHERE id = ? AND meeting_id = ? AND {PRISTINE}"
    ))
    .bind(item_id)
    .bind(meeting_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if deleted == 1 {
        for table in ["action_item_sources", "action_item_people"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE action_item_id = ?"))
                .bind(item_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(())
}
