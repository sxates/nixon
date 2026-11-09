//! specs/0087 — `search_meetings`: full-text search across transcripts, summaries and notes
//! for outside agents. Unlike the app's ⌘K search (best hit per meeting and source), it
//! returns up to [`TRANSCRIPT_HITS_PER_MEETING`] transcript hits per meeting, so an agent
//! sees each place a topic came up, plus the best summary hit and the best notes hit.

use std::collections::HashMap;

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use super::links::{meeting_link, moment_link};
use super::queries::{
    clock_label, one_line, wrap_content, DateRange, SearchArgs, ToolOutput, DATE_LABEL_SQL,
};
use crate::database::repositories::search::SearchRepository;

pub const TRANSCRIPT_HITS_PER_MEETING: i64 = 5;
const DEFAULT_LIMIT: u32 = 20;
const MAX_LIMIT: u32 = 50;

/// The multi-hit search. Modelled on `SEARCH_SQL` (repositories/search.rs): the per-source
/// CTEs are `MATERIALIZED` because FTS5 auxiliary functions (`bm25()`, `snippet()`) only work
/// in a direct query of the FTS table, and without it SQLite's subquery flattening pushes the
/// outer window/ordering into the FTS scan ("unable to use function bm25 in the requested
/// context"). `ROW_NUMBER()` per meeting replaces that query's `GROUP BY … MIN(rank)`.
///
/// bm25 scores aren't comparable across the three tables (accepted there too).
///
/// Binds, in order: match expression x3, the date-range binds, LIMIT.
const SEARCH_SQL_TEMPLATE: &str = "\
WITH transcript_hits AS MATERIALIZED (
    SELECT meeting_id,
           snippet(transcripts_fts, 0, char(1), char(2), '…', 12) AS snip,
           transcripts_fts.rowid AS trow,
           bm25(transcripts_fts) AS rank
    FROM transcripts_fts
    WHERE transcripts_fts MATCH ?
),
summary_hits AS MATERIALIZED (
    SELECT meeting_id,
           snippet(summaries_fts, 0, char(1), char(2), '…', 12) AS snip,
           bm25(summaries_fts) AS rank
    FROM summaries_fts
    WHERE summaries_fts MATCH ?
),
notes_hits AS MATERIALIZED (
    SELECT meeting_id,
           snippet(meeting_notes_fts, -1, char(1), char(2), '…', 12) AS snip,
           bm25(meeting_notes_fts) AS rank
    FROM meeting_notes_fts
    WHERE meeting_notes_fts MATCH ?
),
ranked AS (
    SELECT meeting_id, 'transcript' AS source, snip, trow, rank,
           ROW_NUMBER() OVER (PARTITION BY meeting_id ORDER BY rank) AS rn
    FROM transcript_hits
    UNION ALL
    SELECT meeting_id, 'summary' AS source, snip, NULL AS trow, rank,
           ROW_NUMBER() OVER (PARTITION BY meeting_id ORDER BY rank) AS rn
    FROM summary_hits
    UNION ALL
    SELECT meeting_id, 'notes' AS source, snip, NULL AS trow, rank,
           ROW_NUMBER() OVER (PARTITION BY meeting_id ORDER BY rank) AS rn
    FROM notes_hits
)
SELECT r.meeting_id, m.title, {date_label} AS date_label, r.source, r.snip,
       t.audio_start_time, NULLIF(TRIM(s.display_name), '') AS speaker
FROM ranked r
JOIN meetings m ON m.id = r.meeting_id
LEFT JOIN transcripts t ON r.trow IS NOT NULL AND t.rowid = r.trow
LEFT JOIN speakers s ON s.meeting_id = t.meeting_id AND s.speaker_key = t.speaker
WHERE ((r.source = 'transcript' AND r.rn <= {per_meeting}) OR r.rn = 1){dates}
ORDER BY r.rank, r.meeting_id
LIMIT ?";

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct HitRow {
    meeting_id: String,
    title: String,
    date_label: String,
    source: String,
    snip: String,
    audio_start_time: Option<f64>,
    speaker: Option<String>,
}

