//! specs/0069 W3 — a meeting you add inside Nixon.
//!
//! It is deliberately NOT a new kind of row. `scheduled`-origin rows already exist as prep
//! placeholders for calendar occurrences (specs/0036): they carry prep notes and a cached
//! brief, they are invisible to every list query, and `promote_scheduled_to_recorded` turns
//! one into the recording at record start with all of that intact. A manual entry is that
//! row with a Nixon-minted `calendar_event_id` instead of an EventKit/Google one, so prep,
//! adoption and promotion are the paths that already work, not new ones.
//!
//! The prefix is the whole identity trick, and it is load-bearing in two more places:
//! `is_per_occurrence_event_id` (a manual id names exactly one entry) and
//! `get_manual_scheduled_between` (which must return manual rows and only manual rows, so
//! the Day Agenda never merges a calendar-backed prep row and claims its own event as
//! recorded — the reason `get_between_with_status` excludes `scheduled` at all).

use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, SqlitePool};
use uuid::Uuid;

use super::MeetingsRepository;
use crate::database::models::ManualScheduledRow;

/// Marks a `calendar_event_id` that Nixon minted for a manually added meeting.
pub const MANUAL_EVENT_PREFIX: &str = "nixon-manual:";

pub fn is_manual_event_id(calendar_event_id: &str) -> bool {
    calendar_event_id.starts_with(MANUAL_EVENT_PREFIX)
}

