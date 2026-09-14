use crate::database::models::{
    DateTimeUtc, MeetingListRow, MeetingModel, MeetingStatusRow, TranscriptWithSpeaker,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Error as SqlxError, FromRow, SqlitePool};

use super::MeetingsRepository;

/// One recent meeting a person was part of, for the People directory "Recent with
/// {person}" panel (specs/0038 WS5.b). Serialized camelCase for the frontend; this
/// plain list is separate from the synthesized person roll-up.
#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentPersonMeeting {
    pub id: String,
    pub title: String,
    /// From `meetings.created_at` — the meeting's effective start / displayed date.
    pub started_at: DateTimeUtc,
}

/// One candidate for deferred backlog processing (low-power-mode spec §5).
#[derive(Debug, Clone, FromRow)]
pub struct DeferredCandidateRow {
    pub id: String,
    pub title: String,
    pub folder_path: Option<String>,
    pub transcript_count: i64,
    pub processing_mode: Option<String>,
    pub created_at: String,
}

impl MeetingsRepository {
    /// Meetings with recorded audio whose processing is pending: explicitly
    /// deferred (`processing_mode='defer'`) or effectively untranscribed
    /// (sparse transcript — same threshold as the retention exemption).
    /// The caller filters by on-disk audio presence.
    ///
    /// The sparse arm additionally requires that the meeting has NO completed
    /// summary. A completed `summary_processes` row is the durable evidence that
    /// the meeting already went through full processing; without this guard a
    /// legitimately short (fully-processed) meeting would re-list on every AC
    /// transition and re-run retranscription + diarization + a possibly-paid
    /// summary forever, since processing leaves no other trace that suppresses
    /// re-listing. The explicit `processing_mode='defer'` arm stays
    /// unconditional — an explicitly-deferred meeting is always listed
    /// regardless of any summary.
    pub async fn list_deferred_candidates(
        pool: &SqlitePool,
    ) -> Result<Vec<DeferredCandidateRow>, SqlxError> {
        sqlx::query_as::<_, DeferredCandidateRow>(
            "SELECT m.id, m.title, m.folder_path, m.processing_mode, m.created_at,
                    (SELECT COUNT(*) FROM transcripts t WHERE t.meeting_id = m.id) AS transcript_count
             FROM meetings m
             WHERE m.folder_path IS NOT NULL
               AND (m.processing_mode = 'defer'
                    OR ((SELECT COUNT(*) FROM transcripts t WHERE t.meeting_id = m.id) < ?
                        AND NOT EXISTS (SELECT 1 FROM summary_processes sp
                                        WHERE sp.meeting_id = m.id AND sp.status = 'completed')))
             ORDER BY m.created_at ASC",
        )
        .bind(crate::audio::retention::MIN_TRANSCRIPT_SEGMENTS)
        .fetch_all(pool)
        .await
    }

