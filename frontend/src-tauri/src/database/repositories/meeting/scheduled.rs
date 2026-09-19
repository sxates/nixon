//! The `scheduled`-origin placeholder lifecycle (specs/0036, revised by specs/0064 W1).
//!
//! A `scheduled` row is the backing store for one upcoming calendar occurrence's prep notes
//! and cached brief. It is invisible to every list query (`origin <> 'scheduled'`) until
//! Join & Record promotes it to `recorded`.

use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, SqlitePool};
use uuid::Uuid;

use super::MeetingsRepository;

/// How [`MeetingsRepository::upsert_scheduled_meeting`] resolved one calendar occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduledResolution {
    /// A row already existed for this exact occurrence.
    Existing(String),
    /// A row that belonged to a DIFFERENT day was moved onto this one (specs/0064 W1) — the
    /// meeting was rescheduled, so the caller invalidates its now-stale cached brief.
    Redated(String),
    /// No row matched; a fresh placeholder was minted.
    Created(String),
}

impl ScheduledResolution {
    pub fn id(&self) -> &str {
        match self {
            Self::Existing(id) | Self::Redated(id) | Self::Created(id) => id,
        }
    }

    pub fn into_id(self) -> String {
        match self {
            Self::Existing(id) | Self::Redated(id) | Self::Created(id) => id,
        }
    }

    pub fn was_redated(&self) -> bool {
        matches!(self, Self::Redated(_))
    }
}

/// Whether this calendar event id names ONE occurrence on its own.
///
/// Google instance ids are `gcal:{calendar}/{series}_{instanceTs}` where `instanceTs` is the
/// occurrence's ORIGINAL start, and Google rewrites a moved instance in place — the id does
/// not change when the meeting is rescheduled, so it identifies the occurrence across a move.
/// EventKit has no per-occurrence identifier at all: `eventIdentifier` is shared by every
/// occurrence of a recurring series and can be reissued by a provider re-sync, and
/// `calendarItemExternalIdentifier` (the iCalUID) is shared too. Only the Google form may
/// therefore be matched without also pinning the day.
pub(crate) fn is_per_occurrence_event_id(calendar_event_id: &str) -> bool {
    calendar_event_id.starts_with("gcal:")
}

