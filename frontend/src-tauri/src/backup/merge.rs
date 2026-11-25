//! Add-what's-missing import, SQL half (specs/0092, "Import: Add what's missing").
//!
//! The staged (migrated) backup DB is attached as `backup` on one pooled connection and rows
//! this Mac lacks are copied in. Nothing already here is updated or replaced: every statement is
//! `INSERT OR IGNORE`, and the only `UPDATE`s touch rows this import itself just inserted.
//!
//! The pool enforces foreign keys and `OR IGNORE` does not ignore FK violations, so each insert
//! carries explicit `EXISTS` filters for its references (declared foreign keys plus
//! [`LOGICAL_REFS`]). A row whose target exists neither here nor in the import is skipped.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde::Serialize;
use sqlx::SqliteConnection;

use super::BackupError;

/// How the Add import treats a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableClass {
    /// Describes this machine or install; this Mac's copy always wins.
    MachineBound,
    /// `meetings`: backup meetings whose id is not here are inserted.
    Meeting,
    /// Rows hanging off a meeting; copied only for meetings this import inserted.
    MeetingChild,
    /// Standalone data merged by id, whatever meeting it mentions.
    Entity,
    /// FTS tables and their shadow tables (rebuilt by write triggers) and anything else inert.
    Skip,
}

use TableClass::{Entity, MachineBound, Meeting, MeetingChild, Skip};

/// Every table in today's schema. A test fails if a migration adds one that is missing here.
pub const CLASSIFICATION: &[(&str, TableClass)] = &[
    ("_sqlx_migrations", MachineBound),
    ("settings", MachineBound),
    ("transcript_settings", MachineBound),
    ("summary_prompts_meta", MachineBound),
    ("licensing", MachineBound),
    ("google_calendar_account", MachineBound),
    ("google_calendar_events", MachineBound),
    ("google_calendar_sync", MachineBound),
    ("meetings", Meeting),
    ("transcripts", MeetingChild),
    ("transcript_chunks", MeetingChild),
    ("transcript_speaker_overrides", MeetingChild),
    ("summary_processes", MeetingChild),
    ("meeting_briefs", MeetingChild),
    ("meeting_notes", MeetingChild),
    ("meeting_series_links", MeetingChild),
    ("meeting_participants", MeetingChild),
    ("speakers", MeetingChild),
    ("action_item_extractions", MeetingChild),
    ("topic_mention_scans", MeetingChild),
    ("people", Entity),
    ("voiceprints", Entity),
    ("owner_emails", Entity),
    ("topics", Entity),
    ("topic_links", Entity),
    ("topic_entries", Entity),
    ("topic_revisions", Entity),
    ("topic_suggestions", Entity),
    ("action_items", Entity),
    ("action_item_people", Entity),
    ("action_item_sources", Entity),
    ("summary_prompts", Entity),
    ("series_summary_prompts", Entity),
    ("saved_questions", Entity),
    ("ask_ai_history", Entity),
    ("attendee_photos", Entity),
    ("dismissed_calendar_events", Entity),
    ("transcripts_fts", Skip),
    ("transcripts_fts_config", Skip),
    ("transcripts_fts_data", Skip),
    ("transcripts_fts_docsize", Skip),
    ("transcripts_fts_idx", Skip),
    ("summaries_fts", Skip),
    ("summaries_fts_config", Skip),
    ("summaries_fts_data", Skip),
    ("summaries_fts_docsize", Skip),
    ("summaries_fts_idx", Skip),
    ("meeting_notes_fts", Skip),
    ("meeting_notes_fts_config", Skip),
    ("meeting_notes_fts_data", Skip),
    ("meeting_notes_fts_docsize", Skip),
    ("meeting_notes_fts_idx", Skip),
];

