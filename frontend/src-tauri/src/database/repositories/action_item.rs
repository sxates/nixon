//! `action_items` + `action_item_extractions` table access (specs/0034 Action items v1).
//!
//! `action_items` is the SOURCE OF TRUTH for task state — the summary markdown stays a
//! document. Rows come from LLM extraction over the persisted summary (`source =
//! 'extracted'`) or from the user (`source = 'manual'`; `meeting_id` NULL = a standalone
//! to-do created from the hub). Re-extraction is a DIFF, applied atomically by
//! [`replace_extracted`](ActionItemsRepository::replace_extracted): protected rows
//! (manual / user-edited / non-open) are never modified, deleted, or duplicated (see
//! `crate::action_items::diff`).
//!
//! `action_item_extractions` is the one-row-per-meeting extraction ledger: the input
//! fingerprint (idempotence) plus provider/model/count (debuggability).
//!
//! Mirrors `meeting_participant.rs`: returns `SqlxError`; the command layer maps to
//! user-facing strings. Cascades are EXPLICIT in delete transactions:
//! - meeting delete → drops this meeting's items + ledger row (see `meeting.rs`).
//! - person delete → NULLs `assignee_person_id` (the item outlives the person;
//!   `assignee_raw` keeps the display name — see `people.rs::delete`).

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, FromRow, SqlitePool};
use uuid::Uuid;

use crate::action_items::diff::{ExtractionDiff, ResolvedCandidate};

/// One action item, serialized camelCase for the frontend.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    /// `"ai-<uuid>"`.
    pub id: String,
    /// Source meeting; `None` = standalone manual to-do.
    pub meeting_id: Option<String>,
    pub description: String,
    /// → `people.id`; `None` = me / unresolved / unassigned.
    pub assignee_person_id: Option<String>,
    /// The app owner ("me") — NOT a `people` row (roster excludes self).
    pub assignee_is_self: bool,
    /// Assignee name as extracted when unresolved (display fallback).
    pub assignee_raw: Option<String>,
    /// Verbatim extracted due hint ("Friday"); provenance, never parsed away.
    pub due_hint: Option<String>,
    /// Structured ISO-8601 date (YYYY-MM-DD), sortable/filterable in the hub (specs/0038
    /// WS1.a). Set by the row's date control or best-effort from an unambiguous `due_hint`;
    /// `None` = no structured date. Lives ALONGSIDE `due_hint`.
    pub due_date: Option<String>,
    /// `'open' | 'completed' | 'dismissed'`.
    pub status: String,
    /// `'extracted' | 'manual'`.
    pub source: String,
    /// User has touched this row (edited content OR changed status) → protected from
    /// re-extraction, permanently.
    pub user_edited: bool,
    /// Fingerprint of the normalized description (diff identity).
    pub content_key: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    /// Manual drag-order key (specs/0038 WS1.b); `None` = unordered. Only
    /// [`ActionItemsRepository::reorder`] writes dense 0,1,2,… values. The default `list`
    /// order (meeting recency) ignores this — the hub's Manual sort reads it client-side.
    pub sort_order: Option<i64>,
}

/// Hub row: an action item joined to its source meeting's title/date (both `None` for a
/// standalone to-do or a deleted-but-orphaned reference).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemWithMeeting {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub item: ActionItem,
    pub meeting_title: Option<String>,
    pub meeting_created_at: Option<String>,
}

/// The per-meeting extraction ledger row.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemExtraction {
    pub meeting_id: String,
    /// Fingerprint of the extraction input (summary markdown + notes).
    pub summary_fingerprint: String,
    pub extracted_at: String,
    pub model_provider: String,
    pub model_name: String,
    pub item_count: i64,
}

/// Hub list filters (`api_list_action_items`). `status: None` = no status filter — the
/// command layer applies the spec's default (`'open'`).
#[derive(Debug, Default)]
pub struct ActionItemFilters<'a> {
    pub status: Option<&'a str>,
    pub person_id: Option<&'a str>,
    pub mine_only: bool,
}