impl MeetingsRepository {
    /// The `scheduled`-origin prep row for one calendar-event OCCURRENCE (specs/0036,
    /// revised by specs/0064 W1).
    ///
    /// A per-occurrence id ([`is_per_occurrence_event_id`] — Google) matches on the id alone,
    /// so a rescheduled occurrence still resolves to its own row. Everything else (EventKit)
    /// additionally requires the same UTC calendar day, because one id is shared by every
    /// occurrence of a recurring series and the day is all that pins one of them.
    ///
    /// Used both to make [`Self::upsert_scheduled_meeting`] idempotent and to adopt the prep
    /// row at record start.
    pub async fn find_scheduled_for_occurrence(
        pool: &SqlitePool,
        calendar_event_id: &str,
        occurrence_start: DateTime<Utc>,
    ) -> Result<Option<String>, SqlxError> {
        if calendar_event_id.trim().is_empty() {
            return Ok(None);
        }
        let row: Option<(String,)> = if is_per_occurrence_event_id(calendar_event_id) {
            sqlx::query_as(
                "SELECT id FROM meetings \
                 WHERE calendar_event_id = ?1 AND origin = 'scheduled' \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(calendar_event_id)
            .fetch_optional(pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT id FROM meetings \
                 WHERE calendar_event_id = ?1 AND origin = 'scheduled' \
                   AND date(created_at) = date(?2) \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(calendar_event_id)
            .bind(occurrence_start)
            .fetch_optional(pool)
            .await?
        };
        Ok(row.map(|(id,)| id))
    }

    /// Mint, return, or move the `scheduled` prep row for one occurrence of a calendar event
    /// (specs/0036, revised by specs/0064 W1). The row is dated to `occurrence_start` (its
    /// `created_at`, like Join & Record) and carries the series key. Prep notes and the cached
    /// brief attach to this row; Join & Record later adopts it (see
    /// [`Self::promote_scheduled_to_recorded`]) so it becomes the recording.
    ///
    /// When the id names one occurrence (Google) and its row sits on a different day, the
    /// meeting was rescheduled: the row is **re-dated** onto the new slot so the prep written
    /// against it follows the meeting, rather than being stranded on a row no list will ever
    /// show. `Redated` tells the caller to invalidate the cached brief, whose prior-occurrence
    /// set may have changed.
    pub async fn upsert_scheduled_meeting(
        pool: &SqlitePool,
        calendar_event_id: &str,
        series_key: Option<&str>,
        title: &str,
        occurrence_start: DateTime<Utc>,
    ) -> Result<ScheduledResolution, SqlxError> {
        if let Some(existing) =
            Self::find_scheduled_for_occurrence(pool, calendar_event_id, occurrence_start).await?
        {
            // A per-occurrence id matches regardless of day, so the hit may be the same
            // meeting on its OLD day — move it rather than leaving the prep behind.
            if is_per_occurrence_event_id(calendar_event_id)
                && Self::scheduled_day_differs(pool, &existing, occurrence_start).await?
                && Self::redate_scheduled_meeting(pool, &existing, occurrence_start).await?
            {
                return Ok(ScheduledResolution::Redated(existing));
            }
            return Ok(ScheduledResolution::Existing(existing));
        }
        let meeting_id = format!("meeting-{}", Uuid::new_v4());
        let title = {
            let t = title.trim();
            if t.is_empty() {
                "New Meeting".to_string()
            } else {
                t.to_string()
            }
        };
        let series_key = series_key.map(str::trim).filter(|k| !k.is_empty());
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, origin, calendar_event_id, calendar_series_key) \
             VALUES (?, ?, ?, ?, 'scheduled', ?, ?)",
        )
        .bind(&meeting_id)
        .bind(&title)
        .bind(occurrence_start)
        .bind(now)
        .bind(calendar_event_id)
        .bind(series_key)
        .execute(pool)
        .await?;
        Ok(ScheduledResolution::Created(meeting_id))
    }