/// References the schema does not declare as foreign keys, read off the migrations (every
/// `*_id` / ref column): `(table, column, target_table, target_column)`. Declared FKs are read
/// from `pragma_foreign_key_list` at run time and unioned with these.
///
/// Deliberately absent: polymorphic columns (`topic_links.ref`, typed by `kind`), the ids inside
/// topic bodies (`*_decision_id`), and provenance columns the app already tolerates dangling
/// (`voiceprints.source_meeting_id`, `topic_entries.meeting_id`,
/// `topic_revisions.source_meeting_id`, `topic_suggestions.meeting_id`: each has a
/// `source_deleted_at`/nullable story), plus `meetings.*_id` (`template_id`,
/// `summary_prompt_id`: meetings are never dropped for a missing prompt).
pub const LOGICAL_REFS: &[(&str, &str, &str, &str)] = &[
    ("action_items", "meeting_id", "meetings", "id"),
    ("action_items", "assignee_person_id", "people", "id"),
    ("action_items", "topic_id", "topics", "id"),
    ("action_item_people", "action_item_id", "action_items", "id"),
    ("action_item_people", "person_id", "people", "id"),
    ("action_item_sources", "action_item_id", "action_items", "id"),
    ("action_item_sources", "meeting_id", "meetings", "id"),
    ("action_item_extractions", "meeting_id", "meetings", "id"),
    ("voiceprints", "person_id", "people", "id"),
    ("speakers", "person_id", "people", "id"),
    ("meeting_participants", "meeting_id", "meetings", "id"),
    ("meeting_participants", "person_id", "people", "id"),
    ("transcript_speaker_overrides", "meeting_id", "meetings", "id"),
    ("transcript_speaker_overrides", "transcript_id", "transcripts", "id"),
    ("topic_mention_scans", "meeting_id", "meetings", "id"),
    ("topic_entries", "topic_id", "topics", "id"),
    ("topic_revisions", "entry_id", "topic_entries", "id"),
    ("topic_suggestions", "topic_id", "topics", "id"),
    ("topic_suggestions", "target_entry_id", "topic_entries", "id"),
    ("topic_links", "topic_id", "topics", "id"),
    ("series_summary_prompts", "prompt_id", "summary_prompts", "id"),
];

/// What an Add import did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MergeCounts {
    /// Meetings inserted.
    pub meetings: u32,
    pub people: u32,
    pub topics: u32,
    pub action_items: u32,
    /// How many of the backup's meetings were already on this Mac (same id), so left alone.
    pub already_here: u32,
    /// Ids of the inserted meetings.
    pub new_meeting_ids: Vec<String>,
}

pub fn class_of(table: &str) -> Option<TableClass> {
    CLASSIFICATION.iter().find(|(n, _)| *n == table).map(|(_, c)| *c)
}

fn q(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Ref {
    table: String,
    column: String,
    target_table: String,
    target_column: String,
}

/// `(table, class)` for every table to merge, in `sqlite_master` order. A table that is not
/// classified aborts the import before anything is written.
async fn mergeable_tables(
    conn: &mut SqliteConnection,
) -> Result<Vec<(String, TableClass)>, BackupError> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM main.sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for n in names {
        match class_of(&n) {
            None => {
                return Err(BackupError::Db(format!(
                    "table `{n}` is not classified for Add import"
                )))
            }
            Some(MachineBound | Skip) => {}
            Some(c) => out.push((n, c)),
        }
    }
    Ok(out)
}

/// Declared foreign keys ∪ [`LOGICAL_REFS`], restricted to the tables being merged.
async fn collect_refs(
    conn: &mut SqliteConnection,
    tables: &[(String, TableClass)],
) -> Result<Vec<Ref>, BackupError> {
    let mut refs: BTreeSet<Ref> = BTreeSet::new();
    for (t, _) in tables {
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT \"from\", \"table\", \"to\" FROM pragma_foreign_key_list(?, 'main')",
        )
        .bind(t)
        .fetch_all(&mut *conn)
        .await?;
        for (column, target_table, to) in rows {
            let target_column = match to {
                Some(c) => c,
                None => sqlx::query_scalar(
                    "SELECT name FROM pragma_table_info(?, 'main') WHERE pk = 1",
                )
                .bind(&target_table)
                .fetch_one(&mut *conn)
                .await?,
            };
            refs.insert(Ref { table: t.clone(), column, target_table, target_column });
        }
    }
    for (t, c, tt, tc) in LOGICAL_REFS {
        refs.insert(Ref {
            table: (*t).into(),
            column: (*c).into(),
            target_table: (*tt).into(),
            target_column: (*tc).into(),
        });
    }
    let known: BTreeSet<&str> = tables.iter().map(|(t, _)| t.as_str()).collect();
    Ok(refs
        .into_iter()
        .filter(|r| known.contains(r.table.as_str()) && known.contains(r.target_table.as_str()))
        .collect())
}

