//! Meeting ranking for aggregation questions (specs/0035 gather stage).
//!
//! Split out of `search.rs` (specs/0056 W3) as a no-behaviour-change move: the
//! recall-oriented MATCH builder, the per-meeting best-hit ranking SQL, the
//! metadata-only fallback, and the consent-pinning mode. Everything here is an
//! `impl SearchRepository` block, so callers keep using
//! `SearchRepository::rank_meetings_for_question`.

use super::search::SearchRepository;
use crate::aggregation::scope::AggregationScope;
use sqlx::{Error as SqlxError, SqlitePool};

/// Small English stopword list for the recall-oriented (OR-semantics) question
/// gather (specs/0035). Function words carry no retrieval signal and, OR-joined,
/// would match nearly every meeting — dropping them keeps bm25 ranking on the
/// content words. Checked case-insensitively after trimming edge punctuation.
const RECALL_STOPWORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "been", "but", "by", "did", "do", "does",
    "for", "from", "had", "has", "have", "he", "her", "his", "how", "i", "if", "in", "is", "it",
    "its", "me", "my", "no", "not", "of", "on", "or", "our", "she", "so", "that", "the", "their",
    "them", "they", "this", "to", "us", "was", "we", "were", "what", "when", "where", "which",
    "who", "why", "will", "with", "would", "you", "your",
];

/// One meeting selected by [`SearchRepository::rank_meetings_for_question`].
#[derive(Debug, Clone)]
pub struct RankedMeeting {
    pub meeting_id: String,
    /// Joined from `meetings` at query time.
    pub title: String,
    pub created_at: String,
    /// Source of the meeting's best FTS hit: "transcript" | "summary" | "notes".
    /// `None` when the meeting was selected by metadata only (a question with no
    /// usable terms, or a pinned id without an FTS hit) — no relevance evidence
    /// exists.
    pub best_source: Option<String>,
    /// Best-matching segment id when `best_source` is "transcript". Kept for
    /// compatibility (search deep-links); the excerpt policy now reads
    /// [`Self::transcript_hit_ids`].
    pub best_transcript_id: Option<String>,
    /// The meeting's top [`TRANSCRIPT_HITS_PER_MEETING`] transcript segment ids
    /// by bm25 (best first), recorded **regardless of which source won**
    /// overall (specs/0056 W3). bm25 is not comparable across the three FTS
    /// tables, so a summary out-ranking the transcript says nothing about where
    /// the evidence lives; `aggregation::gather` windows the excerpt around
    /// these. Empty when the transcript had no hit, for the metadata-only
    /// fallback, and for pinned ids without an FTS hit.
    pub transcript_hit_ids: Vec<String>,
}

/// How many transcript segments per meeting travel with a [`RankedMeeting`]
/// (specs/0056 W3: K = 3 — enough to cover a point made twice without turning
/// the excerpt into the whole transcript).
pub const TRANSCRIPT_HITS_PER_MEETING: usize = 3;

/// Separator between ids in the `group_concat` column of [`RANK_SQL_TEMPLATE`]
/// (ASCII unit separator; segment ids are `transcript-<uuid>` and never contain it).
const HIT_ID_SEPARATOR: char = '\u{1f}';

/// Result of ranking meetings for an aggregation question.
#[derive(Debug, Clone)]
pub struct MeetingRanking {
    /// Best-first (bm25) for FTS selection; newest-first for metadata fallback.
    /// Pinned-id scopes rank FTS hits first, then the remaining pinned ids
    /// newest-first. Already capped at the caller's `max_meetings`.
    pub meetings: Vec<RankedMeeting>,
    /// Exact count of matches beyond the cap — surfaced in the egress preview
    /// ("top N of N+dropped"). Always 0 for pinned-id scopes (the set was
    /// already capped at preview time).
    pub dropped: usize,
}