impl MeetingsRepository {
    /// Mint a manual scheduled row. `created_at` carries the occurrence START, exactly as
    /// it does for a calendar-backed scheduled row.
    pub async fn create_manual_scheduled(
        pool: &SqlitePool,
        title: &str,
        starts_at: DateTime<Utc>,
        ends_at: Option<DateTime<Utc>>,
        join_url: Option<&str>,
    ) -> Result<String, SqlxError> {
        let meeting_id = format!("meeting-{}", Uuid::new_v4());
        let event_id = format!("{MANUAL_EVENT_PREFIX}{}", Uuid::new_v4());
        let title = {
            let t = title.trim();
            if t.is_empty() { "New Meeting" } else { t }
        };
        let join_url = join_url.map(str::trim).filter(|u| !u.is_empty());
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO meetings \
             (id, title, created_at, updated_at, origin, calendar_event_id, scheduled_end_at, join_url) \
             VALUES (?, ?, ?, ?, 'scheduled', ?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(title)
        .bind(starts_at)
        .bind(now)
        .bind(&event_id)
        .bind(ends_at)
        .bind(join_url)
        .execute(pool)
        .await?;
        Ok(meeting_id)
    }

    /// Edit one. Refused (`Ok(false)`) once the row has been recorded — at that point it is
    /// a meeting, and its title and date belong to the meeting page.
    pub async fn update_manual_scheduled(
        pool: &SqlitePool,
        meeting_id: &str,
        title: &str,
        starts_at: DateTime<Utc>,
        ends_at: Option<DateTime<Utc>>,
        join_url: Option<&str>,
    ) -> Result<bool, SqlxError> {
        let title = {
            let t = title.trim();
            if t.is_empty() { "New Meeting" } else { t }
        };
        let join_url = join_url.map(str::trim).filter(|u| !u.is_empty());
        let result = sqlx::query(
            "UPDATE meetings \
                SET title = ?, created_at = ?, scheduled_end_at = ?, join_url = ?, updated_at = ? \
              WHERE id = ? AND origin = 'scheduled' \
                AND calendar_event_id LIKE ?",
        )
        .bind(title)
        .bind(starts_at)
        .bind(ends_at)
        .bind(join_url)
        .bind(Utc::now())
        .bind(meeting_id)
        .bind(format!("{MANUAL_EVENT_PREFIX}%"))
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete one. Same refusal: a recorded meeting is deleted through the normal delete
    /// path, which also cleans up its transcripts, summary and folder.
    pub async fn delete_manual_scheduled(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<bool, SqlxError> {
        let result = sqlx::query(
            "DELETE FROM meetings \
              WHERE id = ? AND origin = 'scheduled' AND calendar_event_id LIKE ?",
        )
        .bind(meeting_id)
        .bind(format!("{MANUAL_EVENT_PREFIX}%"))
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Manual entries whose occurrence start falls in `[start_utc, end_utc)`, time-ordered.
    pub async fn get_manual_scheduled_between(
        pool: &SqlitePool,
        start_utc: DateTime<Utc>,
        end_utc: DateTime<Utc>,
    ) -> Result<Vec<ManualScheduledRow>, SqlxError> {
        sqlx::query_as::<_, ManualScheduledRow>(
            "SELECT id, title, created_at, scheduled_end_at, join_url, calendar_event_id \
               FROM meetings \
              WHERE origin = 'scheduled' AND calendar_event_id LIKE ?1 \
                AND created_at >= ?2 AND created_at < ?3 \
              ORDER BY created_at ASC",
        )
        .bind(format!("{MANUAL_EVENT_PREFIX}%"))
        .bind(start_utc)
        .bind(end_utc)
        .fetch_all(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::test_support::{dt, memory_db};

    #[tokio::test]
    async fn a_manual_meeting_is_a_scheduled_row_with_a_nixon_event_id() {
        let pool = memory_db().await;
        let id = MeetingsRepository::create_manual_scheduled(
            &pool,
            "  Call with Sam  ",
            dt("2026-09-20T15:00:00Z"),
            Some(dt("2026-09-20T15:30:00Z")),
            Some("https://zoom.us/j/1"),
        )
        .await
        .unwrap();

        let (title, origin, event_id): (String, String, String) = sqlx::query_as(
            "SELECT title, origin, calendar_event_id FROM meetings WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(title, "Call with Sam", "the title is trimmed");
        assert_eq!(origin, "scheduled");
        assert!(is_manual_event_id(&event_id));
    }

    #[tokio::test]
    async fn it_lands_in_the_day_window_by_its_start_not_its_creation_time() {
        let pool = memory_db().await;
        let id = MeetingsRepository::create_manual_scheduled(
            &pool,
            "Tomorrow",
            dt("2026-09-21T09:00:00Z"),
            None,
            None,
        )
        .await
        .unwrap();

        let today = MeetingsRepository::get_manual_scheduled_between(
            &pool,
            dt("2026-09-20T00:00:00Z"),
            dt("2026-09-21T00:00:00Z"),
        )
        .await
        .unwrap();
        assert!(today.is_empty(), "it is not today's");

        let tomorrow = MeetingsRepository::get_manual_scheduled_between(
            &pool,
            dt("2026-09-21T00:00:00Z"),
            dt("2026-09-22T00:00:00Z"),
        )
        .await
        .unwrap();
        assert_eq!(tomorrow.len(), 1);
        assert_eq!(tomorrow[0].id, id);
        assert!(tomorrow[0].scheduled_end_at.is_none());
    }

    #[tokio::test]
    async fn a_calendar_backed_scheduled_row_is_never_returned() {
        let pool = memory_db().await;
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, origin, calendar_event_id) \
             VALUES ('m-cal', 'From the calendar', ?1, ?1, 'scheduled', 'gcal:cal/evt_1')",
        )
        .bind(dt("2026-09-20T10:00:00Z"))
        .execute(&pool)
        .await
        .unwrap();

        let rows = MeetingsRepository::get_manual_scheduled_between(
            &pool,
            dt("2026-09-20T00:00:00Z"),
            dt("2026-09-21T00:00:00Z"),
        )
        .await
        .unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn once_recorded_it_can_no_longer_be_edited_or_deleted_as_a_manual_entry() {
        let pool = memory_db().await;
        let id = MeetingsRepository::create_manual_scheduled(
            &pool,
            "Call with Sam",
            dt("2026-09-20T15:00:00Z"),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(MeetingsRepository::promote_scheduled_to_recorded(&pool, &id)
            .await
            .unwrap());

        assert!(
            !MeetingsRepository::update_manual_scheduled(
                &pool,
                &id,
                "Renamed",
                dt("2026-09-20T16:00:00Z"),
                None,
                None,
            )
            .await
            .unwrap(),
            "it is a recording now — renaming it belongs to the meeting page"
        );
        assert!(!MeetingsRepository::delete_manual_scheduled(&pool, &id)
            .await
            .unwrap());

        let still_there: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM meetings WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(still_there, 1, "a recording is never deleted by this path");
    }

    #[tokio::test]
    async fn editing_moves_the_row_to_its_new_time() {
        let pool = memory_db().await;
        let id = MeetingsRepository::create_manual_scheduled(
            &pool,
            "Call with Sam",
            dt("2026-09-20T15:00:00Z"),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(MeetingsRepository::update_manual_scheduled(
            &pool,
            &id,
            "Call with Sam",
            dt("2026-09-21T11:00:00Z"),
            Some(dt("2026-09-21T11:45:00Z")),
            Some("https://zoom.us/j/2"),
        )
        .await
        .unwrap());

        let rows = MeetingsRepository::get_manual_scheduled_between(
            &pool,
            dt("2026-09-21T00:00:00Z"),
            dt("2026-09-22T00:00:00Z"),
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].join_url.as_deref(), Some("https://zoom.us/j/2"));
    }

    #[test]
    fn a_manual_id_names_one_occurrence() {
        // Otherwise adopting the prep row at record start falls back to "same UTC day",
        // which breaks across midnight and after a re-date (specs/0069 W3).
        assert!(crate::database::repositories::meeting::is_per_occurrence_event_id(
            "nixon-manual:abc"
        ));
    }
}
