//! specs/0091 — the read-only To-Do and Topic queries behind `nixon --mcp`
//! (`list_todos`, `list_topics`, `get_topic`) and the Topics line in `get_meeting`.
//!
//! The visibility rules are the app's own, reused rather than re-derived: the List view is
//! [`Todo::list_view`] (a Topic kept off the To-Do list hides its items there only), a
//! Topic's To-Dos are [`Todo::list_for_topic`], Topics come from [`TopicsRepository::list`]
//! in the sidebar's manual order. Only SELECTs; the server's pool is read-only.

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::links::{meeting_link, moment_link, topic_link};
use super::queries::{action_item_line, one_line, wrap_content, ToolOutput};
use crate::database::repositories::action_item_todo::{
    ActionItemTodoRepository as Todo, TodoRow, TodoView,
};
use crate::topics::ledger::entries_for_topic;
use crate::topics::links::{links_for_topic, topics_for_meeting, LinkKind};
use crate::topics::model::{TopicEntry, TopicSummary};
use crate::topics::repo::TopicsRepository;

const TODOS_DEFAULT_LIMIT: u32 = 50;
const TODOS_MAX_LIMIT: u32 = 200;
const ENTRIES_DEFAULT: u32 = 20;
const ENTRIES_MAX: u32 = 100;
/// The `topic` value that means "To-Dos with no Topic".
const NO_TOPIC: &str = "none";

// The arg structs double as the MCP tools' input schemas, so their doc comments are what
// the agent reads.

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TodosArgs {
    /// Only To-Dos on this Topic: its id or name (as list_topics shows). "none" = To-Dos
    /// with no Topic. Leave out for the user's whole To-Do list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Also return completed To-Dos, after the open ones (default false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_done: Option<bool>,
    /// Maximum number of To-Dos to return (default 50, at most 200).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TopicsArgs {
    /// Also list archived Topics (default false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_archived: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TopicArgs {
    /// The Topic's id, as list_topics shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The Topic's name, if you don't have its id: an exact name (any case) or the start of
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// How many timeline entries to return, newest first (default 20, at most 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<u32>,
}

// ---------------------------------------------------------------------------
// Resolving a Topic by id or name
// ---------------------------------------------------------------------------

enum Resolved {
    Found(TopicSummary),
    Missing,
    Ambiguous(Vec<TopicSummary>),
}

/// Exact id, then exact name (any case), then a unique name prefix (any case). Archived
/// Topics are included: a name the user gives is never silently ignored.
async fn resolve(pool: &SqlitePool, needle: &str) -> Result<Resolved> {
    let all = TopicsRepository::list(pool, None)
        .await
        .context("Couldn't read the Topics")?;
    if let Some(t) = all.iter().find(|t| t.id == needle) {
        return Ok(Resolved::Found(t.clone()));
    }
    let lower = needle.to_lowercase();
    let pick = |hits: Vec<&TopicSummary>| match hits.len() {
        0 => None,
        1 => Some(Resolved::Found(hits[0].clone())),
        _ => Some(Resolved::Ambiguous(hits.into_iter().cloned().collect())),
    };
    let exact = all
        .iter()
        .filter(|t| t.name.trim().to_lowercase() == lower)
        .collect();
    if let Some(r) = pick(exact) {
        return Ok(r);
    }
    let prefix = all
        .iter()
        .filter(|t| t.name.trim().to_lowercase().starts_with(&lower))
        .collect();
    Ok(pick(prefix).unwrap_or(Resolved::Missing))
}