/// The per-meeting best-hit ranking query for aggregation gather (specs/0035).
///
/// Same three-way FTS UNION and `MATERIALIZED` discipline as `search::SEARCH_SQL`
/// (see its doc comment for why), but grouped one level further: `best` keeps a
/// single row per meeting via SQLite's min/max special case (the bare
/// `source`/`tid` columns come from the row that produced `MIN(rank)`), so each
/// meeting carries the source and transcript segment of its best hit. bm25
/// scores aren't comparable across the three tables — inherited v1 limitation
/// (specs/0033 Risks), bounded by the meeting cap and the visible preview list.
///
/// `transcript_top` (specs/0056 W3) keeps each meeting's top-K transcript
/// segments by bm25 — `ROW_NUMBER() OVER (PARTITION BY meeting_id ORDER BY
/// rank, tid)` over the already-materialized transcript hits, so no FTS
/// auxiliary function is evaluated outside its scan — and folds them into one
/// `group_concat` column in rank order (SQLite ≥ 3.44 honours `ORDER BY`
/// inside the aggregate; the bundled library is 3.46). It is `LEFT JOIN`ed so
/// meetings whose only hits are summary/notes still rank, with `NULL` ids.
///
/// `COUNT(*) OVER ()` evaluates before the `LIMIT`, so `total` is the exact
/// number of matches (drives the preview's "top N of {total}").
///
/// Binds, in order: transcript match expression, the per-meeting hit cap K,
/// summary match expression, notes match expression, then the caller-appended
/// scope binds, then the LIMIT. The `{scope}` placeholder is replaced with
/// `AND …` metadata filters (and, for pinned scopes, the `m.id IN (…)`
/// restriction).
const RANK_SQL_TEMPLATE: &str = "\
WITH transcript_hits AS MATERIALIZED (
    SELECT meeting_id,
           (SELECT t.id FROM transcripts t WHERE t.rowid = transcripts_fts.rowid) AS tid,
           bm25(transcripts_fts) AS rank
    FROM transcripts_fts
    WHERE transcripts_fts MATCH ?
),
transcript_top AS (
    SELECT meeting_id, group_concat(tid, char(31) ORDER BY rank, tid) AS tids
    FROM (
        SELECT meeting_id, tid, rank,
               ROW_NUMBER() OVER (PARTITION BY meeting_id ORDER BY rank, tid) AS rn
        FROM transcript_hits
    )
    WHERE rn <= ?
    GROUP BY meeting_id
),
summary_hits AS MATERIALIZED (
    SELECT meeting_id, bm25(summaries_fts) AS rank
    FROM summaries_fts
    WHERE summaries_fts MATCH ?
),
notes_hits AS MATERIALIZED (
    SELECT meeting_id, bm25(meeting_notes_fts) AS rank
    FROM meeting_notes_fts
    WHERE meeting_notes_fts MATCH ?
),
grouped AS (
    SELECT meeting_id, 'transcript' AS source, tid, MIN(rank) AS rank
    FROM transcript_hits GROUP BY meeting_id
    UNION ALL
    SELECT meeting_id, 'summary' AS source, NULL AS tid, MIN(rank) AS rank
    FROM summary_hits GROUP BY meeting_id
    UNION ALL
    SELECT meeting_id, 'notes' AS source, NULL AS tid, MIN(rank) AS rank
    FROM notes_hits GROUP BY meeting_id
),
best AS (
    SELECT meeting_id, source, tid, MIN(rank) AS rank
    FROM grouped GROUP BY meeting_id
)
SELECT b.meeting_id, m.title, m.created_at, b.source, b.tid, tt.tids,
       COUNT(*) OVER () AS total
FROM best b
JOIN meetings m ON m.id = b.meeting_id
LEFT JOIN transcript_top tt ON tt.meeting_id = b.meeting_id
WHERE 1=1{scope}
ORDER BY b.rank, m.created_at DESC
LIMIT ?";

/// Metadata-only fallback (no usable question terms, no pinned ids): scope
/// filters alone, newest first. Content-less meetings — nothing in
/// `summary_processes.summary_text`, no non-blank `meeting_notes` row, no
/// non-blank `transcripts` row — are excluded in SQL: gather would skip them
/// anyway (they contribute nothing to the model), and letting them through
/// would waste cap slots and make the preview's "top N of {total}" wrong. The
/// FTS branch needs no such filter (an FTS hit implies content).
///
/// Binds, in order: the caller-appended scope binds, then the LIMIT.
const METADATA_FALLBACK_SQL_TEMPLATE: &str = "\
SELECT m.id, m.title, m.created_at, COUNT(*) OVER () AS total
FROM meetings m
WHERE (EXISTS (SELECT 1 FROM summary_processes sp
               WHERE sp.meeting_id = m.id
                 AND TRIM(COALESCE(sp.summary_text, '')) != '')
    OR EXISTS (SELECT 1 FROM meeting_notes mn
               WHERE mn.meeting_id = m.id
                 AND (TRIM(COALESCE(mn.notes_markdown, '')) != ''
                   OR TRIM(COALESCE(mn.enhanced_markdown, '')) != ''))
    OR EXISTS (SELECT 1 FROM transcripts t
               WHERE t.meeting_id = m.id AND TRIM(t.transcript) != ''))\
{scope}
ORDER BY m.created_at DESC
LIMIT ?";