/// Topological order over the refs (self-references ignored); `meetings` first, then names.
fn insert_order(
    tables: &[(String, TableClass)],
    refs: &[Ref],
) -> Result<Vec<(String, TableClass)>, BackupError> {
    let mut deps: HashMap<&str, BTreeSet<&str>> =
        tables.iter().map(|(t, _)| (t.as_str(), BTreeSet::new())).collect();
    for r in refs {
        if r.table != r.target_table {
            if let Some(d) = deps.get_mut(r.table.as_str()) {
                d.insert(r.target_table.as_str());
            }
        }
    }
    for (t, c) in tables {
        if *c == MeetingChild {
            if let Some(d) = deps.get_mut(t.as_str()) {
                d.insert("meetings");
            }
        }
    }
    let mut done: BTreeSet<&str> = BTreeSet::new();
    let mut order = Vec::new();
    while order.len() < tables.len() {
        let next = tables
            .iter()
            .map(|(t, _)| t.as_str())
            .filter(|t| !done.contains(t) && deps[t].iter().all(|d| done.contains(d)))
            .min_by_key(|t| (*t != "meetings", *t));
        let Some(t) = next else {
            return Err(BackupError::Db("foreign keys form a cycle".into()));
        };
        done.insert(t);
        let class = tables.iter().find(|(n, _)| n == t).map(|(_, c)| *c).unwrap_or(Entity);
        order.push((t.to_string(), class));
    }
    Ok(order)
}

fn insert_sql(table: &str, class: TableClass, cols: &[String], refs: &[&Ref]) -> String {
    let col_list = cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
    let sel_list = cols.iter().map(|c| format!("b.{}", q(c))).collect::<Vec<_>>().join(", ");
    let mut conds: Vec<String> = Vec::new();
    match class {
        Meeting => conds.push("b.id NOT IN (SELECT id FROM main.meetings)".into()),
        MeetingChild => {
            conds.push("b.meeting_id IN (SELECT id FROM temp.new_meetings)".into())
        }
        _ => {}
    }
    for r in refs {
        if class == MeetingChild && r.column == "meeting_id" {
            continue; // covered by the scope above
        }
        let col = format!("b.{}", q(&r.column));
        conds.push(format!(
            "({col} IS NULL OR EXISTS (SELECT 1 FROM main.{} WHERE {} = {col}))",
            q(&r.target_table),
            q(&r.target_column)
        ));
    }
    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };
    format!(
        "INSERT OR IGNORE INTO main.{t} ({col_list}) SELECT {sel_list} FROM backup.{t} AS b{where_sql}",
        t = q(table)
    )
}

/// `ATTACH` the staged DB as `backup`, then `BEGIN IMMEDIATE`. Pair with [`end`].
pub async fn begin(conn: &mut SqliteConnection, staged_db: &Path) -> Result<(), BackupError> {
    sqlx::query("ATTACH DATABASE ? AS backup")
        .bind(staged_db.to_string_lossy().into_owned())
        .execute(&mut *conn)
        .await?;
    if let Err(e) = sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await {
        let _ = sqlx::query("DETACH DATABASE backup").execute(&mut *conn).await;
        return Err(e.into());
    }
    Ok(())
}

/// Test hook: while a path fragment is listed, `end` pretends `DETACH` failed for an attached
/// DB whose file path contains it (keyed so parallel tests do not interfere).
#[doc(hidden)]
pub static FAIL_DETACH_FOR_PATHS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

async fn detach_forced_to_fail(conn: &mut SqliteConnection) -> bool {
    let frags = FAIL_DETACH_FOR_PATHS.lock().map(|g| g.clone()).unwrap_or_default();
    if frags.is_empty() {
        return false;
    }
    let file: Option<String> =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name = 'backup'")
            .fetch_optional(&mut *conn)
            .await
            .ok()
            .flatten();
    file.is_some_and(|f| frags.iter().any(|x| f.contains(x.as_str())))
}

