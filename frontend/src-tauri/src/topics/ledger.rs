//! The topic ledger (specs/0089): entries, revisions, suggestions.
//!
//! Immutability rule: machines only INSERT (entries, suggestions). The owner alone changes
//! entry text, and only through `edit_entry` (body only), `revert_entry` (body := original),
//! `rewrite_entry` (replaces body AND `ai_body_json`, only while unedited) and
//! `resolve_suggestion` (accept-merge appends one bullet to body AND `ai_body_json`,
//! deviation 4; existing bullets are never altered). No other path assigns `body_json` or
//! `ai_body_json`. The unit test at the bottom scans all of `src/` to keep it that way.

use anyhow::{anyhow, Result};
use chrono::Utc;
use sqlx::{FromRow, SqliteConnection, SqlitePool};
use std::collections::HashMap;

use super::limits::SUGGESTION_TEXT_MAX;
use super::model::{Bullet, Decision, EntryBody, TopicEntry, TopicRevision, TopicSuggestion};

#[derive(Debug, Clone)]
pub struct NewSuggestion {
    pub topic_id: String,
    pub kind: String,
    pub meeting_id: String,
    pub t_start: Option<f64>,
    pub target_entry_id: Option<String>,
    pub target_decision_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolveOutcome {
    pub topic_id: String,
    pub entry_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenDecision {
    pub entry_id: String,
    pub decision_id: String,
    pub text: String,
    pub occurred_at: String,
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

fn to_json(body: &EntryBody) -> Result<String> {
    Ok(serde_json::to_string(body)?)
}

fn from_json(s: &str) -> EntryBody {
    serde_json::from_str(s).unwrap_or_default()
}

/// Mint a fresh id for every decision, ignoring whatever came in.
fn mint_all(mut body: EntryBody) -> EntryBody {
    for d in &mut body.decisions {
        d.id = new_id("dec");
    }
    body
}

/// Keep ids the entry already has; mint the rest (the owner added a decision).
fn keep_known(mut body: EntryBody, existing: &EntryBody) -> EntryBody {
    for d in &mut body.decisions {
        if !existing.decisions.iter().any(|e| e.id == d.id) {
            d.id = new_id("dec");
        }
    }
    body
}

fn from_json_strict(s: &str) -> Result<EntryBody> {
    serde_json::from_str(s)
        .map_err(|_| anyhow!("This entry's saved text couldn't be read, so nothing was changed."))
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// ----- machine writers (insert only) -----

/// Insert a machine entry. `None` when the topic isn't open, the meeting is gone, or the
/// (topic, meeting) entry already exists. The checks live inside the one statement.
pub async fn insert_machine_entry(
    pool: &SqlitePool,
    topic_id: &str,
    meeting_id: &str,
    kind: &str,
    body: EntryBody,
) -> Result<Option<String>> {
    let id = new_id("te");
    let json = to_json(&mint_all(body))?;
    let res = sqlx::query(
        "INSERT INTO topic_entries
            (id, topic_id, meeting_id, kind, occurred_at, ai_body_json, body_json, created_at)
         SELECT ?1, t.id, m.id, ?4, m.created_at, ?5, ?5, ?6
         FROM topics t, meetings m
         WHERE t.id = ?2 AND t.status = 'open' AND m.id = ?3
         ON CONFLICT DO NOTHING",
    )
    .bind(&id)
    .bind(topic_id)
    .bind(meeting_id)
    .bind(kind)
    .bind(&json)
    .bind(now())
    .execute(pool)
    .await?;
    Ok((res.rows_affected() > 0).then_some(id))
}

/// Insert pending suggestions; returns how many were stored. A `note` is skipped when the
/// topic already has an entry or any suggestion (any status) from that meeting, so a
/// dismissal never comes back. The blocked set is decided before the batch, so one batch may
/// carry several notes for the same (topic, meeting).
pub async fn insert_suggestions(pool: &SqlitePool, rows: &[NewSuggestion]) -> Result<u32> {
    let mut tx = pool.begin().await?;
    let mut blocked: HashMap<(String, String), bool> = HashMap::new();
    let mut stored = 0u32;
    for r in rows {
        if r.kind == "note" {
            let key = (r.topic_id.clone(), r.meeting_id.clone());
            if !blocked.contains_key(&key) {
                let hit: i64 = sqlx::query_scalar(
                    "SELECT (SELECT COUNT(*) FROM topic_entries WHERE topic_id = ?1 AND meeting_id = ?2)
                          + (SELECT COUNT(*) FROM topic_suggestions WHERE topic_id = ?1 AND meeting_id = ?2)",
                )
                .bind(&r.topic_id)
                .bind(&r.meeting_id)
                .fetch_one(&mut *tx)
                .await?;
                blocked.insert(key.clone(), hit > 0);
            }
            if blocked[&key] {
                continue;
            }
        }
        let res = sqlx::query(
            "INSERT INTO topic_suggestions
                (id, topic_id, kind, meeting_id, t_start, target_entry_id, target_decision_id,
                 text, status, created_at)
             SELECT ?1, t.id, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9
             FROM topics t WHERE t.id = ?2 AND t.status = 'open'",
        )
        .bind(new_id("ts"))
        .bind(&r.topic_id)
        .bind(&r.kind)
        .bind(&r.meeting_id)
        .bind(r.t_start)
        .bind(&r.target_entry_id)
        .bind(&r.target_decision_id)
        .bind(truncate_chars(&r.text, SUGGESTION_TEXT_MAX))
        .bind(now())
        .execute(&mut *tx)
        .await?;
        stored += res.rows_affected() as u32;
    }
    tx.commit().await?;
    Ok(stored)
}

// ----- owner writers -----

pub async fn edit_entry(pool: &SqlitePool, entry_id: &str, body: EntryBody) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let cur: Option<String> =
        sqlx::query_scalar("SELECT body_json FROM topic_entries WHERE id = ?")
            .bind(entry_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(cur) = cur else { return Ok(false) };
    let json = to_json(&keep_known(body, &from_json(&cur)))?;
    sqlx::query("UPDATE topic_entries SET body_json = ?2, edited_at = ?3 WHERE id = ?1")
        .bind(entry_id)
        .bind(json)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

/// Restore the machine version. `false` for a missing or manual entry (no original).
pub async fn revert_entry(pool: &SqlitePool, entry_id: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE topic_entries SET body_json = ai_body_json, edited_at = NULL
         WHERE id = ? AND ai_body_json IS NOT NULL",
    )
    .bind(entry_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Replace both bodies with a regenerated one, only while the owner hasn't edited it and
/// only for a `series` entry: a `mention` entry holds notes the owner accepted, which the
/// linked-meeting writer would replace.
pub async fn rewrite_entry(pool: &SqlitePool, entry_id: &str, body: EntryBody) -> Result<bool> {
    let json = to_json(&mint_all(body))?;
    let res = sqlx::query(
        "UPDATE topic_entries SET body_json = ?2, ai_body_json = ?2
         WHERE id = ?1 AND kind = 'series' AND edited_at IS NULL AND ai_body_json IS NOT NULL",
    )
    .bind(entry_id)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn add_manual_entry(
    pool: &SqlitePool,
    topic_id: &str,
    body: EntryBody,
) -> Result<String> {
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM topics WHERE id = ?")
        .bind(topic_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(anyhow!("That topic no longer exists."));
    }
    let id = new_id("te");
    let json = to_json(&mint_all(body))?;
    let t = now();
    sqlx::query(
        "INSERT INTO topic_entries
            (id, topic_id, meeting_id, kind, occurred_at, ai_body_json, body_json, created_at)
         VALUES (?1, ?2, NULL, 'manual', ?3, NULL, ?4, ?3)",
    )
    .bind(&id)
    .bind(topic_id)
    .bind(&t)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn delete_entry(pool: &SqlitePool, entry_id: &str) -> Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM topic_revisions WHERE entry_id = ?")
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
    let res = sqlx::query("DELETE FROM topic_entries WHERE id = ?")
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

#[derive(FromRow)]
struct SuggestionRow {
    topic_id: String,
    kind: String,
    meeting_id: String,
    t_start: Option<f64>,
    target_entry_id: Option<String>,
    target_decision_id: Option<String>,
    text: String,
}

async fn finish(tx: &mut SqliteConnection, id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE topic_suggestions SET status = ?2, resolved_at = ?3 WHERE id = ?1")
        .bind(id)
        .bind(status)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

pub async fn resolve_suggestion(
    pool: &SqlitePool,
    id: &str,
    accept: bool,
) -> Result<ResolveOutcome> {
    let mut tx = pool.begin().await?;
    let row: Option<SuggestionRow> = sqlx::query_as(
        "SELECT topic_id, kind, meeting_id, t_start, target_entry_id, target_decision_id, text
         FROM topic_suggestions WHERE id = ? AND status = 'pending'",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(s) = row else {
        return Err(anyhow!("That suggestion was already handled."));
    };
    let mut out = ResolveOutcome {
        topic_id: s.topic_id.clone(),
        entry_id: None,
    };
    if !accept {
        finish(&mut tx, id, "dismissed").await?;
        tx.commit().await?;
        return Ok(out);
    }
    if s.kind == "revision" {
        let target = match &s.target_entry_id {
            Some(t) => {
                sqlx::query_scalar::<_, String>("SELECT id FROM topic_entries WHERE id = ?")
                    .bind(t)
                    .fetch_optional(&mut *tx)
                    .await?
            }
            None => None,
        };
        let Some(entry_id) = target else {
            finish(&mut tx, id, "dismissed").await?;
            tx.commit().await?;
            return Err(anyhow!("That entry was deleted."));
        };
        sqlx::query(
            "INSERT INTO topic_revisions
                (id, entry_id, decision_id, text, source_meeting_id, t_start, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id("tr"))
        .bind(&entry_id)
        .bind(&s.target_decision_id)
        .bind(&s.text)
        .bind(&s.meeting_id)
        .bind(s.t_start)
        .bind(now())
        .execute(&mut *tx)
        .await?;
        out.entry_id = Some(entry_id);
    } else {
        let bullet = Bullet {
            text: s.text.clone(),
            t: s.t_start,
        };
        let existing: Option<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, body_json, ai_body_json FROM topic_entries
             WHERE topic_id = ? AND meeting_id = ?",
        )
        .bind(&s.topic_id)
        .bind(&s.meeting_id)
        .fetch_optional(&mut *tx)
        .await?;
        let entry_id = if let Some((eid, body, ai)) = existing {
            let mut b = from_json_strict(&body)?;
            b.bullets.push(bullet.clone());
            let ai_json = ai.map(|a| {
                let mut x = from_json_strict(&a)?;
                x.bullets.push(bullet);
                to_json(&x)
            });
            let ai_json = ai_json.transpose()?;
            sqlx::query(
                "UPDATE topic_entries SET body_json = ?2, ai_body_json = COALESCE(?3, ai_body_json)
                 WHERE id = ?1",
            )
            .bind(&eid)
            .bind(to_json(&b)?)
            .bind(ai_json)
            .execute(&mut *tx)
            .await?;
            eid
        } else {
            let eid = new_id("te");
            let json = to_json(&EntryBody {
                bullets: vec![bullet],
                decisions: vec![],
            })?;
            let t = now();
            sqlx::query(
                "INSERT INTO topic_entries
                    (id, topic_id, meeting_id, kind, occurred_at, ai_body_json, body_json, created_at)
                 VALUES (?1, ?2, ?3, 'mention',
                         COALESCE((SELECT created_at FROM meetings WHERE id = ?3), ?5), ?4, ?4, ?5)",
            )
            .bind(&eid)
            .bind(&s.topic_id)
            .bind(&s.meeting_id)
            .bind(json)
            .bind(t)
            .execute(&mut *tx)
            .await?;
            eid
        };
        out.entry_id = Some(entry_id);
    }
    finish(&mut tx, id, "accepted").await?;
    tx.commit().await?;
    Ok(out)
}

// ----- reads -----

#[derive(FromRow)]
struct EntryRow {
    id: String,
    topic_id: String,
    meeting_id: Option<String>,
    meeting_title: Option<String>,
    kind: String,
    occurred_at: String,
    ai_body_json: Option<String>,
    body_json: String,
    edited_at: Option<String>,
    source_deleted_at: Option<String>,
    created_at: String,
}

type RevRow = (
    String,
    Option<String>,
    String,
    Option<String>,
    Option<f64>,
    Option<String>,
    String,
    String,
);

pub async fn entries_for_topic(pool: &SqlitePool, topic_id: &str) -> Result<Vec<TopicEntry>> {
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT e.id, e.topic_id, e.meeting_id, m.title AS meeting_title, e.kind, e.occurred_at,
                e.ai_body_json, e.body_json, e.edited_at, e.source_deleted_at, e.created_at
         FROM topic_entries e LEFT JOIN meetings m ON m.id = e.meeting_id
         WHERE e.topic_id = ?
         ORDER BY e.occurred_at DESC, e.created_at DESC, e.id",
    )
    .bind(topic_id)
    .fetch_all(pool)
    .await?;
    let revs: Vec<RevRow> = sqlx::query_as(
        "SELECT r.entry_id, r.decision_id, r.text, r.source_meeting_id, r.t_start,
                    r.source_deleted_at, r.created_at, r.id
             FROM topic_revisions r JOIN topic_entries e ON e.id = r.entry_id
             WHERE e.topic_id = ? ORDER BY r.created_at, r.id",
    )
    .bind(topic_id)
    .fetch_all(pool)
    .await?;
    let mut by_entry: HashMap<String, Vec<TopicRevision>> = HashMap::new();
    for (
        entry_id,
        decision_id,
        text,
        source_meeting_id,
        t_start,
        source_deleted_at,
        created_at,
        id,
    ) in revs
    {
        by_entry
            .entry(entry_id.clone())
            .or_default()
            .push(TopicRevision {
                id,
                entry_id,
                decision_id,
                text,
                source_meeting_id,
                t_start,
                source_deleted_at,
                created_at,
            });
    }
    Ok(rows
        .into_iter()
        .map(|r| TopicEntry {
            revisions: by_entry.remove(&r.id).unwrap_or_default(),
            id: r.id,
            topic_id: r.topic_id,
            meeting_id: r.meeting_id,
            meeting_title: r.meeting_title,
            kind: r.kind,
            occurred_at: r.occurred_at,
            ai_body: r.ai_body_json.as_deref().map(from_json),
            body: from_json(&r.body_json),
            edited_at: r.edited_at,
            source_deleted_at: r.source_deleted_at,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn pending_suggestions(
    pool: &SqlitePool,
    topic_id: &str,
) -> Result<Vec<TopicSuggestion>> {
    #[derive(FromRow)]
    struct Row {
        id: String,
        topic_id: String,
        kind: String,
        meeting_id: String,
        meeting_title: Option<String>,
        t_start: Option<f64>,
        target_entry_id: Option<String>,
        target_decision_id: Option<String>,
        target_occurred_at: Option<String>,
        target_body: Option<String>,
        text: String,
        status: String,
        created_at: String,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT s.id, s.topic_id, s.kind, s.meeting_id, m.title AS meeting_title, s.t_start,
                s.target_entry_id, s.target_decision_id, e.occurred_at AS target_occurred_at,
                e.body_json AS target_body, s.text, s.status, s.created_at
         FROM topic_suggestions s
         LEFT JOIN meetings m ON m.id = s.meeting_id
         LEFT JOIN topic_entries e ON e.id = s.target_entry_id
         WHERE s.topic_id = ? AND s.status = 'pending'
         ORDER BY s.created_at, s.id",
    )
    .bind(topic_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let target_decision_text = match (&r.target_body, &r.target_decision_id) {
                (Some(b), Some(d)) => from_json(b)
                    .decisions
                    .into_iter()
                    .find(|x: &Decision| &x.id == d)
                    .map(|x| x.text),
                _ => None,
            };
            TopicSuggestion {
                id: r.id,
                topic_id: r.topic_id,
                kind: r.kind,
                meeting_id: r.meeting_id,
                meeting_title: r.meeting_title,
                t_start: r.t_start,
                target_entry_id: r.target_entry_id,
                target_decision_id: r.target_decision_id,
                target_decision_text,
                target_occurred_at: r.target_occurred_at,
                text: r.text,
                status: r.status,
                created_at: r.created_at,
            }
        })
        .collect())
}

/// Decisions from `body_json` (the owner's corrected text), newest entry first.
pub async fn open_decisions(
    pool: &SqlitePool,
    topic_id: &str,
    cap: usize,
) -> Result<Vec<OpenDecision>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, occurred_at, body_json FROM topic_entries WHERE topic_id = ?
         ORDER BY occurred_at DESC, created_at DESC, id",
    )
    .bind(topic_id)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::new();
    'outer: for (entry_id, occurred_at, body) in rows {
        for d in from_json(&body).decisions {
            if out.len() >= cap {
                break 'outer;
            }
            out.push(OpenDecision {
                entry_id: entry_id.clone(),
                decision_id: d.id,
                text: d.text,
                occurred_at: occurred_at.clone(),
            });
        }
    }
    Ok(out)
}

pub async fn has_entry(pool: &SqlitePool, topic_id: &str, meeting_id: &str) -> Result<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM topic_entries WHERE topic_id = ? AND meeting_id = ?",
    )
    .bind(topic_id)
    .bind(meeting_id)
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use regex::Regex;
    use std::path::{Path, PathBuf};

    const ENTRY_WRITERS: [&str; 4] = [
        "edit_entry",
        "revert_entry",
        "rewrite_entry",
        "resolve_suggestion",
    ];

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    struct Stmt {
        fn_name: String,
        /// Text that assigns columns: the SET clause of an UPDATE, or what follows
        /// `DO UPDATE SET` in an upsert.
        assigns: String,
        /// `INSERT OR REPLACE` / `REPLACE INTO`: rewrites a whole row.
        replaces: bool,
    }

    /// Every SQL statement in `src` that writes `table` other than a plain INSERT. A
    /// statement runs from its keyword to the next double quote (the literal's end). The whole
    /// file is scanned, matching is case-insensitive and tolerant of any whitespace.
    fn writes(src: &str, table: &str) -> Vec<Stmt> {
        let kw = Regex::new(&format!(
            r"(?i)\b(update\s+{table}\b|(?:insert\s+or\s+replace\s+into|replace\s+into|insert\s+into)\s+{table}\b)"
        ))
        .unwrap();
        let upsert = Regex::new(r"(?i)do\s+update\s+set").unwrap();
        let where_ = Regex::new(r"(?i)\bwhere\b").unwrap();
        let mut out = Vec::new();
        for m in kw.find_iter(src) {
            let rest = &src[m.start()..];
            let stmt = &rest[..rest.find('"').unwrap_or(rest.len())];
            let head = m.as_str().to_lowercase();
            let assigns = if head.starts_with("update") {
                let set = &stmt[m.len()..];
                set[..where_.find(set).map_or(set.len(), |w| w.start())].to_string()
            } else {
                upsert.find(stmt).map_or(String::new(), |u| {
                    let set = &stmt[u.end()..];
                    set[..where_.find(set).map_or(set.len(), |w| w.start())].to_string()
                })
            };
            let fn_name = src[..m.start()]
                .rfind("fn ")
                .map(|i| {
                    src[i + 3..]
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect()
                })
                .unwrap_or_default();
            out.push(Stmt {
                fn_name,
                assigns,
                replaces: head.contains("replace"),
            });
        }
        out
    }

    fn assigns(set: &str, col: &str) -> bool {
        set.match_indices(col)
            .any(|(i, _)| set[i + col.len()..].trim_start().starts_with('='))
    }

    #[test]
    fn only_named_fns_write_topic_entries() {
        let mut files = Vec::new();
        rs_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut seen_writers = 0;
        for f in files {
            let src = std::fs::read_to_string(&f).unwrap();
            let is_ledger = f.ends_with("topics/ledger.rs");
            let is_repo = f.ends_with("topics/repo.rs");
            for st in writes(&src, "topic_entries") {
                assert!(
                    !st.replaces,
                    "{} fn `{}` replaces whole topic_entries rows",
                    f.display(),
                    st.fn_name
                );
                if assigns(&st.assigns, "body_json") {
                    assert!(
                        is_ledger && ENTRY_WRITERS.contains(&st.fn_name.as_str()),
                        "{} fn `{}` assigns an entry body; only {ENTRY_WRITERS:?} in ledger.rs may",
                        f.display(),
                        st.fn_name
                    );
                    seen_writers += 1;
                }
            }
            for st in writes(&src, "topics") {
                assert!(
                    !st.replaces,
                    "{} fn `{}` replaces whole topics rows",
                    f.display(),
                    st.fn_name
                );
                if assigns(&st.assigns, "notes_markdown") || assigns(&st.assigns, "notes_json") {
                    assert!(
                        is_repo && st.fn_name == "set_notes",
                        "{} fn `{}` assigns topic notes; only repo.rs set_notes may",
                        f.display(),
                        st.fn_name
                    );
                }
            }
        }
        assert!(
            seen_writers >= 4,
            "scan found {seen_writers} writers; the scan is broken"
        );
    }
}