    /// Whether `meeting_id`'s stored occurrence day is not `occurrence_start`'s day.
    async fn scheduled_day_differs(
        pool: &SqlitePool,
        meeting_id: &str,
        occurrence_start: DateTime<Utc>,
    ) -> Result<bool, SqlxError> {
        let same: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM meetings WHERE id = ?1 AND date(created_at) = date(?2)",
        )
        .bind(meeting_id)
        .bind(occurrence_start)
        .fetch_optional(pool)
        .await?;
        Ok(same.is_none())
    }

    /// The one unrecorded `scheduled` row for this event id that carries prep — notes or a
    /// cached brief — together with its stored occurrence start.
    ///
    /// `None` when no such row exists, or when more than one does. Ambiguity must never be
    /// guessed at: on EventKit a sibling occurrence shares the event id, and stranding prep
    /// is recoverable where carrying another occurrence's prep away is not.
    pub async fn scheduled_day_with_prep(
        pool: &SqlitePool,
        calendar_event_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SqlxError> {
        if calendar_event_id.trim().is_empty() {
            return Ok(None);
        }
        let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT m.id, m.created_at FROM meetings m \
             WHERE m.calendar_event_id = ?1 AND m.origin = 'scheduled' \
               AND NOT EXISTS (SELECT 1 FROM transcripts t WHERE t.meeting_id = m.id) \
               AND ( \
                 EXISTS (SELECT 1 FROM meeting_notes n WHERE n.meeting_id = m.id \
                           AND TRIM(COALESCE(n.prep_markdown, '')) <> '') \
                 OR EXISTS (SELECT 1 FROM meeting_briefs b WHERE b.meeting_id = m.id) \
               ) \
             ORDER BY m.created_at DESC LIMIT 2",
        )
        .bind(calendar_event_id)
        .fetch_all(pool)
        .await?;
        Ok(if rows.len() == 1 {
            Some(rows[0].clone())
        } else {
            None
        })
    }

    /// Move a `scheduled` row onto a new occurrence start (specs/0064 W1). Refuses a row that
    /// is no longer `scheduled` or that has transcripts, so a recording is never re-dated.
    /// Returns whether the row moved.
    pub async fn redate_scheduled_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
        occurrence_start: DateTime<Utc>,
    ) -> Result<bool, SqlxError> {
        let result = sqlx::query(
            "UPDATE meetings SET created_at = ?1, updated_at = ?2 \
             WHERE id = ?3 AND origin = 'scheduled' \
               AND NOT EXISTS (SELECT 1 FROM transcripts t WHERE t.meeting_id = meetings.id)",
        )
        .bind(occurrence_start)
        .bind(Utc::now())
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Flip a `scheduled` prep row to `recorded` at record start (specs/0036) so its prep
    /// notes + cached brief carry into the recording as one continuous meeting. Only touches
    /// rows still `origin = 'scheduled'` (no-op once already recorded). `created_at` (the
    /// occurrence start) is preserved — the recording keeps the event's date, like Join &
    /// Record. Returns whether a scheduled row was promoted.
    pub async fn promote_scheduled_to_recorded(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now();
        let result = sqlx::query(
            "UPDATE meetings SET origin = 'recorded', updated_at = ? WHERE id = ? AND origin = 'scheduled'",
        )
        .bind(now)
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::test_support::{dt, memory_db};
    use crate::database::repositories::meeting_note::MeetingNotesRepository;

    #[tokio::test]
    async fn scheduled_meeting_upsert_is_idempotent_per_occurrence_and_adopts() {
        let pool = memory_db().await;
        let occ = dt("2026-07-10T15:00:00Z");

        let a = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            "evt-1",
            Some("series-X"),
            "Standup",
            occ,
        )
        .await
        .unwrap();
        // Same event + same day → same row.
        let b = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            "evt-1",
            Some("series-X"),
            "Standup",
            dt("2026-07-10T15:00:00Z"),
        )
        .await
        .unwrap();
        assert_eq!(a.id(), b.id(), "same occurrence returns the same scheduled row");

        // Same event, DIFFERENT day (next occurrence of the recurring series) → a new row,
        // disambiguated by day even though EventKit shares the event id.
        let next = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            "evt-1",
            Some("series-X"),
            "Standup",
            dt("2026-07-17T15:00:00Z"),
        )
        .await
        .unwrap();
        assert_ne!(a.id(), next.id());

        // The scheduled row is dated to the occurrence start and carries the series key.
        let meta = MeetingsRepository::get_meeting_metadata(&pool, a.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.origin, "scheduled");
        assert_eq!(meta.calendar_series_key.as_deref(), Some("series-X"));

        // Adoption: found for this occurrence, promoted to recorded, id preserved.
        let found = MeetingsRepository::find_scheduled_for_occurrence(&pool, "evt-1", occ)
            .await
            .unwrap();
        assert_eq!(found.as_deref(), Some(a.id()));
        assert!(MeetingsRepository::promote_scheduled_to_recorded(&pool, a.id())
            .await
            .unwrap());
        // Promotion is a no-op the second time (already recorded).
        assert!(
            !MeetingsRepository::promote_scheduled_to_recorded(&pool, a.id())
                .await
                .unwrap()
        );
        let meta = MeetingsRepository::get_meeting_metadata(&pool, a.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.origin, "recorded");
        // Once recorded it's no longer an adoptable scheduled row.
        assert_eq!(
            MeetingsRepository::find_scheduled_for_occurrence(&pool, "evt-1", occ)
                .await
                .unwrap(),
            None
        );
    }

    /// specs/0064 W1 — a Google instance id names ONE occurrence and survives a move, so the
    /// prep row follows the meeting to its new day instead of being stranded.
    #[tokio::test]
    async fn google_occurrence_is_redated_across_a_day_move() {
        let pool = memory_db().await;
        let ev = "gcal:primary/recur123_20260710T150000Z";
        let first = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            ev,
            Some("series-X"),
            "Standup",
            dt("2026-07-10T15:00:00Z"),
        )
        .await
        .unwrap();
        assert!(matches!(first, ScheduledResolution::Created(_)));

        // The user writes prep against that row.
        MeetingNotesRepository::upsert_prep_notes(&pool, first.id(), Some("- cover roadmap"), None)
            .await
            .unwrap();

        // The organiser moves the meeting to the 12th; Google keeps the instance id.
        let moved = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            ev,
            Some("series-X"),
            "Standup",
            dt("2026-07-12T16:00:00Z"),
        )
        .await
        .unwrap();

        assert_eq!(moved.id(), first.id(), "the same row follows the meeting");
        assert!(moved.was_redated());
        let notes = MeetingNotesRepository::get_notes(&pool, moved.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(notes.prep_markdown.as_deref(), Some("- cover roadmap"));
        let meta = MeetingsRepository::get_meeting_metadata(&pool, moved.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            meta.created_at.0,
            dt("2026-07-12T16:00:00Z"),
            "dated to the new slot"
        );
    }

    /// A move WITHIN the day already resolved to the same row before 0064; it must not be
    /// reported as a re-date (the caller invalidates the cached brief on one).
    #[tokio::test]
    async fn google_move_within_the_same_day_is_not_a_redate() {
        let pool = memory_db().await;
        let ev = "gcal:primary/evt-plain";
        let a = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            ev,
            None,
            "1:1",
            dt("2026-07-10T15:00:00Z"),
        )
        .await
        .unwrap();
        let b = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            ev,
            None,
            "1:1",
            dt("2026-07-10T17:00:00Z"),
        )
        .await
        .unwrap();
        assert_eq!(a.id(), b.id());
        assert!(matches!(b, ScheduledResolution::Existing(_)));
    }

    /// A row that has been recorded is a recording, not a placeholder: it keeps its date and
    /// the moved occurrence gets a fresh prep row.
    #[tokio::test]
    async fn a_recorded_row_is_never_redated() {
        let pool = memory_db().await;
        let ev = "gcal:primary/evt-rec";
        let a = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            ev,
            None,
            "Review",
            dt("2026-07-10T15:00:00Z"),
        )
        .await
        .unwrap();
        assert!(
            MeetingsRepository::promote_scheduled_to_recorded(&pool, a.id())
                .await
                .unwrap()
        );

        let next = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            ev,
            None,
            "Review",
            dt("2026-07-12T15:00:00Z"),
        )
        .await
        .unwrap();
        assert_ne!(next.id(), a.id());
        assert!(matches!(next, ScheduledResolution::Created(_)));
        let meta = MeetingsRepository::get_meeting_metadata(&pool, a.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.created_at.0, dt("2026-07-10T15:00:00Z"));
    }

    /// EventKit shares one event id across every occurrence of a recurring series, so the day
    /// must keep pinning the occurrence: a sibling gets its own row, never the other's.
    #[tokio::test]
    async fn eventkit_ids_still_disambiguate_by_day() {
        let pool = memory_db().await;
        let a = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            "evt-1",
            Some("series-X"),
            "Standup",
            dt("2026-07-10T15:00:00Z"),
        )
        .await
        .unwrap();
        let next = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            "evt-1",
            Some("series-X"),
            "Standup",
            dt("2026-07-17T15:00:00Z"),
        )
        .await
        .unwrap();
        assert_ne!(next.id(), a.id(), "a sibling occurrence gets its own row");
        assert!(matches!(next, ScheduledResolution::Created(_)));
    }
}