/// Selects the rows a bulk status change targets (specs/0038 WS1.e — "dismiss items owned
/// by other people"). Encodes the three UI affordances: dismiss all from one person,
/// dismiss everyone-but-me, or an explicit hand-picked set (also the Undo primitive).
#[derive(Debug)]
pub enum BulkStatusFilter<'a> {
    /// Every item assigned to a specific `people` row (`assignee_person_id = ?`).
    Person(&'a str),
    /// Everyone but the owner: `assignee_is_self = 0`. "Me" is the `assignee_is_self`
    /// flag, never a `people` FK (the roster excludes self — specs/0018/0034), so this
    /// covers other people AND unassigned/raw-name rows while never touching my own items.
    NotSelf,
    /// An explicit id set — the general primitive, also how the frontend implements Undo
    /// (re-`'open'` exactly the ids it just dismissed).
    Ids(&'a [String]),
}

const SELECT_ITEM: &str = "SELECT id, meeting_id, description, assignee_person_id, \
     assignee_is_self, assignee_raw, due_hint, due_date, status, source, user_edited, \
     content_key, created_at, updated_at, completed_at, sort_order FROM action_items";

pub struct ActionItemsRepository;

impl ActionItemsRepository {
    /// One item by id.
    pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<ActionItem>, SqlxError> {
        sqlx::query_as::<_, ActionItem>(&format!("{SELECT_ITEM} WHERE id = ?"))
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    /// All items for a meeting (any status/source), oldest first — the per-meeting
    /// section AND the diff's "existing rows" input.
    pub async fn get_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<ActionItem>, SqlxError> {
        sqlx::query_as::<_, ActionItem>(&format!(
            "{SELECT_ITEM} WHERE meeting_id = ? ORDER BY created_at ASC, id ASC"
        ))
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    /// OPEN items across a SET of meetings (specs/0036 pre-call prep carryover): the
    /// still-open commitments from a recurring series' prior occurrences. Empty input → empty
    /// result (no query is issued). Ordered mine-first (`assignee_is_self` DESC) so the Prep
    /// view can split "mine" from "owed by others" and resolve people at the command layer.
    pub async fn get_open_for_meetings(
        pool: &SqlitePool,
        meeting_ids: &[String],
    ) -> Result<Vec<ActionItem>, SqlxError> {
        if meeting_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", meeting_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "{SELECT_ITEM} WHERE status = 'open' AND meeting_id IN ({placeholders}) \
             ORDER BY assignee_is_self DESC, created_at ASC, id ASC"
        );
        let mut query = sqlx::query_as::<_, ActionItem>(&sql);
        for id in meeting_ids {
            query = query.bind(id);
        }
        query.fetch_all(pool).await
    }

    /// Cross-meeting hub query: items joined to `meetings(title, created_at)`, newest
    /// meeting first (standalone to-dos last), filterable by status / assignee person /
    /// "mine only" (`assignee_is_self = 1`).
    pub async fn list(
        pool: &SqlitePool,
        filters: ActionItemFilters<'_>,
    ) -> Result<Vec<ActionItemWithMeeting>, SqlxError> {
        let mut sql = String::from(
            "SELECT ai.id, ai.meeting_id, ai.description, ai.assignee_person_id, \
             ai.assignee_is_self, ai.assignee_raw, ai.due_hint, ai.due_date, ai.status, \
             ai.source, ai.user_edited, ai.content_key, ai.created_at, ai.updated_at, \
             ai.completed_at, ai.sort_order, \
             m.title AS meeting_title, m.created_at AS meeting_created_at \
             FROM action_items ai \
             LEFT JOIN meetings m ON m.id = ai.meeting_id \
             WHERE 1 = 1",
        );
        if filters.status.is_some() {
            sql.push_str(" AND ai.status = ?");
        }
        if filters.person_id.is_some() {
            sql.push_str(" AND ai.assignee_person_id = ?");
        }
        if filters.mine_only {
            sql.push_str(" AND ai.assignee_is_self = 1");
        }
        sql.push_str(
            " ORDER BY (ai.meeting_id IS NULL) ASC, m.created_at DESC, ai.created_at ASC, ai.id ASC",
        );

        let mut query = sqlx::query_as::<_, ActionItemWithMeeting>(&sql);
        if let Some(status) = filters.status {
            query = query.bind(status.to_string());
        }
        if let Some(person_id) = filters.person_id {
            query = query.bind(person_id.to_string());
        }
        query.fetch_all(pool).await
    }

    /// Insert one item and return it. `content_key` is the caller-computed fingerprint of
    /// the normalized description (`crate::action_items::diff::content_key`).
    pub async fn create(
        pool: &SqlitePool,
        meeting_id: Option<&str>,
        candidate: &ResolvedCandidate,
        source: &str,
        content_key: &str,
    ) -> Result<ActionItem, SqlxError> {
        let id = format!("ai-{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO action_items
                (id, meeting_id, description, assignee_person_id, assignee_is_self,
                 assignee_raw, due_hint, due_date, status, source, user_edited, content_key,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'open', ?, 0, ?, ?, ?)",
        )
        .bind(&id)
        .bind(meeting_id)
        .bind(&candidate.description)
        .bind(candidate.assignee_person_id.as_deref())
        .bind(candidate.assignee_is_self)
        .bind(candidate.assignee_raw.as_deref())
        .bind(candidate.due_hint.as_deref())
        .bind(candidate.due_date.as_deref())
        .bind(source)
        .bind(content_key)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        // Fetch back rather than hand-assembling, so the returned row is exactly what
        // any later read will see.
        Self::get(pool, &id).await?.ok_or(SqlxError::RowNotFound)
    }

    /// User edit of an item's CONTENT (description/assignee/due). Sets `user_edited = 1`
    /// (→ protected from re-extraction) and stores the caller-recomputed `content_key`.
    /// Returns the updated row, or `None` when the item no longer exists.
    #[allow(clippy::too_many_arguments)] // one write-shape; a struct would just rename it
    pub async fn update_content(
        pool: &SqlitePool,
        id: &str,
        description: &str,
        assignee_person_id: Option<&str>,
        assignee_is_self: bool,
        assignee_raw: Option<&str>,
        due_hint: Option<&str>,
        due_date: Option<&str>,
        content_key: &str,
    ) -> Result<Option<ActionItem>, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE action_items
             SET description = ?, assignee_person_id = ?, assignee_is_self = ?,
                 assignee_raw = ?, due_hint = ?, due_date = ?, content_key = ?,
                 user_edited = 1, updated_at = ?
             WHERE id = ?",
        )
        .bind(description)
        .bind(assignee_person_id)
        .bind(assignee_is_self)
        .bind(assignee_raw)
        .bind(due_hint)
        .bind(due_date)
        .bind(content_key)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Self::get(pool, id).await
    }

    /// Status transition (`'open' | 'completed' | 'dismissed'`; the command layer
    /// validates). Manages `completed_at` (set on → 'completed', cleared otherwise).
    ///
    /// Only USERS flip status — the machine never calls this — so ANY transition,
    /// including back to `'open'`, also sets `user_edited = 1`: protection from
    /// re-extraction is permanent once the user has touched the row in any way. Without
    /// this, complete → un-check made the row pristine again and the 2026-07
    /// zero-candidate regeneration deleted it. The diff's `status != 'open'` protection
    /// arm still stands (belt-and-braces; see `diff::is_protected`).
    /// Returns the updated row, or `None` when the item no longer exists.
    pub async fn set_status(
        pool: &SqlitePool,
        id: &str,
        status: &str,
    ) -> Result<Option<ActionItem>, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let completed_at = (status == "completed").then_some(now.as_str());
        let res = sqlx::query(
            "UPDATE action_items
             SET status = ?, completed_at = ?, user_edited = 1, updated_at = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(completed_at)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Self::get(pool, id).await
    }

    /// Persists a manual drag order (specs/0038 WS1.b): writes dense `sort_order` values
    /// (0, 1, 2, …) matching `ordered_ids`'s position, in ONE transaction. Ids that no
    /// longer exist are skipped. Returns how many rows were written.
    ///
    /// Deliberately does NOT set `user_edited` (or bump `updated_at`): reordering is a
    /// view-only sort key, orthogonal to content — it must not protect a pristine
    /// extracted row from re-extraction, nor disturb meeting-recency views.
    pub async fn reorder(pool: &SqlitePool, ordered_ids: &[String]) -> Result<u64, SqlxError> {
        let mut tx = pool.begin().await?;
        let mut written: u64 = 0;
        for (idx, id) in ordered_ids.iter().enumerate() {
            let res = sqlx::query("UPDATE action_items SET sort_order = ? WHERE id = ?")
                .bind(idx as i64)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            written += res.rows_affected();
        }
        tx.commit().await?;
        Ok(written)
    }

    /// Bulk status change over a [`BulkStatusFilter`] set in ONE transaction (specs/0038
    /// WS1.e). Primary use: `status = 'dismissed'` to clear items owned by other people.
    /// Returns the affected row count (for an Undo toast).
    ///
    /// Like [`set_status`](Self::set_status), it manages `completed_at` and stamps
    /// `user_edited = 1` on every touched row — dismissing an extracted item must protect
    /// it, so 0034's re-extraction diff (`is_protected`: `status != 'open'` OR
    /// `user_edited`) never resurrects it.
    ///
    /// Scope: the `Person`/`NotSelf` "clear the clutter" modes only touch rows that are
    /// currently `'open'` (already-completed items owned by others stay resolved). The
    /// explicit `Ids` mode has NO status guard — it acts on exactly the given rows, which
    /// is what lets the frontend Undo a dismissal by re-`'open'`-ing those same ids.
    pub async fn bulk_set_status(
        pool: &SqlitePool,
        filter: &BulkStatusFilter<'_>,
        status: &str,
    ) -> Result<u64, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let completed_at = (status == "completed").then_some(now.as_str());

        let mut tx = pool.begin().await?;
        let affected = match filter {
            BulkStatusFilter::Person(person_id) => sqlx::query(
                "UPDATE action_items
                     SET status = ?, completed_at = ?, user_edited = 1, updated_at = ?
                     WHERE assignee_person_id = ? AND status = 'open'",
            )
            .bind(status)
            .bind(completed_at)
            .bind(&now)
            .bind(person_id)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            BulkStatusFilter::NotSelf => sqlx::query(
                "UPDATE action_items
                     SET status = ?, completed_at = ?, user_edited = 1, updated_at = ?
                     WHERE assignee_is_self = 0 AND status = 'open'",
            )
            .bind(status)
            .bind(completed_at)
            .bind(&now)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            BulkStatusFilter::Ids(ids) => {
                if ids.is_empty() {
                    tx.rollback().await?;
                    return Ok(0);
                }
                let placeholders = std::iter::repeat_n("?", ids.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "UPDATE action_items
                     SET status = ?, completed_at = ?, user_edited = 1, updated_at = ?
                     WHERE id IN ({placeholders})"
                );
                let mut query = sqlx::query(&sql).bind(status).bind(completed_at).bind(&now);
                for id in *ids {
                    query = query.bind(id);
                }
                query.execute(&mut *tx).await?.rows_affected()
            }
        };
        tx.commit().await?;
        Ok(affected)
    }

    /// Hard delete. The UI offers this for MANUAL items only — deleting an extracted item
    /// would just get re-proposed on the next run; dismissing is the persistent "no".
    pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool, SqlxError> {
        let res = sqlx::query("DELETE FROM action_items WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// The meeting's extraction ledger row, if any run ever completed.
    pub async fn get_extraction(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<ActionItemExtraction>, SqlxError> {
        sqlx::query_as::<_, ActionItemExtraction>(
            "SELECT meeting_id, summary_fingerprint, extracted_at, model_provider,
                    model_name, item_count
             FROM action_item_extractions WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
    }

    /// Applies an extraction diff atomically: pristine deletes + pristine in-place
    /// updates + fresh inserts + the ledger upsert, all in ONE transaction (specs/0034
    /// step 6). `item_count` records how many candidates the extractor returned.
    ///
    /// Returns `Ok(None)` — after rolling back, writing NOTHING — when the meeting no
    /// longer exists: the extraction's LLM call is slow and fire-and-forget, so the user
    /// may delete the meeting mid-run; committing would resurrect its items + ledger row
    /// as orphans (the FKs are documentation-only, nothing else stops it). Otherwise
    /// returns `Ok(Some(n))` where `n` is the number of item rows actually inserted or
    /// updated (the new IPC contract's return value — skipped guard-protected rows are
    /// not counted).
    ///
    /// Protected rows never appear in the diff by construction, but the plan is computed
    /// from a SNAPSHOT — the user can complete/edit/dismiss a row between `compute_diff`
    /// and this apply. Every DELETE/UPDATE therefore re-asserts pristineness
    /// (`source = 'extracted' AND status = 'open' AND user_edited = 0`) in its WHERE
    /// clause: a row the user touched in that window is silently skipped (the user won
    /// the race), never overwritten.
    pub async fn replace_extracted(
        pool: &SqlitePool,
        meeting_id: &str,
        diff: &ExtractionDiff,
        summary_fingerprint: &str,
        model_provider: &str,
        model_name: &str,
        item_count: u32,
    ) -> Result<Option<u32>, SqlxError> {
        let mut tx = pool.begin().await?;
        let now = Utc::now().to_rfc3339();
        let mut rows_written: u32 = 0;

        // Abort if the meeting was deleted while extraction ran (see doc above).
        let meeting_exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(&mut *tx)
            .await?;
        if meeting_exists.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }

        for id in &diff.delete_ids {
            // meeting_id guard: the diff was computed from this meeting's rows only, but
            // never let a stale plan delete another meeting's item. Pristine guard: only
            // machine-owned rows are deletable (see doc above).
            sqlx::query(
                "DELETE FROM action_items
                 WHERE id = ? AND meeting_id = ?
                   AND source = 'extracted' AND status = 'open' AND user_edited = 0",
            )
            .bind(id)
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
        }

        for update in &diff.updates {
            // In-place rewrite: id, created_at, status, source, user_edited preserved.
            // Pristine guard: a row the user touched since the diff snapshot is skipped.
            let res = sqlx::query(
                "UPDATE action_items
                 SET description = ?, assignee_person_id = ?, assignee_is_self = ?,
                     assignee_raw = ?, due_hint = ?, due_date = ?, content_key = ?,
                     updated_at = ?
                 WHERE id = ? AND meeting_id = ?
                   AND source = 'extracted' AND status = 'open' AND user_edited = 0",
            )
            .bind(&update.candidate.description)
            .bind(update.candidate.assignee_person_id.as_deref())
            .bind(update.candidate.assignee_is_self)
            .bind(update.candidate.assignee_raw.as_deref())
            .bind(update.candidate.due_hint.as_deref())
            .bind(update.candidate.due_date.as_deref())
            .bind(&update.content_key)
            .bind(&now)
            .bind(&update.id)
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
            rows_written += res.rows_affected() as u32;
        }

        for insert in &diff.inserts {
            let id = format!("ai-{}", Uuid::new_v4());
            sqlx::query(
                "INSERT INTO action_items
                    (id, meeting_id, description, assignee_person_id, assignee_is_self,
                     assignee_raw, due_hint, due_date, status, source, user_edited,
                     content_key, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'open', 'extracted', 0, ?, ?, ?)",
            )
            .bind(&id)
            .bind(meeting_id)
            .bind(&insert.candidate.description)
            .bind(insert.candidate.assignee_person_id.as_deref())
            .bind(insert.candidate.assignee_is_self)
            .bind(insert.candidate.assignee_raw.as_deref())
            .bind(insert.candidate.due_hint.as_deref())
            .bind(insert.candidate.due_date.as_deref())
            .bind(&insert.content_key)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            rows_written += 1;
        }

        sqlx::query(
            "INSERT INTO action_item_extractions
                (meeting_id, summary_fingerprint, extracted_at, model_provider,
                 model_name, item_count)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(meeting_id) DO UPDATE SET
                summary_fingerprint = excluded.summary_fingerprint,
                extracted_at = excluded.extracted_at,
                model_provider = excluded.model_provider,
                model_name = excluded.model_name,
                item_count = excluded.item_count",
        )
        .bind(meeting_id)
        .bind(summary_fingerprint)
        .bind(&now)
        .bind(model_provider)
        .bind(model_name)
        .bind(item_count)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(rows_written))
    }
}

#[cfg(test)]
mod action_items_tests {
    // Named `action_items_tests` (not the conventional `tests`) so the spec's
    // verification filter `cargo test action_items` picks these up alongside the
    // `action_items::` module tests.
    use super::*;
    use crate::action_items::diff::{self, CandidateInsert, PristineUpdate};
    use crate::database::repositories::meeting::MeetingsRepository;
    use crate::database::repositories::people::PeopleRepository;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Fresh in-memory SQLite brought up through the app's real migration set (mirrors
    /// `meeting.rs` tests). One connection max — each in-memory connection is a separate
    /// database.
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

    fn candidate(description: &str) -> ResolvedCandidate {
        ResolvedCandidate {
            description: description.to_string(),
            assignee_person_id: None,
            assignee_is_self: false,
            assignee_raw: None,
            due_hint: None,
            due_date: None,
        }
    }

    async fn create_meeting(pool: &SqlitePool) -> String {
        MeetingsRepository::create_meeting(pool, Some("Sync".into()), None, None, None, None)
            .await
            .expect("create_meeting")
    }

    #[tokio::test]
    async fn open_for_meetings_scopes_to_set_and_open_only_mine_first() {
        let pool = test_pool().await;
        let m1 = create_meeting(&pool).await;
        let m2 = create_meeting(&pool).await;
        let m_other = create_meeting(&pool).await;

        // m1: one mine (open), one others (open), one completed.
        let mut mine = candidate("I will circulate the roadmap");
        mine.assignee_is_self = true;
        ActionItemsRepository::create(
            &pool,
            Some(&m1),
            &mine,
            "extracted",
            &diff::content_key(&mine.description),
        )
        .await
        .unwrap();
        let others = candidate("Priya to send the metrics");
        ActionItemsRepository::create(
            &pool,
            Some(&m1),
            &others,
            "extracted",
            &diff::content_key(&others.description),
        )
        .await
        .unwrap();
        let done = candidate("Already handled");
        let done_item = ActionItemsRepository::create(
            &pool,
            Some(&m1),
            &done,
            "extracted",
            &diff::content_key(&done.description),
        )
        .await
        .unwrap();
        ActionItemsRepository::set_status(&pool, &done_item.id, "completed")
            .await
            .unwrap();

        // m2: one open. m_other (not in the set): one open that must NOT appear.
        let m2_item = candidate("Backfill the hiring plan");
        ActionItemsRepository::create(
            &pool,
            Some(&m2),
            &m2_item,
            "extracted",
            &diff::content_key(&m2_item.description),
        )
        .await
        .unwrap();
        let outside = candidate("Do not include me");
        ActionItemsRepository::create(
            &pool,
            Some(&m_other),
            &outside,
            "extracted",
            &diff::content_key(&outside.description),
        )
        .await
        .unwrap();

        let open = ActionItemsRepository::get_open_for_meetings(&pool, &[m1.clone(), m2.clone()])
            .await
            .unwrap();
        // 3 open items across the set (m1 mine + m1 others + m2), completed excluded, outside excluded.
        assert_eq!(open.len(), 3);
        assert!(open.iter().all(|i| i.status == "open"));
        assert!(!open
            .iter()
            .any(|i| i.description.contains("Do not include")));
        // Mine sorts first (assignee_is_self DESC).
        assert!(open[0].assignee_is_self, "mine-first ordering");

        // Empty input short-circuits to empty.
        assert!(ActionItemsRepository::get_open_for_meetings(&pool, &[])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn create_get_and_meeting_scope_roundtrip() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;

        let mut c = candidate("Send the deck to Alice");
        c.due_hint = Some("Friday".into());
        let item = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &c,
            "extracted",
            &diff::content_key(&c.description),
        )
        .await
        .unwrap();

        assert!(item.id.starts_with("ai-"));
        assert_eq!(item.meeting_id.as_deref(), Some(meeting_id.as_str()));
        assert_eq!(item.status, "open");
        assert_eq!(item.source, "extracted");
        assert!(!item.user_edited);
        assert_eq!(item.due_hint.as_deref(), Some("Friday"));

        let for_meeting = ActionItemsRepository::get_for_meeting(&pool, &meeting_id)
            .await
            .unwrap();
        assert_eq!(for_meeting.len(), 1);
        assert_eq!(for_meeting[0].id, item.id);

        // Standalone manual to-do: no meeting.
        let standalone = ActionItemsRepository::create(
            &pool,
            None,
            &candidate("Water the plants"),
            "manual",
            &diff::content_key("Water the plants"),
        )
        .await
        .unwrap();
        assert!(standalone.meeting_id.is_none());
        assert_eq!(standalone.source, "manual");
        // It does not leak into the meeting's list.
        assert_eq!(
            ActionItemsRepository::get_for_meeting(&pool, &meeting_id)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn list_joins_meeting_and_applies_filters() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
            .await
            .unwrap();

        let mut for_alice = candidate("Book the room");
        for_alice.assignee_person_id = Some(alice.id.clone());
        ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &for_alice,
            "extracted",
            &diff::content_key("Book the room"),
        )
        .await
        .unwrap();

        let mut mine = candidate("Send the deck");
        mine.assignee_is_self = true;
        let mine_row = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &mine,
            "extracted",
            &diff::content_key("Send the deck"),
        )
        .await
        .unwrap();
        ActionItemsRepository::set_status(&pool, &mine_row.id, "completed")
            .await
            .unwrap();

        // Standalone open to-do (no meeting → NULL title, ordered last).
        ActionItemsRepository::create(
            &pool,
            None,
            &candidate("Standalone task"),
            "manual",
            &diff::content_key("Standalone task"),
        )
        .await
        .unwrap();

        // Default hub filter: open only.
        let open = ActionItemsRepository::list(
            &pool,
            ActionItemFilters {
                status: Some("open"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].meeting_title.as_deref(), Some("Sync"));
        assert!(open[0].meeting_created_at.is_some());
        // Standalone last.
        assert!(open[1].meeting_title.is_none());
        assert!(open[1].item.meeting_id.is_none());

        // Person filter.
        let alices = ActionItemsRepository::list(
            &pool,
            ActionItemFilters {
                status: Some("open"),
                person_id: Some(&alice.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(alices.len(), 1);
        assert_eq!(alices[0].item.description, "Book the room");

        // Mine-only (regardless of status here: completed + mine).
        let mine_items = ActionItemsRepository::list(
            &pool,
            ActionItemFilters {
                status: Some("completed"),
                mine_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(mine_items.len(), 1);
        assert_eq!(mine_items[0].item.description, "Send the deck");

        // No status filter → everything.
        let all = ActionItemsRepository::list(&pool, ActionItemFilters::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn update_content_sets_user_edited_and_new_content_key() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let item = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("Send the deck"),
            "extracted",
            &diff::content_key("Send the deck"),
        )
        .await
        .unwrap();

        let updated = ActionItemsRepository::update_content(
            &pool,
            &item.id,
            "Send the FINAL deck",
            None,
            true,
            None,
            Some("Friday"),
            Some("2026-07-10"),
            &diff::content_key("Send the FINAL deck"),
        )
        .await
        .unwrap()
        .expect("item exists");

        assert_eq!(updated.id, item.id);
        assert_eq!(updated.description, "Send the FINAL deck");
        assert!(updated.user_edited, "content edit protects the row");
        assert!(updated.assignee_is_self);
        assert_eq!(updated.due_hint.as_deref(), Some("Friday"));
        assert_eq!(
            updated.due_date.as_deref(),
            Some("2026-07-10"),
            "structured due_date round-trips alongside the hint"
        );
        assert_eq!(
            updated.content_key,
            diff::content_key("Send the FINAL deck")
        );
        assert_eq!(updated.created_at, item.created_at, "created_at preserved");
        assert_eq!(updated.status, "open", "status untouched by content edit");

        // Missing id → None.
        assert!(ActionItemsRepository::update_content(
            &pool,
            "ai-missing",
            "x",
            None,
            false,
            None,
            None,
            None,
            "k"
        )
        .await
        .unwrap()
        .is_none());
    }

    /// Regression (2026-07 zero-candidate incident): EVERY user status transition —
    /// completed, dismissed, AND back to open — sets `user_edited = 1`. Before this,
    /// un-checking a completed item made the row pristine again and a suspect
    /// re-extraction deleted it as "unmatched pristine".
    #[tokio::test]
    async fn set_status_manages_completed_at_and_marks_user_edited() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let item = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("Send the deck"),
            "extracted",
            &diff::content_key("Send the deck"),
        )
        .await
        .unwrap();
        assert!(!item.user_edited, "freshly extracted row starts pristine");

        let done = ActionItemsRepository::set_status(&pool, &item.id, "completed")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.status, "completed");
        assert!(done.completed_at.is_some());
        assert!(done.user_edited, "completing marks the row user-touched");

        let reopened = ActionItemsRepository::set_status(&pool, &item.id, "open")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reopened.status, "open");
        assert!(
            reopened.completed_at.is_none(),
            "reopen clears completed_at"
        );
        assert!(
            reopened.user_edited,
            "un-checking back to open must NOT reset protection (the incident)"
        );

        let dismissed = ActionItemsRepository::set_status(&pool, &item.id, "dismissed")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dismissed.status, "dismissed");
        assert!(dismissed.completed_at.is_none());
        assert!(dismissed.user_edited);

        // A fresh row's FIRST transition is dismissed → still marked.
        let other = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("Book the room"),
            "extracted",
            &diff::content_key("Book the room"),
        )
        .await
        .unwrap();
        let dismissed_first = ActionItemsRepository::set_status(&pool, &other.id, "dismissed")
            .await
            .unwrap()
            .unwrap();
        assert!(
            dismissed_first.user_edited,
            "dismissing marks the row user-touched"
        );

        assert!(
            ActionItemsRepository::set_status(&pool, "ai-missing", "open")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let pool = test_pool().await;
        let item = ActionItemsRepository::create(
            &pool,
            None,
            &candidate("Standalone"),
            "manual",
            &diff::content_key("Standalone"),
        )
        .await
        .unwrap();
        assert!(ActionItemsRepository::delete(&pool, &item.id)
            .await
            .unwrap());
        assert!(!ActionItemsRepository::delete(&pool, &item.id)
            .await
            .unwrap());
        assert!(ActionItemsRepository::get(&pool, &item.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn replace_extracted_applies_diff_and_upserts_ledger_atomically() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;

        // Seed: one pristine row to update, one pristine row to delete.
        let keep = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("send the deck to alice"),
            "extracted",
            &diff::content_key("send the deck to alice"),
        )
        .await
        .unwrap();
        let drop_me = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("book the room"),
            "extracted",
            &diff::content_key("book the room"),
        )
        .await
        .unwrap();

        let plan = ExtractionDiff {
            inserts: vec![CandidateInsert {
                candidate: candidate("follow up with legal"),
                content_key: diff::content_key("follow up with legal"),
            }],
            updates: vec![PristineUpdate {
                id: keep.id.clone(),
                candidate: candidate("send deck to alice"),
                content_key: diff::content_key("send deck to alice"),
            }],
            delete_ids: vec![drop_me.id.clone()],
        };

        let written = ActionItemsRepository::replace_extracted(
            &pool,
            &meeting_id,
            &plan,
            "fp-1",
            "ollama",
            "llama3.2:latest",
            2,
        )
        .await
        .unwrap();
        assert_eq!(
            written,
            Some(2),
            "1 insert + 1 update actually written (deletes not counted)"
        );

        let rows = ActionItemsRepository::get_for_meeting(&pool, &meeting_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        let updated = rows
            .iter()
            .find(|r| r.id == keep.id)
            .expect("updated row kept");
        assert_eq!(updated.description, "send deck to alice");
        assert_eq!(
            updated.created_at, keep.created_at,
            "in-place update preserves created_at"
        );
        assert!(
            rows.iter().all(|r| r.id != drop_me.id),
            "pristine unmatched deleted"
        );
        assert!(rows.iter().any(|r| r.description == "follow up with legal"));

        let ledger = ActionItemsRepository::get_extraction(&pool, &meeting_id)
            .await
            .unwrap()
            .expect("ledger written");
        assert_eq!(ledger.summary_fingerprint, "fp-1");
        assert_eq!(ledger.model_provider, "ollama");
        assert_eq!(ledger.item_count, 2);

        // Second run upserts (one row per meeting); an empty diff writes zero rows.
        let written = ActionItemsRepository::replace_extracted(
            &pool,
            &meeting_id,
            &ExtractionDiff::default(),
            "fp-2",
            "ollama",
            "llama3.2:latest",
            0,
        )
        .await
        .unwrap();
        assert_eq!(written, Some(0));
        let ledger = ActionItemsRepository::get_extraction(&pool, &meeting_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.summary_fingerprint, "fp-2");
        assert_eq!(ledger.item_count, 0);
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM action_item_extractions WHERE meeting_id = ?")
                .bind(&meeting_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count.0, 1);
    }

    /// Regression (delete-then-commit): the extraction's LLM call is in flight while the
    /// user deletes the meeting; the apply must roll back and write NOTHING (no orphan
    /// items, no orphan ledger row) — the FKs are documentation-only, so this check is
    /// the only thing standing between the in-flight task and resurrection.
    #[tokio::test]
    async fn replace_extracted_aborts_when_meeting_deleted_mid_run() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;

        // Plan computed while the meeting still existed...
        let plan = ExtractionDiff {
            inserts: vec![CandidateInsert {
                candidate: candidate("send the deck"),
                content_key: diff::content_key("send the deck"),
            }],
            updates: vec![],
            delete_ids: vec![],
        };

        // ...then the user deletes the meeting before the apply.
        assert!(MeetingsRepository::delete_meeting(&pool, &meeting_id)
            .await
            .unwrap());

        let written = ActionItemsRepository::replace_extracted(
            &pool,
            &meeting_id,
            &plan,
            "fp",
            "ollama",
            "m",
            1,
        )
        .await
        .unwrap();
        assert_eq!(written, None, "meeting gone → abort, not apply");

        assert!(
            ActionItemsRepository::get_for_meeting(&pool, &meeting_id)
                .await
                .unwrap()
                .is_empty(),
            "no orphan items resurrected"
        );
        assert!(
            ActionItemsRepository::get_extraction(&pool, &meeting_id)
                .await
                .unwrap()
                .is_none(),
            "no orphan ledger row"
        );
    }

    /// Regression (snapshot race): the diff plan targets rows that were pristine at
    /// snapshot time, but the user completes one and edits another before the apply.
    /// The write-time guards (`source='extracted' AND status='open' AND user_edited=0`)
    /// must skip both — the user won the race — and the skipped update must not count
    /// toward the rows-written return.
    #[tokio::test]
    async fn write_guards_skip_rows_user_touched_after_snapshot() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;

        let edited_target = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("send the deck to alice"),
            "extracted",
            &diff::content_key("send the deck to alice"),
        )
        .await
        .unwrap();
        let completed_target = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("book the room"),
            "extracted",
            &diff::content_key("book the room"),
        )
        .await
        .unwrap();

        // The plan, computed from the pristine snapshot: rewrite one, delete the other.
        let plan = ExtractionDiff {
            inserts: vec![],
            updates: vec![PristineUpdate {
                id: edited_target.id.clone(),
                candidate: candidate("send deck to alice"),
                content_key: diff::content_key("send deck to alice"),
            }],
            delete_ids: vec![completed_target.id.clone()],
        };

        // Between snapshot and apply, the user touches BOTH rows.
        let user_text = "send the deck to alice (final numbers!)";
        ActionItemsRepository::update_content(
            &pool,
            &edited_target.id,
            user_text,
            None,
            false,
            None,
            None,
            None,
            &diff::content_key(user_text),
        )
        .await
        .unwrap()
        .unwrap();
        ActionItemsRepository::set_status(&pool, &completed_target.id, "completed")
            .await
            .unwrap()
            .unwrap();

        let written = ActionItemsRepository::replace_extracted(
            &pool,
            &meeting_id,
            &plan,
            "fp-race",
            "ollama",
            "m",
            1,
        )
        .await
        .unwrap();
        assert_eq!(
            written,
            Some(0),
            "guard-skipped update must not count as written"
        );

        let rows = ActionItemsRepository::get_for_meeting(&pool, &meeting_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "completed row NOT deleted by the stale plan");
        let edited_after = rows.iter().find(|r| r.id == edited_target.id).unwrap();
        assert_eq!(
            edited_after.description, user_text,
            "user's edit survives the stale update"
        );
        assert!(edited_after.user_edited);
        let completed_after = rows.iter().find(|r| r.id == completed_target.id).unwrap();
        assert_eq!(completed_after.status, "completed");
    }

    /// The migration's partial unique index: at most one EXTRACTED row per
    /// (meeting, content_key); manual rows and other meetings are exempt.
    #[tokio::test]
    async fn extracted_rows_are_unique_per_meeting_and_content_key() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let other_meeting = create_meeting(&pool).await;
        let key = diff::content_key("send the deck");

        ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("send the deck"),
            "extracted",
            &key,
        )
        .await
        .unwrap();
        // Same meeting + key + extracted → constraint error (the DB backstop).
        assert!(
            ActionItemsRepository::create(
                &pool,
                Some(&meeting_id),
                &candidate("send the deck"),
                "extracted",
                &key
            )
            .await
            .is_err(),
            "duplicate extracted row must violate the unique index"
        );
        // Manual duplicate is allowed (user's prerogative)...
        ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("send the deck"),
            "manual",
            &key,
        )
        .await
        .unwrap();
        // ...and so is the same content on another meeting.
        ActionItemsRepository::create(
            &pool,
            Some(&other_meeting),
            &candidate("send the deck"),
            "extracted",
            &key,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn meeting_delete_cascades_items_and_ledger() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("task"),
            "extracted",
            &diff::content_key("task"),
        )
        .await
        .unwrap();
        // A standalone to-do must SURVIVE the meeting delete.
        let standalone = ActionItemsRepository::create(
            &pool,
            None,
            &candidate("standalone"),
            "manual",
            &diff::content_key("standalone"),
        )
        .await
        .unwrap();
        ActionItemsRepository::replace_extracted(
            &pool,
            &meeting_id,
            &ExtractionDiff::default(),
            "fp",
            "ollama",
            "m",
            1,
        )
        .await
        .unwrap();

        assert!(MeetingsRepository::delete_meeting(&pool, &meeting_id)
            .await
            .unwrap());

        assert!(ActionItemsRepository::get_for_meeting(&pool, &meeting_id)
            .await
            .unwrap()
            .is_empty());
        assert!(ActionItemsRepository::get_extraction(&pool, &meeting_id)
            .await
            .unwrap()
            .is_none());
        assert!(ActionItemsRepository::get(&pool, &standalone.id)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn person_delete_nulls_assignee_but_keeps_item() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
            .await
            .unwrap();

        let mut c = candidate("Book the room");
        c.assignee_person_id = Some(alice.id.clone());
        c.assignee_raw = Some("Alice R.".into());
        let item = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &c,
            "extracted",
            &diff::content_key("Book the room"),
        )
        .await
        .unwrap();
        // The common shape: resolution set the FK and left `assignee_raw` NULL — the
        // delete must backfill it with the person's display name (finding: otherwise the
        // task silently flips to "unassigned").
        let mut fk_only = candidate("Send the agenda");
        fk_only.assignee_person_id = Some(alice.id.clone());
        let fk_only_item = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &fk_only,
            "extracted",
            &diff::content_key("Send the agenda"),
        )
        .await
        .unwrap();
        assert!(fk_only_item.assignee_raw.is_none());

        assert!(PeopleRepository::delete(&pool, &alice.id).await.unwrap());

        let after = ActionItemsRepository::get(&pool, &item.id)
            .await
            .unwrap()
            .expect("item outlives the person");
        assert!(after.assignee_person_id.is_none(), "FK nulled");
        assert_eq!(
            after.assignee_raw.as_deref(),
            Some("Alice R."),
            "an existing raw name is never overwritten"
        );

        let fk_only_after = ActionItemsRepository::get(&pool, &fk_only_item.id)
            .await
            .unwrap()
            .unwrap();
        assert!(fk_only_after.assignee_person_id.is_none());
        assert_eq!(
            fk_only_after.assignee_raw.as_deref(),
            Some("Alice"),
            "deleted person's display name backfilled into assignee_raw"
        );
    }

    // ---- specs/0038 WS1 ---------------------------------------------------------------

    /// WS1.b — `reorder` writes dense 0,1,2,… `sort_order` in the given id order, skips
    /// unknown ids, and does NOT protect the row (order is orthogonal to content).
    #[tokio::test]
    async fn reorder_writes_dense_sort_order_without_protecting_rows() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let a = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("alpha"),
            "extracted",
            &diff::content_key("alpha"),
        )
        .await
        .unwrap();
        let b = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("bravo"),
            "extracted",
            &diff::content_key("bravo"),
        )
        .await
        .unwrap();
        let c = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("charlie"),
            "extracted",
            &diff::content_key("charlie"),
        )
        .await
        .unwrap();

        // Fresh rows have no order.
        assert!(a.sort_order.is_none());

        // Reorder c, a, b — plus a ghost id that no longer exists (skipped).
        let written = ActionItemsRepository::reorder(
            &pool,
            &[
                c.id.clone(),
                a.id.clone(),
                b.id.clone(),
                "ai-does-not-exist".to_string(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(written, 3, "only the three real rows are written");

        let after = |id: &str| {
            let pool = &pool;
            let id = id.to_string();
            async move {
                ActionItemsRepository::get(pool, &id)
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        assert_eq!(after(&c.id).await.sort_order, Some(0));
        assert_eq!(after(&a.id).await.sort_order, Some(1));
        assert_eq!(after(&b.id).await.sort_order, Some(2));
        // Reordering must NOT protect a pristine extracted row from re-extraction.
        assert!(
            !after(&a.id).await.user_edited,
            "reorder is view-only — never sets user_edited"
        );

        // Re-reordering rewrites densely (a first now).
        ActionItemsRepository::reorder(&pool, &[a.id.clone(), b.id.clone(), c.id.clone()])
            .await
            .unwrap();
        assert_eq!(after(&a.id).await.sort_order, Some(0));
        assert_eq!(after(&c.id).await.sort_order, Some(2));
    }

    /// WS1.e — `NotSelf` bulk dismiss flips every OPEN item not assigned to me, leaves my
    /// items and already-completed others' items untouched, returns the affected count,
    /// and marks each touched row `user_edited = 1`.
    #[tokio::test]
    async fn bulk_set_status_not_self_dismisses_only_others_open() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
            .await
            .unwrap();

        let mut mine = candidate("my task");
        mine.assignee_is_self = true;
        let mine = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &mine,
            "extracted",
            &diff::content_key("my task"),
        )
        .await
        .unwrap();

        let mut for_alice = candidate("alice open task");
        for_alice.assignee_person_id = Some(alice.id.clone());
        ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &for_alice,
            "extracted",
            &diff::content_key("alice open task"),
        )
        .await
        .unwrap();

        // Unassigned (assignee_is_self = 0) — counts as "not me".
        let unassigned = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("unassigned task"),
            "extracted",
            &diff::content_key("unassigned task"),
        )
        .await
        .unwrap();

        // Alice, but already completed → must be left resolved (status guard is 'open').
        let mut alice_done = candidate("alice done task");
        alice_done.assignee_person_id = Some(alice.id.clone());
        let alice_done = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &alice_done,
            "extracted",
            &diff::content_key("alice done task"),
        )
        .await
        .unwrap();
        ActionItemsRepository::set_status(&pool, &alice_done.id, "completed")
            .await
            .unwrap();

        let count =
            ActionItemsRepository::bulk_set_status(&pool, &BulkStatusFilter::NotSelf, "dismissed")
                .await
                .unwrap();
        assert_eq!(
            count, 2,
            "alice-open + unassigned-open dismissed; count returned"
        );

        let mine_after = ActionItemsRepository::get(&pool, &mine.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mine_after.status, "open", "my own item is never touched");
        let unassigned_after = ActionItemsRepository::get(&pool, &unassigned.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unassigned_after.status, "dismissed");
        assert!(
            unassigned_after.user_edited,
            "bulk dismiss protects the row from re-extraction"
        );
        let alice_done_after = ActionItemsRepository::get(&pool, &alice_done.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            alice_done_after.status, "completed",
            "already-completed other's item stays resolved"
        );
    }

    /// WS1.e — `Person` mode dismisses one person's open items; `Ids` mode acts on exactly
    /// the given rows regardless of status (the Undo primitive: re-`open` dismissed ids).
    #[tokio::test]
    async fn bulk_set_status_by_person_then_reopen_by_ids() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let bob = PeopleRepository::create(&pool, "Bob", None, None, None)
            .await
            .unwrap();

        let mut for_bob = candidate("bob task one");
        for_bob.assignee_person_id = Some(bob.id.clone());
        let bob1 = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &for_bob,
            "extracted",
            &diff::content_key("bob task one"),
        )
        .await
        .unwrap();
        let mut for_bob2 = candidate("bob task two");
        for_bob2.assignee_person_id = Some(bob.id.clone());
        let bob2 = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &for_bob2,
            "extracted",
            &diff::content_key("bob task two"),
        )
        .await
        .unwrap();

        let count = ActionItemsRepository::bulk_set_status(
            &pool,
            &BulkStatusFilter::Person(&bob.id),
            "dismissed",
        )
        .await
        .unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            ActionItemsRepository::get(&pool, &bob1.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "dismissed"
        );

        // Undo: re-open exactly those ids via the Ids mode (acts on dismissed rows).
        let reopened = ActionItemsRepository::bulk_set_status(
            &pool,
            &BulkStatusFilter::Ids(&[bob1.id.clone(), bob2.id.clone()]),
            "open",
        )
        .await
        .unwrap();
        assert_eq!(reopened, 2, "Ids mode has no status guard — undo works");
        assert_eq!(
            ActionItemsRepository::get(&pool, &bob2.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "open"
        );

        // Empty id set short-circuits to zero.
        assert_eq!(
            ActionItemsRepository::bulk_set_status(&pool, &BulkStatusFilter::Ids(&[]), "dismissed")
                .await
                .unwrap(),
            0
        );
    }

    /// WS1.e — a BULK-dismissed extracted item survives re-extraction: it is protected
    /// (`status != 'open'` AND `user_edited = 1`), so a candidate that reproduces it is
    /// dropped by the diff, never resurrected or duplicated.
    #[tokio::test]
    async fn bulk_dismissed_items_survive_re_extraction() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        let item = ActionItemsRepository::create(
            &pool,
            Some(&meeting_id),
            &candidate("follow up with legal"),
            "extracted",
            &diff::content_key("follow up with legal"),
        )
        .await
        .unwrap();

        // Bulk dismiss (unassigned → not me).
        assert_eq!(
            ActionItemsRepository::bulk_set_status(&pool, &BulkStatusFilter::NotSelf, "dismissed")
                .await
                .unwrap(),
            1
        );

        // Re-extraction reproduces the same task verbatim.
        let existing = ActionItemsRepository::get_for_meeting(&pool, &meeting_id)
            .await
            .unwrap();
        let candidates = [candidate("follow up with legal")];
        let plan = diff::compute_diff(&existing, &candidates);
        assert!(
            plan.is_empty(),
            "candidate absorbed by the protected dismissed row — no insert, no delete"
        );
        ActionItemsRepository::replace_extracted(&pool, &meeting_id, &plan, "fp", "ollama", "m", 1)
            .await
            .unwrap();

        let rows = ActionItemsRepository::get_for_meeting(&pool, &meeting_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "no resurrection, no duplicate");
        assert_eq!(rows[0].id, item.id);
        assert_eq!(rows[0].status, "dismissed");
    }
}
