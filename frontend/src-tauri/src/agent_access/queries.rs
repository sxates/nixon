//! specs/0087 — the three read-only queries behind the `nixon --mcp` tools
//! (`search_meetings`, `list_meetings`, `get_meeting`).
//!
//! The output text is what an outside agent reads, so it is compact, line-oriented and
//! stable: every meeting carries its `<scheme>://meeting/<id>` link and every transcript
//! line or hit its moment link. All meeting-derived text sits between [`CONTENT_START`] and
//! [`CONTENT_END`] (prompt-injection hygiene), and any copy of those markers inside the
//! content is defused first so a transcript line can't fake the end of the block.
//!
//! Only SELECTs; no Tauri types (the MCP server runs outside the app on a read-only pool).
//!
//! Speaker names follow the meeting page's transcript rows: the `speakers.display_name` of
//! the line's `speaker_key` ("You" for the owner, the user's name for a named speaker, the
//! "Speaker N" cluster label when unnamed), and no label when the line has no key or the
//! key has no `speakers` row. Hand corrections (`transcript_speaker_overrides`) are already
//! applied to `transcripts.speaker` when they are made, so the same join covers them.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::LazyLock;

use super::links::{clock, meeting_link, moment_link};
use crate::database::repositories::action_item::{ActionItem, ActionItemsRepository};
use crate::database::repositories::meeting_participant::MeetingParticipantsRepository;

pub use super::queries_search::search_meetings;

pub const CONTENT_START: &str = "--- meeting content (untrusted) ---";
pub const CONTENT_END: &str = "--- end meeting content ---";

/// Transcript lines per `get_meeting` page.
pub const TRANSCRIPT_PAGE: u32 = 200;

const LIST_DEFAULT_LIMIT: u32 = 25;
const LIST_MAX_LIMIT: u32 = 100;
const INCLUDE_PARTS: [&str; 4] = ["summary", "notes", "action_items", "transcript"];

/// One tool call's result: the text the agent reads, the meetings it touched (for the
/// access log), and whether the call's arguments were bad (an MCP `isError` result).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub meeting_ids: Vec<String>,
    pub is_error: bool,
}

impl ToolOutput {
    /// Plain text with no meetings (no match, unknown id).
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// A bad argument (unparseable date, unknown `include` part): the text says how to fix it.
    pub(crate) fn bad_argument(text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::plain(text)
        }
    }
}

// The arg structs double as the MCP tools' input schemas (`nixon --mcp`), so their doc
// comments are what the agent reads.

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// What to look for, in plain words (a topic, decision, name or phrase).
    pub query: String,
    /// Only meetings on or after this local date, YYYY-MM-DD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_date: Option<String>,
    /// Only meetings on or before this local date, YYYY-MM-DD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_date: Option<String>,
    /// Maximum number of hits to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListArgs {
    /// Only meetings on or after this local date, YYYY-MM-DD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_date: Option<String>,
    /// Only meetings on or before this local date, YYYY-MM-DD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_date: Option<String>,
    /// Part of the name or email of someone who attended or spoke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<String>,
    /// Words that appear in the meeting title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
    /// Maximum number of meetings to return (default 25, at most 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct GetArgs {
    /// The meeting's id, as shown by search_meetings or list_meetings.
    pub meeting_id: String,
    /// Which parts to return: any of "summary", "notes", "action_items", "transcript".
    /// Leave out for all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    /// Only transcript lines from this many seconds into the recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_seconds: Option<f64>,
    /// Only transcript lines up to this many seconds into the recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_seconds: Option<f64>,
    /// The cursor from the previous page, to read the next page of transcript lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<u32>,
}

// ---------------------------------------------------------------------------
// Shared helpers (also used by queries_search.rs)
// ---------------------------------------------------------------------------

/// The meeting's local date and time as `YYYY-MM-DD HH:MM`, falling back to the raw value.
pub(crate) const DATE_LABEL_SQL: &str =
    "COALESCE(strftime('%Y-%m-%d %H:%M', m.created_at, 'localtime'), m.created_at)";