/// Searches with the recall expression (natural-language OR of the non-stopword terms).
/// Queries with no usable terms, and any FTS syntax error, give the plain no-match text.
pub async fn search_meetings(
    pool: &SqlitePool,
    scheme: &str,
    args: SearchArgs,
) -> Result<ToolOutput> {
    let no_match = || ToolOutput::plain(format!("No matches for \"{}\".", one_line(&args.query)));
    let dates = match DateRange::parse(args.from_date.as_deref(), args.to_date.as_deref()) {
        Ok(d) => d,
        Err(text) => return Ok(ToolOutput::plain(text)),
    };
    let Some(expr) = SearchRepository::build_recall_match_expr(&args.query) else {
        return Ok(no_match());
    };
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let hits = fetch_hits(pool, &expr, &dates, limit).await?;
    if hits.is_empty() {
        return Ok(no_match());
    }
    Ok(render(scheme, &args.query, hits))
}

/// Runs the search SQL. An FTS query error (the recall expression quotes every token, so
/// this shouldn't happen) becomes an empty result rather than reaching the agent.
pub(crate) async fn fetch_hits(
    pool: &SqlitePool,
    expr: &str,
    dates: &DateRange,
    limit: u32,
) -> Result<Vec<HitRow>> {
    let (date_sql, date_binds) = dates.sql();
    let sql = SEARCH_SQL_TEMPLATE
        .replace("{date_label}", DATE_LABEL_SQL)
        .replace("{per_meeting}", &TRANSCRIPT_HITS_PER_MEETING.to_string())
        .replace("{dates}", &date_sql);
    let mut query = sqlx::query_as::<_, HitRow>(&sql)
        .bind(expr)
        .bind(expr)
        .bind(expr);
    for b in &date_binds {
        query = query.bind(b);
    }
    match query.bind(i64::from(limit)).fetch_all(pool).await {
        Ok(rows) => Ok(rows),
        Err(e) if is_fts_query_error(&e) => Ok(Vec::new()),
        Err(e) => Err(e).context("Couldn't search the meetings"),
    }
}

fn is_fts_query_error(e: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db) = e else {
        return false;
    };
    let msg = db.message().to_lowercase();
    ["fts5", "syntax error", "unterminated", "malformed match"]
        .iter()
        .any(|needle| msg.contains(needle))
}

/// Groups hits by meeting (meetings in order of their best hit, hits by rank within).
fn render(scheme: &str, query: &str, hits: Vec<HitRow>) -> ToolOutput {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<HitRow>> = HashMap::new();
    for hit in hits {
        if !groups.contains_key(&hit.meeting_id) {
            order.push(hit.meeting_id.clone());
        }
        groups.entry(hit.meeting_id.clone()).or_default().push(hit);
    }
    let total: usize = groups.values().map(Vec::len).sum();

    let mut body = String::new();
    for id in &order {
        let group = &groups[id];
        let first = &group[0];
        body.push_str(&format!(
            "Meeting: {} | {} | id: {id} | {}\n",
            one_line(&first.title),
            first.date_label,
            meeting_link(scheme, id)
        ));
        for hit in group {
            let snippet = one_line(&hit.snip.replace(['\u{1}', '\u{2}'], "**"));
            if hit.source == "transcript" {
                let speaker = hit
                    .speaker
                    .as_deref()
                    .map(|s| format!("{}: ", one_line(s)))
                    .unwrap_or_default();
                body.push_str(&format!(
                    "  transcript {} {speaker}{snippet}  {}\n",
                    clock_label(hit.audio_start_time),
                    moment_link(scheme, id, hit.audio_start_time)
                ));
            } else {
                body.push_str(&format!(
                    "  {}: {snippet}  {}\n",
                    hit.source,
                    meeting_link(scheme, id)
                ));
            }
        }
    }
    let header = format!(
        "{total} hit{} in {} meeting{} for \"{}\", grouped by meeting, best first. \
         Matched words are in **bold**. Cite the links; read around a hit with get_meeting \
         and from_seconds/to_seconds.",
        if total == 1 { "" } else { "s" },
        order.len(),
        if order.len() == 1 { "" } else { "s" },
        one_line(query),
    );
    ToolOutput {
        text: format!("{header}\n{}", wrap_content(&body)),
        meeting_ids: order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn fts_syntax_error_is_an_empty_result() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let dates = DateRange {
            from: None,
            to: None,
        };
        // Raw FTS5 syntax that SQLite rejects; the recall expression never produces it.
        for expr in ["AND", "\"unterminated", "NEAR("] {
            let hits = fetch_hits(&pool, expr, &dates, 10)
                .await
                .unwrap_or_else(|e| panic!("{expr:?} errored: {e:#}"));
            assert!(hits.is_empty());
        }
    }
}