impl SearchRepository {
    /// Turns a natural-language question into a recall-oriented (OR-semantics)
    /// FTS5 MATCH expression, or `None` when no usable terms remain (specs/0035).
    ///
    /// [`Self::sanitize_match_query`] ANDs every token — right for
    /// search-as-you-type, fatal for a question ("what did we decide about the
    /// pricing page" would require all seven words to co-occur). Here we drop a
    /// small English stopword list, then quote each surviving token with the
    /// same `"…"` escaping discipline (doubled internal quotes; quoted tokens
    /// are string literals, never operators) and join with ` OR `. No trailing
    /// `*` — questions are complete words, not prefixes. Edge punctuation is
    /// trimmed per token ("page?" → "page") so stopword matching works and the
    /// quoted literal matches what unicode61 tokenized; duplicate terms are
    /// dropped case-insensitively.
    pub fn build_recall_match_expr(question: &str) -> Option<String> {
        let mut seen = std::collections::HashSet::new();
        let mut terms: Vec<String> = Vec::new();

        for raw_token in question.split_whitespace() {
            let trimmed = raw_token.trim_matches(|c: char| !c.is_alphanumeric());
            if trimmed.is_empty() {
                continue;
            }
            let lower = trimmed.to_lowercase();
            if RECALL_STOPWORDS.contains(&lower.as_str()) {
                continue;
            }
            if !seen.insert(lower) {
                continue;
            }
            let escaped = trimmed.replace('"', "\"\"");
            terms.push(format!("\"{escaped}\""));
        }

        if terms.is_empty() {
            None
        } else {
            Some(terms.join(" OR "))
        }
    }

    /// Appends the metadata scope filters (`AND …` fragments referencing the
    /// joined `meetings m`) and collects their bind values in order.
    ///
    /// Date bounds are full ISO-8601 UTC instants compared with `datetime()`:
    /// `date_from` inclusive, `date_to` **exclusive** (the frontend sends
    /// local-midnight day boundaries converted to UTC; `date_to` is the start
    /// of the day after the selected end day).
    fn push_scope_filters(scope: &AggregationScope, sql: &mut String, binds: &mut Vec<String>) {
        if let Some(from) = scope.date_from.as_deref().filter(|s| !s.trim().is_empty()) {
            sql.push_str(" AND datetime(m.created_at) >= datetime(?)");
            binds.push(from.to_string());
        }
        if let Some(to) = scope.date_to.as_deref().filter(|s| !s.trim().is_empty()) {
            sql.push_str(" AND datetime(m.created_at) < datetime(?)");
            binds.push(to.to_string());
        }
        if let Some(person) = scope.person_id.as_deref().filter(|s| !s.trim().is_empty()) {
            // Union semantics (specs/0035): on the roster OR actually spoke.
            sql.push_str(
                " AND (EXISTS (SELECT 1 FROM meeting_participants mp \
                               WHERE mp.meeting_id = m.id AND mp.person_id = ? AND mp.removed_at IS NULL) \
                   OR EXISTS (SELECT 1 FROM speakers s \
                               WHERE s.meeting_id = m.id AND s.person_id = ?))",
            );
            binds.push(person.to_string());
            binds.push(person.to_string());
        }
    }

