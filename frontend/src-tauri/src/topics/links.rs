//! Topic links and membership (specs/0089). A topic collects meetings through links:
//! a single meeting, a calendar/manual series, or a bare normalized title. Membership is one
//! SQL query that mirrors the three-arm union in `find_prior_series_occurrences`, so a topic
//! linked to a series sees exactly the meetings the series' prep brief would. Linking never
//! writes `meeting_series_links` (that table stays owned by the series feature).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::limits::BACKFILL_DAYS;
use super::model::Topic;
use crate::database::repositories::meeting::{normalize_title, MeetingsRepository};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Series,
    Title,
    Meeting,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkKind::Series => "series",
            LinkKind::Title => "title",
            LinkKind::Meeting => "meeting",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "series" => Some(LinkKind::Series),
            "title" => Some(LinkKind::Title),
            "meeting" => Some(LinkKind::Meeting),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TopicLink {
    pub topic_id: String,
    pub kind: LinkKind,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub title: Option<String>,
    /// Display name: the series' newest meeting title, or the meeting's own title.
    pub label: String,
}

/// Does meeting `m` (with optional `l` = its `meeting_series_links` row) match link `tl`?
/// The series arm is the SQL of `find_prior_series_occurrences` with the key/title taken
/// from the link row; title folding is `LOWER(TRIM(..))` on both operands.
const MATCHES_LINK: &str = "( \
    (tl.kind = 'meeting' AND tl.ref = m.id) \
    OR (tl.kind = 'series' AND ( m.calendar_series_key = tl.ref OR l.series_key = tl.ref \
        OR ( TRIM(COALESCE(tl.title, '')) <> '' AND LOWER(TRIM(m.title)) = LOWER(TRIM(tl.title)) \
             AND m.calendar_series_key IS NULL \
             AND (l.series_key IS NULL OR l.series_key = tl.ref) ) )) \
    OR (tl.kind = 'title' AND TRIM(tl.ref) <> '' AND LOWER(TRIM(m.title)) = LOWER(TRIM(tl.ref))) \
)";