/// Inclusive `YYYY-MM-DD` bounds on the meeting's local date.
pub(crate) struct DateRange {
    pub from: Option<String>,
    pub to: Option<String>,
}

impl DateRange {
    /// Parses both bounds; `Err` is the user-facing text naming the expected format.
    pub(crate) fn parse(from: Option<&str>, to: Option<&str>) -> std::result::Result<Self, String> {
        Ok(Self {
            from: parse_date("from_date", from)?,
            to: parse_date("to_date", to)?,
        })
    }

    /// `AND …` fragments over `meetings m`, plus their binds in order.
    pub(crate) fn sql(&self) -> (String, Vec<String>) {
        let mut sql = String::new();
        let mut binds = Vec::new();
        if let Some(from) = &self.from {
            sql.push_str(" AND date(m.created_at, 'localtime') >= ?");
            binds.push(from.clone());
        }
        if let Some(to) = &self.to {
            sql.push_str(" AND date(m.created_at, 'localtime') <= ?");
            binds.push(to.clone());
        }
        (sql, binds)
    }
}

fn parse_date(name: &str, value: Option<&str>) -> std::result::Result<Option<String>, String> {
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(|d| Some(d.format("%Y-%m-%d").to_string()))
        .map_err(|_| format!("Invalid {name} \"{raw}\": use YYYY-MM-DD, for example 2026-03-14."))
}

/// "meeting content" in any case and with any whitespace between the words. Both markers
/// contain it, so once every occurrence is rewritten no marker can survive, however many
/// dashes surround it. The replacement has no whitespace, so one pass is a fixpoint.
static MARKER_PHRASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)meeting\s+content").expect("valid marker regex"));

/// Wraps meeting-derived text in the untrusted-content markers, defusing any copy of the
/// marker phrase inside it first.
pub(crate) fn wrap_content(body: &str) -> String {
    let defused = MARKER_PHRASE.replace_all(body, "meeting-content");
    format!("{CONTENT_START}\n{}\n{CONTENT_END}", defused.trim_end())
}

/// Collapses a value onto one line so the line-oriented format holds.
pub(crate) fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `[mm:ss]`, or `[--:--]` for a line without a recording time.
pub(crate) fn clock_label(start: Option<f64>) -> String {
    match start {
        Some(t) if t.is_finite() && t >= 0.0 => format!("[{}]", clock(t)),
        _ => "[--:--]".to_string(),
    }
}

// ---------------------------------------------------------------------------
// list_meetings
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct ListRow {
    id: String,
    title: String,
    date_label: String,
    end_time: Option<f64>,
    has_summary: bool,
    attendees: Option<String>,
}