/// The text for a name that didn't resolve to one Topic.
fn unresolved_text(scheme: &str, needle: &str, r: &Resolved) -> String {
    match r {
        Resolved::Ambiguous(hits) => {
            let list = hits
                .iter()
                .map(|t| {
                    format!(
                        "- {} | id: {} | {}",
                        one_line(&t.name),
                        t.id,
                        topic_link(scheme, &t.id)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "More than one Topic matches \"{}\". Call again with one of these ids:\n{}",
                one_line(needle),
                wrap_content(&list)
            )
        }
        _ => format!(
            "No Topic matches \"{}\". Call list_topics to see them all.",
            one_line(needle)
        ),
    }
}

fn status_label(status: &str) -> &str {
    if status == "open" {
        "open"
    } else {
        "archived"
    }
}

/// An RFC 3339 time as a local `YYYY-MM-DD`; anything else as its first ten characters.
fn local_date(at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|_| at.chars().take(10).collect())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------------------
// To-Do lines (shared by list_todos and get_topic)
// ---------------------------------------------------------------------------

/// `people.id → display_name` for the rows' assignees.
async fn assignee_names(pool: &SqlitePool, rows: &[TodoRow]) -> Result<Vec<(String, String)>> {
    let mut ids: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.item.assignee_person_id.as_deref())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ph = vec!["?"; ids.len()].join(", ");
    let sql = format!("SELECT id, display_name FROM people WHERE id IN ({ph})");
    let mut q = sqlx::query_as::<_, (String, String)>(&sql);
    for id in ids {
        q = q.bind(id);
    }
    q.fetch_all(pool)
        .await
        .context("Couldn't read the To-Dos' owners")
}

/// One To-Do: the meeting page's action-item line, then highlight, Topic and source.
fn todo_line(scheme: &str, row: &TodoRow, names: &[(String, String)], show_topic: bool) -> String {
    let mut line = action_item_line(&row.item, names);
    if let Some(h) = row.highlight.as_deref().filter(|h| !h.is_empty()) {
        line.push_str(&format!(" | highlight: {h}"));
    }
    if show_topic {
        if let (Some(id), Some(name)) = (row.topic_id.as_deref(), row.topic_name.as_deref()) {
            line.push_str(&format!(
                " | Topic: [{}]({})",
                one_line(name),
                topic_link(scheme, id)
            ));
        }
    }
    if let Some(mid) = row.item.meeting_id.as_deref() {
        let title = row.meeting_title.as_deref().unwrap_or("Untitled");
        line.push_str(&format!(
            " | from: [{}]({})",
            one_line(title),
            meeting_link(scheme, mid)
        ));
    }
    line
}

fn source_meetings(rows: &[TodoRow], into: &mut Vec<String>) {
    for id in rows.iter().filter_map(|r| r.item.meeting_id.as_deref()) {
        if !into.iter().any(|x| x == id) {
            into.push(id.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// list_todos
// ---------------------------------------------------------------------------

/// The To-Do List view in the user's order (open), then optionally the Done view (newest
/// first). With a `topic`, that Topic's To-Dos, including a Topic kept off the To-Do list.
pub async fn list_todos(pool: &SqlitePool, scheme: &str, args: TodosArgs) -> Result<ToolOutput> {
    let include_done = args.include_done.unwrap_or(false);
    let limit = args
        .limit
        .unwrap_or(TODOS_DEFAULT_LIMIT)
        .clamp(1, TODOS_MAX_LIMIT) as usize;
    let needle = args
        .topic
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());

    // (which To-Dos, open rows, done rows)
    let (scope, mut rows, done) = match needle {
        None => (
            "on your To-Do list".to_string(),
            Todo::list_view(pool, TodoView::List).await?,
            done_rows(pool, include_done, |_| true).await?,
        ),
        Some(n) if n.eq_ignore_ascii_case(NO_TOPIC) => (
            "with no Topic".to_string(),
            Todo::list_view(pool, TodoView::List)
                .await?
                .into_iter()
                .filter(|r| r.topic_id.is_none())
                .collect(),
            done_rows(pool, include_done, |r| r.topic_id.is_none()).await?,
        ),
        Some(n) => {
            let topic = match resolve(pool, n).await? {
                Resolved::Found(t) => t,
                other => return Ok(ToolOutput::plain(unresolved_text(scheme, n, &other))),
            };
            let open = Todo::list_for_topic(pool, &topic.id)
                .await?
                .into_iter()
                .filter(|r| r.item.status == "open")
                .collect();
            let id = topic.id.clone();
            let done =
                done_rows(pool, include_done, |r| r.topic_id.as_deref() == Some(&id)).await?;
            (
                format!(
                    "on the Topic {} ({})",
                    one_line(&topic.name),
                    topic_link(scheme, &topic.id)
                ),
                open,
                done,
            )
        }
    };
    rows.extend(done);
    if rows.is_empty() {
        return Ok(ToolOutput::plain(format!(
            "No {}To-Dos {scope}.",
            if include_done { "" } else { "open " }
        )));
    }
    let total = rows.len();
    rows.truncate(limit);

    let names = assignee_names(pool, &rows).await?;
    let show_topic = needle.is_none();
    let body = rows
        .iter()
        .map(|r| todo_line(scheme, r, &names, show_topic))
        .collect::<Vec<_>>()
        .join("\n");
    let shown = if rows.len() < total {
        format!("first {} of {total}", rows.len())
    } else {
        plural(total, "To-Do", "To-Dos")
    };
    let order = if include_done {
        "open ones in the user's order, then completed ones newest first"
    } else {
        "open, in the user's order"
    };
    let mut meeting_ids = Vec::new();
    source_meetings(&rows, &mut meeting_ids);
    Ok(ToolOutput {
        text: format!(
            "{shown} {scope} ({order}). Cite the from: links when you use where a To-Do came \
             from.\n{}",
            wrap_content(&body)
        ),
        meeting_ids,
        is_error: false,
    })
}

async fn done_rows(
    pool: &SqlitePool,
    include: bool,
    keep: impl Fn(&TodoRow) -> bool,
) -> Result<Vec<TodoRow>> {
    if !include {
        return Ok(Vec::new());
    }
    Ok(Todo::list_view(pool, TodoView::Done)
        .await?
        .into_iter()
        .filter(keep)
        .collect())
}

// ---------------------------------------------------------------------------
// list_topics
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct TopicCounts {
    id: String,
    series: i64,
    meetings: i64,
    open_todos: i64,
}

/// Topics in the sidebar's manual order, open ones only unless `include_archived`.
pub async fn list_topics(pool: &SqlitePool, scheme: &str, args: TopicsArgs) -> Result<ToolOutput> {
    let archived = args.include_archived.unwrap_or(false);
    let topics = TopicsRepository::list(pool, if archived { None } else { Some("open") })
        .await
        .context("Couldn't read the Topics")?;
    if topics.is_empty() {
        return Ok(ToolOutput::plain(if archived {
            "The user has no Topics yet."
        } else {
            "The user has no open Topics. Pass include_archived=true to see archived ones."
        }));
    }
    // Open To-Dos count the way a Topic's card shows them (listed and open).
    let counts: Vec<TopicCounts> = sqlx::query_as(
        "SELECT t.id,
                (SELECT COUNT(*) FROM topic_links l
                  WHERE l.topic_id = t.id AND l.kind IN ('series', 'title')) AS series,
                (SELECT COUNT(*) FROM topic_links l
                  WHERE l.topic_id = t.id AND l.kind = 'meeting') AS meetings,
                (SELECT COUNT(*) FROM action_items ai WHERE ai.topic_id = t.id
                  AND ai.triage = 'listed' AND ai.status = 'open') AS open_todos
         FROM topics t",
    )
    .fetch_all(pool)
    .await
    .context("Couldn't read the Topics' counts")?;

    let mut body = String::new();
    for t in &topics {
        let c = counts.iter().find(|c| c.id == t.id);
        let (series, meetings, todos) =
            c.map_or((0, 0, 0), |c| (c.series, c.meetings, c.open_todos));
        let mut parts = vec![format!(
            "{}{}",
            if t.starred { "★ " } else { "" },
            one_line(&t.name)
        )];
        parts.push(format!("status: {}", status_label(&t.status)));
        if t.hide_from_todo {
            parts.push("kept off the To-Do list".to_string());
        }
        if let Some(first) = t.description.lines().map(str::trim).find(|l| !l.is_empty()) {
            parts.push(format!("about: {}", one_line(first)));
        }
        parts.push(format!(
            "linked: {}, {}",
            plural(series as usize, "Series", "Series"),
            plural(meetings as usize, "Meeting", "Meetings")
        ));
        parts.push(plural(todos as usize, "open To-Do", "open To-Dos"));
        if let Some(last) = t.last_activity.as_deref() {
            parts.push(format!("last entry: {}", local_date(last)));
        }
        parts.push(format!("id: {}", t.id));
        parts.push(topic_link(scheme, &t.id));
        body.push_str(&format!("- {}\n", parts.join(" | ")));
    }
    Ok(ToolOutput {
        text: format!(
            "{}, in the user's order. Read one with get_topic; cite its link when you refer \
             to it.\n{}",
            plural(topics.len(), "Topic", "Topics"),
            wrap_content(&body)
        ),
        ..ToolOutput::default()
    })
}

// ---------------------------------------------------------------------------
// get_topic
// ---------------------------------------------------------------------------

/// One Topic: description, the user's notes, links, open To-Dos and the newest timeline
/// entries, every point and decision linked to its meeting moment.
pub async fn get_topic(pool: &SqlitePool, scheme: &str, args: TopicArgs) -> Result<ToolOutput> {
    let id = args.id.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let name = args
        .name
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let topic_id = match (id, name) {
        (Some(id), _) => id.to_string(),
        (None, Some(n)) => match resolve(pool, n).await? {
            Resolved::Found(t) => t.id,
            other => return Ok(ToolOutput::plain(unresolved_text(scheme, n, &other))),
        },
        (None, None) => {
            return Ok(ToolOutput::bad_argument(
                "Pass the Topic's id or name (list_topics shows both).",
            ))
        }
    };
    let Some(topic) = TopicsRepository::get(pool, &topic_id)
        .await
        .context("Couldn't read the Topic")?
    else {
        return Ok(ToolOutput::plain(format!(
            "No Topic with id {}. Call list_topics to see them all.",
            one_line(&topic_id)
        )));
    };
    let link = topic_link(scheme, &topic.id);

    let mut status = status_label(&topic.status).to_string();
    if topic.starred {
        status.push_str(", starred");
    }
    if topic.hide_from_todo {
        status.push_str(", kept off the To-Do list");
    }
    let mut body = format!(
        "Topic: {}\nStatus: {status}\nLink: {link}\n",
        one_line(&topic.name)
    );
    let desc = topic.description.trim();
    body.push_str(&format!(
        "\n== Description ==\n{}\n",
        if desc.is_empty() { "(none)" } else { desc }
    ));
    let notes = topic.notes_markdown.as_deref().map(str::trim).unwrap_or("");
    body.push_str(&format!(
        "\n== My notes ==\n{}\n",
        if notes.is_empty() { "(none)" } else { notes }
    ));

    let mut meeting_ids = Vec::new();
    let links = links_for_topic(pool, &topic.id)
        .await
        .context("Couldn't read the Topic's links")?;
    body.push_str("\n== Linked ==\n");
    if links.is_empty() {
        body.push_str("(none)\n");
    }
    for l in &links {
        match l.kind {
            LinkKind::Meeting => {
                body.push_str(&format!(
                    "- Meeting: [{}]({})\n",
                    one_line(&l.label),
                    meeting_link(scheme, &l.ref_)
                ));
                meeting_ids.push(l.ref_.clone());
            }
            LinkKind::Series | LinkKind::Title => {
                body.push_str(&format!("- Series: {}\n", one_line(&l.label)));
            }
        }
    }

    let todos: Vec<TodoRow> = Todo::list_for_topic(pool, &topic.id)
        .await?
        .into_iter()
        .filter(|r| r.item.status == "open")
        .collect();
    body.push_str("\n== Open To-Dos ==\n");
    if todos.is_empty() {
        body.push_str("(none)\n");
    } else {
        let names = assignee_names(pool, &todos).await?;
        for r in &todos {
            body.push_str(&todo_line(scheme, r, &names, false));
            body.push('\n');
        }
    }
    source_meetings(&todos, &mut meeting_ids);

    let entries = entries_for_topic(pool, &topic.id)
        .await
        .context("Couldn't read the Topic's timeline")?;
    let cap = args
        .entries
        .unwrap_or(ENTRIES_DEFAULT)
        .clamp(1, ENTRIES_MAX) as usize;
    let shown = entries.len().min(cap);
    body.push_str(&if entries.is_empty() {
        "\n== Timeline ==\n(none)\n".to_string()
    } else {
        format!(
            "\n== Timeline: newest {shown} of {}, newest first ==\n",
            entries.len()
        )
    });
    for e in entries.iter().take(cap) {
        body.push_str(&entry_text(scheme, e));
        if let Some(m) = e
            .meeting_id
            .as_deref()
            .filter(|_| e.source_deleted_at.is_none())
        {
            if !meeting_ids.iter().any(|x| x == m) {
                meeting_ids.push(m.to_string());
            }
        }
    }

    let mut text = wrap_content(&body);
    text.push_str(&format!(
        "\nCite this Topic as a markdown link to {link}; for a point or decision, link the \
         meeting moment beside it."
    ));
    if entries.len() > shown {
        text.push_str(&format!(
            "\nOlder entries: call get_topic again with a larger entries (up to {ENTRIES_MAX})."
        ));
    }
    Ok(ToolOutput {
        text,
        meeting_ids,
        is_error: false,
    })
}

/// One timeline entry: date and source meeting, then its points and decisions.
fn entry_text(scheme: &str, e: &TopicEntry) -> String {
    let live_meeting = e
        .meeting_id
        .as_deref()
        .filter(|_| e.source_deleted_at.is_none());
    let source = match (e.meeting_id.as_deref(), live_meeting) {
        (_, Some(m)) => format!(
            "[{}]({})",
            one_line(e.meeting_title.as_deref().unwrap_or("Untitled")),
            meeting_link(scheme, m)
        ),
        (Some(_), None) => "a meeting that was deleted".to_string(),
        (None, None) => "added by hand".to_string(),
    };
    let mut out = format!("\n{} · {source}\n", local_date(&e.occurred_at));
    let at = |t: Option<f64>| {
        live_meeting
            .map(|m| format!("  {}", moment_link(scheme, m, t)))
            .unwrap_or_default()
    };
    if !e.body.bullets.is_empty() {
        out.push_str("Points:\n");
        for b in &e.body.bullets {
            out.push_str(&format!("- {}{}\n", one_line(&b.text), at(b.t)));
        }
    }
    if !e.body.decisions.is_empty() {
        out.push_str("Decisions:\n");
        for d in &e.body.decisions {
            out.push_str(&format!("- {}{}\n", one_line(&d.text), at(d.t)));
        }
    }
    if e.body.bullets.is_empty() && e.body.decisions.is_empty() {
        out.push_str("(no points)\n");
    }
    out
}

// ---------------------------------------------------------------------------
// get_meeting's Topics line
// ---------------------------------------------------------------------------

/// `Topics: [Name](link), …` (or `Topics: none`) for a meeting, archived Topics included.
pub(crate) async fn meeting_topics_line(
    pool: &SqlitePool,
    scheme: &str,
    meeting_id: &str,
) -> Result<String> {
    let topics = topics_for_meeting(pool, meeting_id, false)
        .await
        .context("Couldn't read the meeting's Topics")?;
    let list = if topics.is_empty() {
        "none".to_string()
    } else {
        topics
            .iter()
            .map(|t| format!("[{}]({})", one_line(&t.name), topic_link(scheme, &t.id)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(format!("\nTopics: {list}\n"))
}