async fn meeting_title(pool: &SqlitePool, meeting_id: &str) -> Result<String> {
    sqlx::query_scalar::<_, String>("SELECT title FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("That meeting no longer exists."))
}

async fn insert_link(
    pool: &SqlitePool,
    topic_id: &str,
    kind: LinkKind,
    ref_: &str,
    title: Option<&str>,
) -> Result<()> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM topics WHERE id = ?")
        .bind(topic_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(anyhow!("That topic no longer exists."));
    }
    sqlx::query(
        "INSERT OR IGNORE INTO topic_links (topic_id, kind, ref, title, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(topic_id)
    .bind(kind.as_str())
    .bind(ref_)
    .bind(title)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// The newest member meeting's title for a series/title link, else the stored title.
async fn label_for(pool: &SqlitePool, link: &TopicLink) -> Result<String> {
    if link.kind == LinkKind::Meeting {
        return Ok(meeting_title(pool, &link.ref_)
            .await
            .unwrap_or_else(|_| link.ref_.clone()));
    }
    let newest: Option<String> = sqlx::query_scalar(&format!(
        "SELECT m.title FROM topic_links tl, meetings m
         LEFT JOIN meeting_series_links l ON l.meeting_id = m.id
         WHERE tl.topic_id = ?1 AND tl.kind = ?2 AND tl.ref = ?3 AND {MATCHES_LINK}
         ORDER BY m.created_at DESC LIMIT 1"
    ))
    .bind(&link.topic_id)
    .bind(link.kind.as_str())
    .bind(&link.ref_)
    .fetch_optional(pool)
    .await?;
    Ok(newest
        .or_else(|| link.title.clone())
        .unwrap_or_else(|| link.ref_.clone()))
}

async fn build(
    pool: &SqlitePool,
    topic_id: &str,
    kind: LinkKind,
    ref_: &str,
    title: Option<String>,
) -> Result<TopicLink> {
    let mut link = TopicLink {
        topic_id: topic_id.to_string(),
        kind,
        ref_: ref_.to_string(),
        title,
        label: String::new(),
    };
    link.label = label_for(pool, &link).await?;
    Ok(link)
}

pub async fn link_meeting(
    pool: &SqlitePool,
    topic_id: &str,
    meeting_id: &str,
) -> Result<TopicLink> {
    meeting_title(pool, meeting_id).await?;
    insert_link(pool, topic_id, LinkKind::Meeting, meeting_id, None).await?;
    build(pool, topic_id, LinkKind::Meeting, meeting_id, None).await
}

/// Link the series `meeting_id` belongs to: its effective key when it has one, else its
/// normalized title. Never writes `meeting_series_links`.
pub async fn link_series_of(
    pool: &SqlitePool,
    topic_id: &str,
    meeting_id: &str,
) -> Result<TopicLink> {
    let title = normalize_title(&meeting_title(pool, meeting_id).await?);
    let key = MeetingsRepository::resolve_effective_series_key(pool, meeting_id).await?;
    let (kind, ref_, stored) = match key {
        Some(k) => (
            LinkKind::Series,
            k,
            Some(title.clone()).filter(|t| !t.is_empty()),
        ),
        None if !title.is_empty() => (LinkKind::Title, title, None),
        None => return Err(anyhow!("This meeting isn't part of a series.")),
    };
    insert_link(pool, topic_id, kind, &ref_, stored.as_deref()).await?;
    build(pool, topic_id, kind, &ref_, stored).await
}

pub async fn unlink(pool: &SqlitePool, topic_id: &str, kind: LinkKind, ref_: &str) -> Result<bool> {
    let done = sqlx::query("DELETE FROM topic_links WHERE topic_id = ? AND kind = ? AND ref = ?")
        .bind(topic_id)
        .bind(kind.as_str())
        .bind(ref_)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn links_for_topic(pool: &SqlitePool, topic_id: &str) -> Result<Vec<TopicLink>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT kind, ref, title FROM topic_links WHERE topic_id = ? ORDER BY created_at, kind, ref",
    )
    .bind(topic_id)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (kind, ref_, title) in rows {
        if let Some(kind) = LinkKind::parse(&kind) {
            out.push(build(pool, topic_id, kind, &ref_, title).await?);
        }
    }
    Ok(out)
}

/// Topics any of whose links match this meeting, by name. One query.
pub async fn topics_for_meeting(
    pool: &SqlitePool,
    meeting_id: &str,
    open_only: bool,
) -> Result<Vec<Topic>> {
    Ok(sqlx::query_as::<_, Topic>(&format!(
        "SELECT DISTINCT t.id, t.name, t.description, t.status, t.notes_markdown, t.notes_json,
                t.created_at, t.updated_at, t.closed_at
         FROM topics t
         JOIN topic_links tl ON tl.topic_id = t.id
         JOIN meetings m ON m.id = ?1
         LEFT JOIN meeting_series_links l ON l.meeting_id = m.id
         WHERE (?2 = 0 OR t.status = 'open') AND {MATCHES_LINK}
         ORDER BY t.name COLLATE NOCASE, t.id"
    ))
    .bind(meeting_id)
    .bind(open_only as i64)
    .fetch_all(pool)
    .await?)
}

/// Meetings a new series link should be backfilled from: prior occurrences within
/// `BACKFILL_DAYS` that have a completed summary, oldest first.
pub async fn backfill_candidates(
    pool: &SqlitePool,
    link: &TopicLink,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let (key, title) = match link.kind {
        LinkKind::Series => (
            Some(link.ref_.as_str()),
            link.title.as_deref().unwrap_or(""),
        ),
        LinkKind::Title => (None, link.ref_.as_str()),
        LinkKind::Meeting => return Ok(Vec::new()),
    };
    let prior =
        MeetingsRepository::find_prior_series_occurrences(pool, key, title, now, 500).await?;
    if prior.is_empty() {
        return Ok(prior);
    }
    let cutoff = now - Duration::days(BACKFILL_DAYS);
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT m.id FROM meetings m
         WHERE m.id IN (SELECT value FROM json_each(?1)) AND m.created_at >= ?2
           AND EXISTS (SELECT 1 FROM summary_processes sp
                       WHERE sp.meeting_id = m.id AND sp.status = 'completed')
         ORDER BY m.created_at ASC, m.id",
    )
    .bind(serde_json::to_string(&prior)?)
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Meetings to queue for the entry writer right after `link` was made from `seed`: the seed
/// meeting itself whenever it has a completed summary (whatever `backfill` says, because the
/// owner just added it and its summary won't run again on its own), plus, for a series or
/// title link with `backfill`, its prior occurrences. Oldest first, no duplicates.
pub async fn entry_targets(
    pool: &SqlitePool,
    link: &TopicLink,
    seed: &str,
    backfill: bool,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let mut ids = if backfill {
        backfill_candidates(pool, link, now).await?
    } else {
        Vec::new()
    };
    if !ids.iter().any(|id| id == seed) {
        ids.push(seed.to_string());
    }
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT m.id FROM meetings m
         WHERE m.id IN (SELECT value FROM json_each(?1))
           AND EXISTS (SELECT 1 FROM summary_processes sp
                       WHERE sp.meeting_id = m.id AND sp.status = 'completed')
         ORDER BY m.created_at ASC, m.id",
    )
    .bind(serde_json::to_string(&ids)?)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// The newest `limit` meetings any of the topic's links match that have a completed
/// summary, newest first. One query over the same membership predicate.
pub async fn recent_summarised_meetings(
    pool: &SqlitePool,
    topic_id: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(&format!(
        "SELECT m.id FROM meetings m
         LEFT JOIN meeting_series_links l ON l.meeting_id = m.id
         WHERE EXISTS (SELECT 1 FROM topic_links tl WHERE tl.topic_id = ?1 AND {MATCHES_LINK})
           AND EXISTS (SELECT 1 FROM summary_processes sp
                       WHERE sp.meeting_id = m.id AND sp.status = 'completed')
         ORDER BY m.created_at DESC, m.id LIMIT ?2"
    ))
    .bind(topic_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}