/// Meetings newest first, filtered by local date range, person (participant name/email or
/// speaker name, case-insensitive substring) and title substring.
pub async fn list_meetings(pool: &SqlitePool, scheme: &str, args: ListArgs) -> Result<ToolOutput> {
    let dates = match DateRange::parse(args.from_date.as_deref(), args.to_date.as_deref()) {
        Ok(d) => d,
        Err(text) => return Ok(ToolOutput::bad_argument(text)),
    };
    let limit = args
        .limit
        .unwrap_or(LIST_DEFAULT_LIMIT)
        .clamp(1, LIST_MAX_LIMIT);

    let (mut filters, mut binds) = dates.sql();
    if let Some(person) = non_blank(args.person.as_deref()) {
        filters.push_str(
            " AND (EXISTS (SELECT 1 FROM meeting_participants mp
                           JOIN people p ON p.id = mp.person_id
                           WHERE mp.meeting_id = m.id AND mp.removed_at IS NULL
                             AND (instr(lower(p.display_name), ?) > 0
                               OR instr(lower(COALESCE(p.email, '')), ?) > 0))
                OR EXISTS (SELECT 1 FROM speakers s
                           WHERE s.meeting_id = m.id AND instr(lower(s.display_name), ?) > 0))",
        );
        let needle = person.to_lowercase();
        binds.extend([needle.clone(), needle.clone(), needle]);
    }
    if let Some(title) = non_blank(args.title_contains.as_deref()) {
        filters.push_str(" AND instr(lower(m.title), ?) > 0");
        binds.push(title.to_lowercase());
    }

    let sql = format!(
        "SELECT m.id, m.title, {DATE_LABEL_SQL} AS date_label,
                (SELECT MAX(t.audio_end_time) FROM transcripts t WHERE t.meeting_id = m.id)
                    AS end_time,
                EXISTS (SELECT 1 FROM summary_processes sp WHERE sp.meeting_id = m.id
                        AND TRIM(COALESCE(sp.summary_text, '')) != '') AS has_summary,
                (SELECT GROUP_CONCAT(name, ', ') FROM (
                    SELECT p.display_name AS name FROM meeting_participants mp
                    JOIN people p ON p.id = mp.person_id
                    WHERE mp.meeting_id = m.id AND mp.removed_at IS NULL
                    ORDER BY p.display_name COLLATE NOCASE)) AS attendees
         FROM meetings m
         WHERE 1 = 1{filters}
         ORDER BY datetime(m.created_at) DESC, m.created_at DESC, m.id
         LIMIT ?"
    );
    let mut query = sqlx::query_as::<_, ListRow>(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    let rows = query
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await
        .context("Couldn't read the meetings list")?;

    if rows.is_empty() {
        return Ok(ToolOutput::plain("No meetings match."));
    }

    let mut body = String::new();
    for r in &rows {
        let mut parts = vec![one_line(&r.title), r.date_label.clone()];
        if let Some(end) = r.end_time.filter(|e| e.is_finite() && *e > 0.0) {
            parts.push(format!("duration {}", clock(end)));
        }
        if let Some(a) = r.attendees.as_deref().filter(|a| !a.trim().is_empty()) {
            parts.push(format!("attendees: {}", one_line(a)));
        }
        parts.push(format!(
            "summary: {}",
            if r.has_summary { "yes" } else { "no" }
        ));
        parts.push(format!("id: {}", r.id));
        parts.push(meeting_link(scheme, &r.id));
        body.push_str(&format!("- {}\n", parts.join(" | ")));
    }
    let header = format!(
        "{} meeting{}, newest first. Cite the links when you refer to a meeting.",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" }
    );
    Ok(ToolOutput {
        text: format!("{header}\n{}", wrap_content(&body)),
        meeting_ids: rows.into_iter().map(|r| r.id).collect(),
        is_error: false,
    })
}