    /// Recent meetings this person was part of — on the roster
    /// (`meeting_participants`, excluding tombstoned/removed rows) OR actually spoke
    /// (`speakers.person_id`) — newest first, capped at `limit`. Union semantics match
    /// the aggregation person scope
    /// (`AggregationScope.person_id`, specs/0035). Backs the People directory
    /// "Recent with {person}" plain list (specs/0038 WS5.b), separate from the
    /// synthesized roll-up. `scheduled` prep placeholders are excluded (they aren't
    /// real meetings, mirroring [`get_meetings_enriched`]).
    pub async fn recent_meetings_with_person(
        pool: &SqlitePool,
        person_id: &str,
        limit: i64,
    ) -> Result<Vec<RecentPersonMeeting>, SqlxError> {
        sqlx::query_as::<_, RecentPersonMeeting>(
            "SELECT m.id AS id, m.title AS title, m.created_at AS started_at \
             FROM meetings m \
             WHERE m.origin <> 'scheduled' \
               AND (EXISTS (SELECT 1 FROM meeting_participants mp \
                              WHERE mp.meeting_id = m.id AND mp.person_id = ?1 \
                                AND mp.removed_at IS NULL) \
                 OR EXISTS (SELECT 1 FROM speakers s \
                              WHERE s.meeting_id = m.id AND s.person_id = ?1)) \
             ORDER BY m.created_at DESC \
             LIMIT ?2",
        )
        .bind(person_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }

    pub async fn get_meetings(pool: &SqlitePool) -> Result<Vec<MeetingModel>, sqlx::Error> {
        let meetings =
            sqlx::query_as::<_, MeetingModel>("SELECT * FROM meetings ORDER BY created_at DESC")
                .fetch_all(pool)
                .await?;
        Ok(meetings)
    }

    /// Returns enriched meeting-list rows for the dashboard / sidebar in a single
    /// query (no N+1). Each row carries the meeting id/title/timestamps plus:
    /// - `duration_seconds`: `MAX(audio_end_time)` over the meeting's transcripts,
    ///   `NULL` when there are no timed transcripts.
    /// - `summary_result`: the raw `summary_processes.result` JSON blob (or `NULL`).
    ///   Gist extraction (parsing the `markdown` field, stripping headings) happens
    ///   in the command layer so the SQL stays portable.
    /// - `first_transcript`: the earliest transcript line (by `audio_start_time`
    ///   then `timestamp`), used as the gist fallback when there is no summary.
    ///
    /// Ordered `created_at DESC` so the dashboard can group by day top-down.
    pub async fn get_meetings_enriched(
        pool: &SqlitePool,
    ) -> Result<Vec<MeetingListRow>, sqlx::Error> {
        let rows = sqlx::query_as::<_, MeetingListRow>(
            r#"
            -- specs/0057: the reel ordinal is a stable archival handle — a 1-based
            -- number over the SAME set the list shows (scheduled rows excluded), oldest
            -- first. Numbered in a CTE so the `origin` filter applies BEFORE ROW_NUMBER.
            WITH reels AS (
                SELECT id,
                       ROW_NUMBER() OVER (ORDER BY created_at ASC, id ASC) AS reel_number
                  FROM meetings
                 WHERE origin <> 'scheduled'
            )
            SELECT
                m.id AS id,
                m.title AS title,
                m.created_at AS created_at,
                m.updated_at AS updated_at,
                m.origin AS origin,
                m.calendar_event_id AS calendar_event_id,
                (SELECT MAX(t.audio_end_time)
                   FROM transcripts t
                  WHERE t.meeting_id = m.id) AS duration_seconds,
                (SELECT sp.result
                   FROM summary_processes sp
                  WHERE sp.meeting_id = m.id) AS summary_result,
                (SELECT t.transcript
                   FROM transcripts t
                  WHERE t.meeting_id = m.id
                  ORDER BY t.audio_start_time IS NULL, t.audio_start_time, t.timestamp
                  LIMIT 1) AS first_transcript,
                r.reel_number AS reel_number
            FROM meetings m
            JOIN reels r ON r.id = m.id
            -- specs/0036: 'scheduled' rows are pre-call-prep placeholders (backing storage for
            -- an upcoming event's brief + prep notes), NOT real meetings — never list them.
            WHERE m.origin <> 'scheduled'
            ORDER BY m.created_at DESC
            "#,
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// The reel ordinal for a SINGLE meeting (specs/0057), for the meeting-details
    /// header — the same number `get_meetings_enriched` puts on the list row, computed
    /// without materialising the whole list: count the non-scheduled meetings that sort
    /// before this one under the list's `(created_at ASC, id ASC)` order, plus one.
    ///
    /// Returns `None` when the meeting does not exist or is a 'scheduled' placeholder
    /// (placeholders are outside the numbering, so they have no reel).
    pub async fn get_reel_number(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<i64>, SqlxError> {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT (SELECT COUNT(*)
                      FROM meetings o
                     WHERE o.origin <> 'scheduled'
                       AND (o.created_at < m.created_at
                            OR (o.created_at = m.created_at AND o.id < m.id))) + 1
              FROM meetings m
             WHERE m.id = ?1 AND m.origin <> 'scheduled'
            "#,
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
    }

    /// Returns one status row per meeting whose `created_at` falls in
    /// `[start_utc, end_utc)` (a local day's UTC bounds computed by the caller —
    /// today or any navigated day, specs/0038 WS4), for the Day Agenda
    /// (specs/0012). All four processing-status flags
    /// are derived in a SINGLE query via correlated `EXISTS`/aggregate subqueries —
    /// no N+1 across meetings:
    /// - `has_folder`: `folder_path` is set and non-empty (proxy for "recorded").
    /// - `has_transcript`: ≥1 row in `transcripts`.
    /// - `has_summary`: a `summary_processes.result` is present (non-NULL).
    /// - `has_speakers`: ≥1 row in `speakers` (the meeting was diarized).
    ///
    /// Ordered by `created_at ASC` so the agenda's recording items already arrive
    /// in start-time order before merging with calendar events.
    pub async fn get_between_with_status(
        pool: &SqlitePool,
        start_utc: DateTime<Utc>,
        end_utc: DateTime<Utc>,
    ) -> Result<Vec<MeetingStatusRow>, SqlxError> {
        let rows = sqlx::query_as::<_, MeetingStatusRow>(
            r#"
            SELECT
                m.id AS id,
                m.title AS title,
                m.created_at AS created_at,
                m.folder_path AS folder_path,
                (SELECT MAX(t.audio_end_time)
                   FROM transcripts t
                  WHERE t.meeting_id = m.id) AS duration_seconds,
                CASE WHEN m.folder_path IS NOT NULL AND TRIM(m.folder_path) <> ''
                     THEN 1 ELSE 0 END AS has_folder,
                CASE WHEN EXISTS (SELECT 1 FROM transcripts t WHERE t.meeting_id = m.id)
                     THEN 1 ELSE 0 END AS has_transcript,
                CASE WHEN EXISTS (SELECT 1 FROM summary_processes sp
                                   WHERE sp.meeting_id = m.id AND sp.result IS NOT NULL)
                     THEN 1 ELSE 0 END AS has_summary,
                CASE WHEN EXISTS (SELECT 1 FROM speakers s WHERE s.meeting_id = m.id)
                     THEN 1 ELSE 0 END AS has_speakers
            FROM meetings m
            -- specs/0036: exclude 'scheduled' prep placeholders so they never get merged into
            -- the Day Agenda (a scheduled row shares its event's title + start, so the merge
            -- would otherwise CLAIM the event as "recorded" and break the dismissal invariant).
            WHERE m.created_at >= ? AND m.created_at < ? AND m.origin <> 'scheduled'
            ORDER BY m.created_at ASC
            "#,
        )
        .bind(start_utc)
        .bind(end_utc)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// Get meeting transcripts with pagination support. Each row is LEFT JOINed to
    /// its `speakers` row so the frontend gets the resolved `speaker_name`
    /// (NULL/absent for un-diarized meetings — identical to today's behaviour).
    pub async fn get_meeting_transcripts_paginated(
        pool: &SqlitePool,
        meeting_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<TranscriptWithSpeaker>, i64), SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        // Get total count of transcripts for this meeting
        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_one(pool)
            .await?;

        // Get paginated transcripts ordered by audio_start_time
        let transcripts = sqlx::query_as::<_, TranscriptWithSpeaker>(
            "SELECT t.id, t.meeting_id, t.transcript, t.timestamp,
                    t.audio_start_time, t.audio_end_time, t.duration, t.speaker,
                    s.display_name AS speaker_name
             FROM transcripts t
             LEFT JOIN speakers s
               ON s.meeting_id = t.meeting_id AND s.speaker_key = t.speaker
             WHERE t.meeting_id = ?
             ORDER BY t.audio_start_time ASC
             LIMIT ? OFFSET ?",
        )
        .bind(meeting_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok((transcripts, total.0))
    }

    /// Suggests a summary template for a (new) meeting from its title
    /// (specs/0020 task 6): the template of the **most recent** prior meeting
    /// (by `created_at`) whose normalized title matches, skipping meetings with
    /// no explicit template and optionally excluding the meeting itself.
    ///
    /// Normalization is `LOWER(TRIM(...))` applied by SQLite to *both* sides so
    /// the column and the parameter are folded identically (Rust's Unicode
    /// `to_lowercase` disagrees with SQLite's ASCII-only `LOWER` on non-ASCII
    /// titles). The Rust-side [`normalize_title`] mirrors the v1 semantics and
    /// guards the degenerate empty-title case, which must never match anything.
    pub async fn suggest_template_for_title(
        pool: &SqlitePool,
        title: &str,
        exclude_meeting_id: Option<&str>,
    ) -> Result<Option<String>, SqlxError> {
        if normalize_title(title).is_empty() {
            return Ok(None);
        }

        let row: Option<(String,)> = sqlx::query_as(
            "SELECT template_id FROM meetings \
             WHERE LOWER(TRIM(title)) = LOWER(TRIM(?1)) \
               AND template_id IS NOT NULL \
               AND TRIM(template_id) != '' \
               AND (?2 IS NULL OR id != ?2) \
             ORDER BY created_at DESC \
             LIMIT 1",
        )
        .bind(title)
        .bind(exclude_meeting_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(|(template_id,)| template_id))
    }
}

/// Normalizes a meeting title for auto-select matching (specs/0020 task 6):
/// v1 is trim + lowercase, i.e. recurring calendar events that repeat their
/// title verbatim (possibly with stray whitespace/case differences) match.
/// Keep in sync with the SQL-side `LOWER(TRIM(...))` in
/// [`MeetingsRepository::suggest_template_for_title`].
pub fn normalize_title(title: &str) -> String {
    title.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::test_support::{
        dt, memory_db, recorded_with_summary,
    };

    /// specs/0020 task 6: auto-select a template from the most recent
    /// same-titled meeting.
    #[tokio::test]
    async fn suggest_template_for_title_matching() {
        let pool = memory_db().await;
        let base = Utc::now() - chrono::Duration::days(3);

        // Older meeting: "Weekly Sync" → retrospective.
        let older = MeetingsRepository::create_meeting(
            &pool,
            Some("Weekly Sync".to_string()),
            None,
            None,
            None,
            Some(base),
        )
        .await
        .unwrap();
        assert!(
            MeetingsRepository::set_meeting_template(&pool, &older, Some("retrospective"))
                .await
                .unwrap()
        );

        // Newer meeting, same title modulo case/whitespace → project_sync.
        let newer = MeetingsRepository::create_meeting(
            &pool,
            Some("  weekly SYNC ".to_string()),
            None,
            None,
            None,
            Some(base + chrono::Duration::days(1)),
        )
        .await
        .unwrap();
        assert!(
            MeetingsRepository::set_meeting_template(&pool, &newer, Some("project_sync"))
                .await
                .unwrap()
        );

        // Hit is case/whitespace-insensitive and the most recent wins.
        assert_eq!(
            MeetingsRepository::suggest_template_for_title(&pool, "weekly sync", None)
                .await
                .unwrap(),
            Some("project_sync".to_string())
        );
        assert_eq!(
            MeetingsRepository::suggest_template_for_title(&pool, "  WEEKLY SYNC  ", None)
                .await
                .unwrap(),
            Some("project_sync".to_string())
        );

        // Excluding the newest match falls back to the next most recent.
        assert_eq!(
            MeetingsRepository::suggest_template_for_title(&pool, "Weekly Sync", Some(&newer))
                .await
                .unwrap(),
            Some("retrospective".to_string())
        );

        // A different title misses.
        assert_eq!(
            MeetingsRepository::suggest_template_for_title(&pool, "Daily Standup", None)
                .await
                .unwrap(),
            None
        );
    }

    /// specs/0020 task 6: NULL/blank template_id rows never win, and blank
    /// titles never match.
    #[tokio::test]
    async fn suggest_template_for_title_ignores_untemplated_and_blank() {
        let pool = memory_db().await;
        let base = Utc::now() - chrono::Duration::days(3);

        let templated = MeetingsRepository::create_meeting(
            &pool,
            Some("Weekly Sync".to_string()),
            None,
            None,
            None,
            Some(base),
        )
        .await
        .unwrap();
        assert!(
            MeetingsRepository::set_meeting_template(&pool, &templated, Some("retrospective"))
                .await
                .unwrap()
        );

        // Newer meeting with NULL template_id (never set) is skipped.
        MeetingsRepository::create_meeting(
            &pool,
            Some("Weekly Sync".to_string()),
            None,
            None,
            None,
            Some(base + chrono::Duration::days(1)),
        )
        .await
        .unwrap();

        // Newest meeting with an (illegally) empty-string template_id is skipped
        // too — the setter normalizes '' to NULL, so force it with raw SQL.
        let newest = MeetingsRepository::create_meeting(
            &pool,
            Some("Weekly Sync".to_string()),
            None,
            None,
            None,
            Some(base + chrono::Duration::days(2)),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE meetings SET template_id = '' WHERE id = ?")
            .bind(&newest)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            MeetingsRepository::suggest_template_for_title(&pool, "Weekly Sync", None)
                .await
                .unwrap(),
            Some("retrospective".to_string())
        );

        // A blank/whitespace title never matches anything (even though blank
        // titles can't be persisted, the query must not treat '' as a key).
        assert_eq!(
            MeetingsRepository::suggest_template_for_title(&pool, "   ", None)
                .await
                .unwrap(),
            None
        );
    }

    #[test]
    fn normalize_title_trims_and_lowercases() {
        assert_eq!(normalize_title("  Weekly SYNC  "), "weekly sync");
        assert_eq!(normalize_title("   "), "");
    }

    /// specs/0036 (code-review): `scheduled` prep placeholders must never appear in the
    /// all-meetings list or the Day Agenda — they are backing storage for an upcoming event's
    /// brief, not real meetings, and would otherwise show as phantom rows / get claimed as
    /// recordings.
    #[tokio::test]
    async fn scheduled_rows_excluded_from_lists_and_agenda() {
        let pool = memory_db().await;
        let recorded =
            recorded_with_summary(&pool, "Weekly", "2026-07-10T10:00:00Z", Some("S")).await;
        let scheduled = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            "evt-1",
            Some("S"),
            "Weekly",
            dt("2026-07-10T15:00:00Z"),
        )
        .await
        .unwrap();

        // Enriched list (dashboard / sidebar / all-meetings / ⌘K) omits the scheduled row.
        let enriched = MeetingsRepository::get_meetings_enriched(&pool)
            .await
            .unwrap();
        let ids: Vec<&str> = enriched.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&recorded.as_str()));
        assert!(
            !ids.contains(&scheduled.as_str()),
            "scheduled row must not list"
        );

        // Day Agenda status query (same local day window) omits the scheduled row.
        let start = dt("2026-07-10T00:00:00Z");
        let end = dt("2026-07-11T00:00:00Z");
        let today = MeetingsRepository::get_between_with_status(&pool, start, end)
            .await
            .unwrap();
        let today_ids: Vec<&str> = today.iter().map(|r| r.id.as_str()).collect();
        assert!(today_ids.contains(&recorded.as_str()));
        assert!(
            !today_ids.contains(&scheduled.as_str()),
            "scheduled row must not enter the agenda merge"
        );
    }

    /// specs/0057 Task 4 — reel ordinal: every non-scheduled meeting gets a stable
    /// 1-based number by `created_at` ASC (ties broken by id), scheduled placeholders
    /// are excluded from the numbering entirely, and the single-meeting lookup used by
    /// the meeting-details header returns exactly the number the list carries.
    #[tokio::test]
    async fn reel_numbers_are_stable_ordinals_over_non_scheduled_meetings() {
        let pool = memory_db().await;

        let first = recorded_with_summary(&pool, "First", "2026-01-01T10:00:00Z", None).await;
        // A scheduled placeholder sitting BETWEEN two recordings must not consume an ordinal.
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, origin) \
             VALUES ('scheduled-1', 'Upcoming', '2026-01-02T10:00:00Z', '2026-01-02T10:00:00Z', 'scheduled')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let second = recorded_with_summary(&pool, "Second", "2026-01-03T10:00:00Z", None).await;
        let third = recorded_with_summary(&pool, "Third", "2026-01-04T10:00:00Z", None).await;

        let rows = MeetingsRepository::get_meetings_enriched(&pool)
            .await
            .unwrap();
        let by_id: std::collections::HashMap<&str, i64> = rows
            .iter()
            .map(|r| (r.id.as_str(), r.reel_number))
            .collect();

        assert_eq!(rows.len(), 3, "scheduled rows stay out of the list");
        assert_eq!(by_id[first.as_str()], 1);
        assert_eq!(by_id[second.as_str()], 2);
        assert_eq!(by_id[third.as_str()], 3);

        // Single-meeting lookup agrees with the list for every row.
        for (id, expected) in [(&first, 1), (&second, 2), (&third, 3)] {
            assert_eq!(
                MeetingsRepository::get_reel_number(&pool, id).await.unwrap(),
                Some(expected),
                "single-meeting reel number must match the list"
            );
        }

        // A scheduled placeholder has no reel number at all.
        assert_eq!(
            MeetingsRepository::get_reel_number(&pool, "scheduled-1")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            MeetingsRepository::get_reel_number(&pool, "no-such-meeting")
                .await
                .unwrap(),
            None
        );
    }
}
