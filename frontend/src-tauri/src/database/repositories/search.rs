//! Full-text search over transcripts, summaries, and notes (specs/0033).
//!
//! Queries the three external-content FTS5 tables created by
//! `migrations/20260706000000_add_fts5_search.sql`, keeping the best hit per
//! (meeting, source) by bm25 and returning `snippet()` excerpts delimited by
//! `\u{1}`/`\u{2}` sentinel characters that the frontend converts to `<mark>`
//! (the backend never emits HTML).

use crate::search::MeetingSearchHit;
use sqlx::{Error as SqlxError, SqlitePool};

pub use super::search_rank::{MeetingRanking, RankedMeeting};

/// Default number of hits returned when the caller doesn't specify a limit
/// (decided 2026-07-02: cap 20, best-per-meeting-per-source).
const DEFAULT_LIMIT: u32 = 20;

/// The UNION query over the three FTS tables.
///
/// Each per-source CTE is `MATERIALIZED` deliberately: FTS5 auxiliary functions
/// (`bm25()`, `snippet()`) may only be evaluated inside a direct query of the
/// FTS table, and without `MATERIALIZED` SQLite's subquery flattening pushes the
/// outer `MIN(rank)` aggregate into the FTS scan, which fails with
/// "unable to use function bm25 in the requested context".
///
/// The `GROUP BY meeting_id` + `MIN(rank)` in the grouping stage relies on
/// SQLite's documented min/max special case: the bare `snip`/`tid` columns are
/// taken from the row that produced the minimum rank, i.e. the best-matching
/// segment per meeting.
///
/// bm25 scores aren't comparable across the three tables; accepted for v1
/// (per-source MIN grouping + a small LIMIT keeps it sane — specs/0033 Risks).
///
/// `snippet()` column selection: 0 for the single-text-column tables, -1 for
/// meeting_notes_fts so the snippet comes from whichever of
/// notes_markdown/enhanced_markdown actually matched.
///
/// Binds, in order: match expression x3 (transcripts, summaries, notes), LIMIT.
const SEARCH_SQL: &str = "\
WITH transcript_hits AS MATERIALIZED (
    SELECT meeting_id,
           snippet(transcripts_fts, 0, char(1), char(2), '…', 12) AS snip,
           (SELECT t.id FROM transcripts t WHERE t.rowid = transcripts_fts.rowid) AS tid,
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
grouped AS (
    SELECT meeting_id, 'transcript' AS source, snip, tid, MIN(rank) AS rank
    FROM transcript_hits GROUP BY meeting_id
    UNION ALL
    SELECT meeting_id, 'summary' AS source, snip, NULL AS tid, MIN(rank) AS rank
    FROM summary_hits GROUP BY meeting_id
    UNION ALL
    SELECT meeting_id, 'notes' AS source, snip, NULL AS tid, MIN(rank) AS rank
    FROM notes_hits GROUP BY meeting_id
)
SELECT g.meeting_id, m.title, m.created_at, g.source,
       g.snip AS snippet, g.tid AS transcript_id, g.rank
FROM grouped g
JOIN meetings m ON m.id = g.meeting_id
ORDER BY g.rank
LIMIT ?";

pub struct SearchRepository;

impl SearchRepository {
    /// Turns raw user input into a safe FTS5 MATCH expression, or `None` when
    /// there is nothing to search for.
    ///
    /// Never pass user input to MATCH raw: bare `AND`/`"`/`(` are FTS5 syntax
    /// errors (the FTS analog of the 0028 non-ASCII slicing panic). Strategy:
    /// split on whitespace, escape internal `"` by doubling, wrap every token in
    /// `"…"` (quoted tokens are string literals, never operators), and suffix
    /// the final token with `*` so results appear while the user is still
    /// typing. Tokens are implicitly ANDed by FTS5.
    ///
    /// A token that tokenizes to nothing (e.g. `"` alone) yields an empty
    /// phrase, which FTS5 matches against zero rows without erroring
    /// (verified empirically; fixture-covered in tests/fts_search.rs).
    pub fn sanitize_match_query(raw: &str) -> Option<String> {
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        let last = tokens.len().checked_sub(1)?;

        let expr = tokens
            .iter()
            .enumerate()
            .map(|(i, token)| {
                let escaped = token.replace('"', "\"\"");
                if i == last {
                    // Prefix-star the final token for search-as-you-type.
                    format!("\"{escaped}\"*")
                } else {
                    format!("\"{escaped}\"")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        Some(expr)
    }

    /// Ranked full-text search across transcripts, summaries, and notes.
    /// Returns at most `limit` (default 20) hits, best hit per (meeting, source),
    /// ordered by bm25 rank (lower = better). An empty/whitespace query returns
    /// no hits without touching the database.
    pub async fn search(
        pool: &SqlitePool,
        raw_query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<MeetingSearchHit>, SqlxError> {
        let Some(match_expr) = Self::sanitize_match_query(raw_query) else {
            return Ok(Vec::new());
        };
        let limit = i64::from(limit.unwrap_or(DEFAULT_LIMIT));

        // Name-matched decode (`FromRow` on MeetingSearchHit): a future column
        // reorder in SEARCH_SQL fails loudly instead of silently misassigning
        // same-typed fields.
        sqlx::query_as::<_, MeetingSearchHit>(SEARCH_SQL)
            .bind(&match_expr)
            .bind(&match_expr)
            .bind(&match_expr)
            .bind(limit)
            .fetch_all(pool)
            .await
    }
}

#[cfg(test)]
mod sanitize_tests {
    use super::SearchRepository;

    fn sanitize(raw: &str) -> Option<String> {
        SearchRepository::sanitize_match_query(raw)
    }

    #[test]
    fn empty_and_whitespace_yield_none() {
        assert_eq!(sanitize(""), None);
        assert_eq!(sanitize("   \t\n "), None);
    }

    #[test]
    fn bare_operator_keywords_are_quoted_literals() {
        // Unquoted, `AND` is an FTS5 operator and a syntax error on its own.
        assert_eq!(sanitize("AND"), Some("\"AND\"*".to_string()));
        assert_eq!(
            sanitize("AND budget"),
            Some("\"AND\" \"budget\"*".to_string())
        );
    }

    #[test]
    fn parens_are_neutralized_by_quoting() {
        // Unquoted `(` is a syntax error; quoted it is a (token-less) literal.
        assert_eq!(sanitize("("), Some("\"(\"*".to_string()));
        assert_eq!(sanitize("(budget)"), Some("\"(budget)\"*".to_string()));
    }

    #[test]
    fn double_quotes_are_escaped_by_doubling() {
        assert_eq!(sanitize("\""), Some("\"\"\"\"*".to_string()));
        assert_eq!(
            sanitize("say \"hello\""),
            Some("\"say\" \"\"\"hello\"\"\"*".to_string())
        );
    }

    #[test]
    fn non_ascii_tokens_pass_through_untouched() {
        assert_eq!(
            sanitize("café 日本語"),
            Some("\"café\" \"日本語\"*".to_string())
        );
        assert_eq!(sanitize("résumé"), Some("\"résumé\"*".to_string()));
    }

    #[test]
    fn only_last_token_gets_prefix_star() {
        assert_eq!(
            sanitize("quarterly budget fore"),
            Some("\"quarterly\" \"budget\" \"fore\"*".to_string())
        );
    }
}