    /// Selects and ranks the meetings an aggregation question should draw from
    /// (specs/0035 gather stage). Read-only; no LLM involvement.
    ///
    /// Three selection modes:
    /// - **Relevance (default):** the question's OR-semantics MATCH expression
    ///   over all three FTS tables, best hit per meeting (`MIN(bm25)`),
    ///   intersected with the scope's metadata filters. Each returned meeting
    ///   records its best-hit source and, for transcript hits, the segment id
    ///   (the transcript-excerpt policy's anchor). Capped at `max_meetings`;
    ///   `dropped` is the exact count of matches beyond the cap.
    /// - **Pinned ids (consent pinning):** explicit `meeting_ids` fix the SET
    ///   (nothing outside it is ever returned; the run can't diverge from what
    ///   the preview showed) while the question's FTS ranking still supplies
    ///   order and best-hit metadata within it — so transcript excerpts keep
    ///   working. Pinned ids with no FTS hit are appended after the ranked
    ///   hits, newest first. `dropped` is always 0 (the set was already capped
    ///   at preview time); `max_meetings` still applies as a hard upper bound.
    /// - **Metadata-only fallback:** a question with no usable terms selects by
    ///   scope filters alone, newest first, excluding meetings with no content
    ///   (gather would skip them). Same cap/`dropped` treatment as relevance.
    ///
    /// An explicitly empty `meeting_ids` list selects nothing.
    pub async fn rank_meetings_for_question(
        pool: &SqlitePool,
        question: &str,
        scope: &AggregationScope,
        max_meetings: usize,
    ) -> Result<MeetingRanking, SqlxError> {
        if let Some(ids) = scope.meeting_ids.as_deref() {
            if ids.is_empty() {
                return Ok(MeetingRanking {
                    meetings: Vec::new(),
                    dropped: 0,
                });
            }
            let mut meetings = Self::rank_pinned_ids(pool, question, scope, ids).await?;
            meetings.truncate(max_meetings);
            return Ok(MeetingRanking {
                meetings,
                dropped: 0,
            });
        }

        let (meetings, total) = match Self::build_recall_match_expr(question) {
            Some(expr) => Self::fetch_fts_ranked(pool, &expr, scope, "", &[], max_meetings).await?,
            None => {
                let mut scope_sql = String::new();
                let mut binds: Vec<String> = Vec::new();
                Self::push_scope_filters(scope, &mut scope_sql, &mut binds);
                let sql = METADATA_FALLBACK_SQL_TEMPLATE.replace("{scope}", &scope_sql);

                let mut query = sqlx::query_as::<_, (String, String, String, i64)>(&sql);
                for bind in &binds {
                    query = query.bind(bind);
                }
                let rows = query.bind(max_meetings as i64).fetch_all(pool).await?;

                let total = rows.first().map_or(0, |row| row.3 as usize);
                let meetings = rows
                    .into_iter()
                    .map(|(meeting_id, title, created_at, _total)| RankedMeeting {
                        meeting_id,
                        title,
                        created_at,
                        best_source: None,
                        best_transcript_id: None,
                        transcript_hit_ids: Vec::new(),
                    })
                    .collect();
                (meetings, total)
            }
        };

        let dropped = total.saturating_sub(meetings.len());
        Ok(MeetingRanking { meetings, dropped })
    }

    /// The pinned-ids mode: FTS-rank the question WITHIN the pinned set (so
    /// best-hit source/segment metadata — and thereby transcript excerpts —
    /// keep preview/run doc parity), then append pinned ids with no FTS hit,
    /// newest first. A question with no usable terms degrades to the pure
    /// metadata fetch over the set.
    async fn rank_pinned_ids(
        pool: &SqlitePool,
        question: &str,
        scope: &AggregationScope,
        ids: &[String],
    ) -> Result<Vec<RankedMeeting>, SqlxError> {
        let mut meetings: Vec<RankedMeeting> = Vec::new();

        if let Some(expr) = Self::build_recall_match_expr(question) {
            let placeholders = vec!["?"; ids.len()].join(", ");
            let id_filter = format!(" AND m.id IN ({placeholders})");
            let (ranked, _total) =
                Self::fetch_fts_ranked(pool, &expr, scope, &id_filter, ids, ids.len()).await?;
            meetings = ranked;
        }

        // Pinned ids the FTS pass didn't surface (no hit, or no usable terms):
        // appended after the ranked hits, newest first.
        let remaining: Vec<String> = ids
            .iter()
            .filter(|id| !meetings.iter().any(|m| &m.meeting_id == *id))
            .cloned()
            .collect();
        if !remaining.is_empty() {
            let placeholders = vec!["?"; remaining.len()].join(", ");
            let mut sql = format!(
                "SELECT m.id, m.title, m.created_at FROM meetings m \
                 WHERE m.id IN ({placeholders})"
            );
            let mut binds: Vec<String> = remaining;
            Self::push_scope_filters(scope, &mut sql, &mut binds);
            sql.push_str(" ORDER BY m.created_at DESC");

            let mut query = sqlx::query_as::<_, (String, String, String)>(&sql);
            for bind in &binds {
                query = query.bind(bind);
            }
            meetings.extend(query.fetch_all(pool).await?.into_iter().map(
                |(meeting_id, title, created_at)| RankedMeeting {
                    meeting_id,
                    title,
                    created_at,
                    best_source: None,
                    best_transcript_id: None,
                    transcript_hit_ids: Vec::new(),
                },
            ));
        }

        Ok(meetings)
    }