/// `COMMIT` or `ROLLBACK`, drop the temp tables, `DETACH`. Detach and cleanup run even if the
/// commit fails (after a rollback). `Err` means the transaction did NOT commit. `Ok(true)` means
/// the connection is clean; `Ok(false)` means the commit (or rollback) succeeded but `DETACH`
/// did not, so the connection still has `backup` attached and must not go back to the pool.
pub async fn end(conn: &mut SqliteConnection, commit: bool) -> Result<bool, BackupError> {
    let mut result = Ok(());
    if commit {
        if let Err(e) = sqlx::query("COMMIT").execute(&mut *conn).await {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            result = Err(BackupError::from(e));
        }
    } else {
        let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    }
    for t in ["new_meetings", "new_action_items", "new_topics"] {
        let _ = sqlx::query(&format!("DROP TABLE IF EXISTS temp.{t}")).execute(&mut *conn).await;
    }
    let forced = detach_forced_to_fail(conn).await;
    let detached = !forced
        && sqlx::query("DETACH DATABASE backup").execute(&mut *conn).await.is_ok();
    result?;
    Ok(detached)
}

/// Run every insert inside the transaction [`begin`] opened. Does not commit.
pub async fn insert_missing(conn: &mut SqliteConnection) -> Result<MergeCounts, BackupError> {
    let tables = mergeable_tables(conn).await?;
    let refs = collect_refs(conn, &tables).await?;
    let order = insert_order(&tables, &refs)?;

    let already_here: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM backup.meetings WHERE id IN (SELECT id FROM main.meetings)",
    )
    .fetch_one(&mut *conn)
    .await?;

    // Ids this import may add, and where their sort_order should start.
    for (tmp, src) in [
        ("new_meetings", "meetings"),
        ("new_action_items", "action_items"),
        ("new_topics", "topics"),
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS temp.{tmp}")).execute(&mut *conn).await?;
        sqlx::query(&format!(
            "CREATE TEMP TABLE {tmp} AS SELECT id FROM backup.{s} WHERE id NOT IN (SELECT id FROM main.{s})",
            s = q(src)
        ))
        .execute(&mut *conn)
        .await?;
    }
    let mut base: HashMap<&str, i64> = HashMap::new();
    for t in ["action_items", "topics"] {
        let next: i64 =
            sqlx::query_scalar(&format!("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM main.{t}"))
                .fetch_one(&mut *conn)
                .await?;
        base.insert(t, next);
    }

    let mut counts = MergeCounts { already_here: already_here as u32, ..Default::default() };
    for (table, class) in &order {
        let cols: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?, 'main') ORDER BY cid")
                .bind(table)
                .fetch_all(&mut *conn)
                .await?;
        let own: Vec<&Ref> = refs.iter().filter(|r| &r.table == table).collect();
        let inserted = sqlx::query(&insert_sql(table, *class, &cols, &own))
            .execute(&mut *conn)
            .await?
            .rows_affected() as u32;
        match table.as_str() {
            "meetings" => {
                // Whatever OR IGNORE dropped is not new.
                sqlx::query("DELETE FROM temp.new_meetings WHERE id NOT IN (SELECT id FROM main.meetings)")
                    .execute(&mut *conn)
                    .await?;
                counts.meetings = inserted;
            }
            "people" => counts.people = inserted,
            "topics" => counts.topics = inserted,
            "action_items" => counts.action_items = inserted,
            _ => {}
        }
    }

    for t in ["action_items", "topics"] {
        let ids: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT b.id FROM backup.{t} AS b WHERE b.id IN (SELECT id FROM temp.new_{t}) \
             AND b.id IN (SELECT id FROM main.{t}) \
             ORDER BY b.sort_order IS NULL, b.sort_order, b.rowid"
        ))
        .fetch_all(&mut *conn)
        .await?;
        for (i, id) in ids.iter().enumerate() {
            sqlx::query(&format!("UPDATE main.{t} SET sort_order = ? WHERE id = ?"))
                .bind(base[t] + i as i64)
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }
    }

    counts.new_meeting_ids =
        sqlx::query_scalar("SELECT id FROM temp.new_meetings ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    Ok(counts)
}

/// Attach, insert everything missing, commit, detach. A failure rolls everything back.
pub async fn merge_add_missing(
    conn: &mut SqliteConnection,
    staged_db: &Path,
) -> Result<MergeCounts, BackupError> {
    begin(conn, staged_db).await?;
    match insert_missing(conn).await {
        Ok(counts) => {
            end(conn, true).await?; // committed; a dirty connection is the caller's to drop
            Ok(counts)
        }
        Err(e) => {
            let _ = end(conn, false).await;
            Err(e)
        }
    }
}