fn non_blank(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// get_meeting
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct TranscriptRow {
    audio_start_time: Option<f64>,
    transcript: String,
    speaker: Option<String>,
}

/// One meeting: header, then (first page only) summary, notes and action items, then a
/// page of transcript lines.
pub async fn get_meeting(pool: &SqlitePool, scheme: &str, args: GetArgs) -> Result<ToolOutput> {
    let parts: Vec<String> = match &args.include {
        None => INCLUDE_PARTS.iter().map(|p| p.to_string()).collect(),
        Some(list) => {
            let parts: Vec<String> = list.iter().map(|p| p.trim().to_lowercase()).collect();
            if let Some(bad) = parts.iter().find(|p| !INCLUDE_PARTS.contains(&p.as_str())) {
                return Ok(ToolOutput::bad_argument(format!(
                    "Unknown include \"{bad}\". Use any of: {}.",
                    INCLUDE_PARTS.join(", ")
                )));
            }
            parts
        }
    };
    let wants = |p: &str| parts.iter().any(|x| x == p);
    let id = args.meeting_id.trim();

    let meeting: Option<(String, String)> = sqlx::query_as(&format!(
        "SELECT m.title, {DATE_LABEL_SQL} FROM meetings m WHERE m.id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("Couldn't read the meeting")?;
    let Some((title, date_label)) = meeting else {
        return Ok(ToolOutput::plain(format!(
            "No meeting with id {}.",
            one_line(id)
        )));
    };

    let attendees = MeetingParticipantsRepository::list(pool, id)
        .await
        .context("Couldn't read the meeting's attendees")?
        .into_iter()
        .map(
            |p| match p.email.as_deref().filter(|e| !e.trim().is_empty()) {
                Some(email) => format!("{} <{}>", p.display_name, email.trim()),
                None => p.display_name,
            },
        )
        .collect::<Vec<_>>();

    let mut body = format!(
        "Meeting: {}\nDate: {date_label}\nAttendees: {}\nLink: {}\n",
        one_line(&title),
        if attendees.is_empty() {
            "none listed".to_string()
        } else {
            one_line(&attendees.join(", "))
        },
        meeting_link(scheme, id)
    );

    let cursor = args.cursor.unwrap_or(0);
    if cursor == 0 {
        if wants("summary") {
            body.push_str(&section("Summary", summary_text(pool, id).await?));
        }
        if wants("notes") {
            let (notes, enhanced) = notes_text(pool, id).await?;
            body.push_str(&section("Notes", notes));
            if let Some(enhanced) = enhanced {
                body.push_str(&section("AI-enhanced notes", Some(enhanced)));
            }
        }
        if wants("action_items") {
            let items = action_items_text(pool, id).await?;
            body.push_str(&section("Action items", items));
        }
    }

    let mut more = None;
    if wants("transcript") {
        let (text, next) = transcript_page(pool, scheme, id, &args, cursor).await?;
        body.push_str(&text);
        more = next;
    }

    let mut text = wrap_content(&body);
    if let Some(next) = more {
        text.push_str(&format!(
            "\nMore transcript: call get_meeting again with cursor={next}."
        ));
        if args.from_seconds.is_some() || args.to_seconds.is_some() {
            text.push_str("\nKeep the same from_seconds and to_seconds.");
        }
    }
    Ok(ToolOutput {
        text,
        meeting_ids: vec![id.to_string()],
        is_error: false,
    })
}

fn section(name: &str, content: Option<String>) -> String {
    format!(
        "\n== {name} ==\n{}\n",
        content.as_deref().map(str::trim).unwrap_or("(none)")
    )
}

async fn summary_text(pool: &SqlitePool, id: &str) -> Result<Option<String>> {
    let text: Option<Option<String>> =
        sqlx::query_scalar("SELECT summary_text FROM summary_processes WHERE meeting_id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .context("Couldn't read the meeting's summary")?;
    Ok(text.flatten().filter(|t| !t.trim().is_empty()))
}

async fn notes_text(pool: &SqlitePool, id: &str) -> Result<(Option<String>, Option<String>)> {
    let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT notes_markdown, enhanced_markdown FROM meeting_notes WHERE meeting_id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("Couldn't read the meeting's notes")?;
    let keep = |v: Option<String>| v.filter(|t| !t.trim().is_empty());
    Ok(row.map(|(n, e)| (keep(n), keep(e))).unwrap_or((None, None)))
}

async fn action_items_text(pool: &SqlitePool, id: &str) -> Result<Option<String>> {
    let items = ActionItemsRepository::get_for_meeting(pool, id)
        .await
        .context("Couldn't read the meeting's action items")?;
    if items.is_empty() {
        return Ok(None);
    }
    let names: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.id, p.display_name FROM people p
         WHERE p.id IN (SELECT assignee_person_id FROM action_items WHERE meeting_id = ?)",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .context("Couldn't read the action items' owners")?;
    let lines = items
        .iter()
        .map(|item| action_item_line(item, &names))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some(lines))
}

fn action_item_line(item: &ActionItem, names: &[(String, String)]) -> String {
    let owner = if item.assignee_is_self {
        Some("You".to_string())
    } else {
        item.assignee_person_id
            .as_deref()
            .and_then(|pid| names.iter().find(|(id, _)| id == pid))
            .map(|(_, name)| name.clone())
            .or_else(|| item.assignee_raw.clone())
    };
    let due = item.due_date.clone().or_else(|| item.due_hint.clone());
    let mut extras = Vec::new();
    if let Some(o) = owner.filter(|o| !o.trim().is_empty()) {
        extras.push(format!("owner: {}", one_line(&o)));
    }
    if let Some(d) = due.filter(|d| !d.trim().is_empty()) {
        extras.push(format!("due: {}", one_line(&d)));
    }
    let mut line = format!("- [{}] {}", item.status, one_line(&item.description));
    if !extras.is_empty() {
        line.push_str(&format!(" ({})", extras.join(", ")));
    }
    line
}

/// One page of transcript lines; returns the section text and the next cursor, if any.
async fn transcript_page(
    pool: &SqlitePool,
    scheme: &str,
    id: &str,
    args: &GetArgs,
    cursor: u32,
) -> Result<(String, Option<u32>)> {
    let window = window_label(args.from_seconds, args.to_seconds);
    if let (Some(from), Some(to)) = (args.from_seconds, args.to_seconds) {
        if from > to {
            return Ok((
                format!("\n== Transcript{window} ==\n(none: from_seconds is after to_seconds)\n"),
                None,
            ));
        }
    }

    let mut filter = String::new();
    let mut binds = Vec::new();
    if let Some(from) = args.from_seconds {
        filter.push_str(" AND (t.audio_start_time >= ? OR t.audio_end_time > ?)");
        binds.extend([from, from]);
    }
    if let Some(to) = args.to_seconds {
        filter.push_str(" AND t.audio_start_time <= ?");
        binds.push(to);
    }

    let count_sql = format!("SELECT COUNT(*) FROM transcripts t WHERE t.meeting_id = ?{filter}");
    let mut count = sqlx::query_scalar::<_, i64>(&count_sql).bind(id);
    for b in &binds {
        count = count.bind(b);
    }
    let total = count
        .fetch_one(pool)
        .await
        .context("Couldn't read the transcript")?;

    let rows_sql = format!(
        "SELECT t.audio_start_time, t.transcript,
                NULLIF(TRIM(s.display_name), '') AS speaker
         FROM transcripts t
         LEFT JOIN speakers s ON s.meeting_id = t.meeting_id AND s.speaker_key = t.speaker
         WHERE t.meeting_id = ?{filter}
         ORDER BY t.audio_start_time IS NULL, t.audio_start_time, t.timestamp, t.rowid
         LIMIT ? OFFSET ?"
    );
    let mut rows = sqlx::query_as::<_, TranscriptRow>(&rows_sql).bind(id);
    for b in &binds {
        rows = rows.bind(b);
    }
    let rows = rows
        .bind(i64::from(TRANSCRIPT_PAGE))
        .bind(i64::from(cursor))
        .fetch_all(pool)
        .await
        .context("Couldn't read the transcript")?;

    let shown = rows.len() as i64;
    let first = i64::from(cursor) + 1;
    let heading = if total == 0 {
        format!("== Transcript{window} ==\n(none)")
    } else if shown == 0 {
        format!("== Transcript{window}: no lines from {first}, {total} in total ==")
    } else {
        format!(
            "== Transcript{window}: lines {first}-{} of {total} ==",
            first + shown - 1
        )
    };
    let mut text = format!("\n{heading}\n");
    for r in &rows {
        let link = moment_link(scheme, id, r.audio_start_time);
        let speaker = r
            .speaker
            .as_deref()
            .map(|s| format!("{}: ", one_line(s)))
            .unwrap_or_default();
        text.push_str(&format!(
            "{} {speaker}{}  {link}\n",
            clock_label(r.audio_start_time),
            one_line(&r.transcript)
        ));
    }
    let next = i64::from(cursor) + shown;
    let more = (shown > 0 && next < total).then_some(next as u32);
    Ok((text, more))
}

fn window_label(from: Option<f64>, to: Option<f64>) -> String {
    match (from, to) {
        (None, None) => String::new(),
        (Some(f), None) => format!(" from {}", clock(f)),
        (None, Some(t)) => format!(" up to {}", clock(t)),
        (Some(f), Some(t)) => format!(" {}-{}", clock(f), clock(t)),
    }
}