    /// Runs [`RANK_SQL_TEMPLATE`] with the given extra `{scope}` fragment
    /// (`extra_filter_sql` + its `extra_binds`, then the scope's own filters),
    /// returning the capped rows and the exact pre-cap match total.
    async fn fetch_fts_ranked(
        pool: &SqlitePool,
        match_expr: &str,
        scope: &AggregationScope,
        extra_filter_sql: &str,
        extra_binds: &[String],
        limit: usize,
    ) -> Result<(Vec<RankedMeeting>, usize), SqlxError> {
        let mut scope_sql = String::from(extra_filter_sql);
        let mut binds: Vec<String> = extra_binds.to_vec();
        Self::push_scope_filters(scope, &mut scope_sql, &mut binds);
        let sql = RANK_SQL_TEMPLATE.replace("{scope}", &scope_sql);

        type Row = (
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
        );
        let mut query = sqlx::query_as::<_, Row>(&sql)
            .bind(match_expr)
            .bind(TRANSCRIPT_HITS_PER_MEETING as i64)
            .bind(match_expr)
            .bind(match_expr);
        for bind in &binds {
            query = query.bind(bind);
        }
        let rows = query.bind(limit as i64).fetch_all(pool).await?;

        let total = rows.first().map_or(0, |row| row.6 as usize);
        let meetings = rows
            .into_iter()
            .map(
                |(meeting_id, title, created_at, source, tid, tids, _total)| RankedMeeting {
                    meeting_id,
                    title,
                    created_at,
                    best_source: Some(source),
                    best_transcript_id: tid,
                    transcript_hit_ids: split_hit_ids(tids.as_deref()),
                },
            )
            .collect();
        Ok((meetings, total))
    }
}

/// Splits the `group_concat` column of [`RANK_SQL_TEMPLATE`] back into ids,
/// preserving its rank order. `None`/empty → no transcript hits.
fn split_hit_ids(tids: Option<&str>) -> Vec<String> {
    tids.unwrap_or_default()
        .split(HIT_ID_SEPARATOR)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod recall_expr_tests {
    use super::SearchRepository;

    fn recall(question: &str) -> Option<String> {
        SearchRepository::build_recall_match_expr(question)
    }

    #[test]
    fn empty_and_whitespace_yield_none() {
        assert_eq!(recall(""), None);
        assert_eq!(recall("   \t\n "), None);
    }

    #[test]
    fn stopwords_only_yields_none() {
        assert_eq!(recall("what did we do about it"), None);
        assert_eq!(recall("Who is that?"), None);
    }

    #[test]
    fn content_words_survive_and_are_or_joined() {
        assert_eq!(
            recall("what did we decide about the pricing page"),
            Some("\"decide\" OR \"pricing\" OR \"page\"".to_string())
        );
    }

    #[test]
    fn no_trailing_prefix_star() {
        // Questions are complete words — unlike sanitize_match_query, no `*`.
        assert_eq!(recall("budget"), Some("\"budget\"".to_string()));
    }

    #[test]
    fn edge_punctuation_is_trimmed_for_stopwords_and_terms() {
        // "we?" must still be recognized as a stopword; "pricing?" must quote clean.
        assert_eq!(recall("we? pricing?"), Some("\"pricing\"".to_string()));
    }

    #[test]
    fn stopword_check_is_case_insensitive() {
        assert_eq!(recall("What DID We"), None);
        assert_eq!(
            recall("What About Pricing"),
            Some("\"Pricing\"".to_string())
        );
    }

    #[test]
    fn duplicate_terms_are_dropped_case_insensitively() {
        assert_eq!(
            recall("pricing Pricing PRICING page"),
            Some("\"pricing\" OR \"page\"".to_string())
        );
    }

    #[test]
    fn internal_double_quotes_are_escaped_by_doubling() {
        // Edge quotes get trimmed as punctuation; internal ones must be doubled
        // so the quoted literal stays a literal.
        assert_eq!(recall("foo\"bar"), Some("\"foo\"\"bar\"".to_string()));
    }

    #[test]
    fn non_ascii_terms_pass_through() {
        assert_eq!(
            recall("café 日本語"),
            Some("\"café\" OR \"日本語\"".to_string())
        );
    }

    #[test]
    fn punctuation_only_tokens_are_dropped() {
        assert_eq!(recall("??? -- ()"), None);
    }
}

#[cfg(test)]
mod rank_meetings_tests;
