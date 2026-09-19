//! The `scheduled`-origin placeholder lifecycle (specs/0036, revised by specs/0064 W1).
//!
//! A `scheduled` row is the backing store for one upcoming calendar occurrence's prep notes
//! and cached brief. It is invisible to every list query (`origin <> 'scheduled'`) until
//! Join & Record promotes it to `recorded`.

use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, SqlitePool};
use uuid::Uuid;

use super::MeetingsRepository;

impl MeetingsRepository {
    /// A `scheduled`-origin prep row for one calendar-event OCCURRENCE (specs/0036): same
    /// `calendar_event_id` AND same UTC calendar day as `occurrence_start`. The day match
    /// disambiguates EventKit's shared-across-series event id (all occurrences of a recurring
    /// series carry the same id, so the day is what pins one occurrence). Used both to make
    /// [`upsert_scheduled_meeting`] idempotent and to adopt the prep row at record start.
    pub async fn find_scheduled_for_occurrence(
        pool: &SqlitePool,
        calendar_event_id: &str,
        occurrence_start: DateTime<Utc>,
    ) -> Result<Option<String>, SqlxError> {
        if calendar_event_id.trim().is_empty() {
            return Ok(None);
        }
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM meetings \
             WHERE calendar_event_id = ?1 AND origin = 'scheduled' \
               AND date(created_at) = date(?2) \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(calendar_event_id)
        .bind(occurrence_start)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    /// Mint (or return the existing) `scheduled` prep row for one occurrence of a calendar
    /// event (specs/0036). Idempotent by `(calendar_event_id, occurrence day)`. The row is
    /// dated to `occurrence_start` (its `created_at`, like Join & Record) and carries the
    /// series key. Prep notes and the cached brief attach to this row; Join & Record later
    /// adopts it (see [`promote_scheduled_to_recorded`]) so it becomes the recording.
    pub async fn upsert_scheduled_meeting(
        pool: &SqlitePool,
        calendar_event_id: &str,
        series_key: Option<&str>,
        title: &str,
        occurrence_start: DateTime<Utc>,
    ) -> Result<String, SqlxError> {
        if let Some(existing) =
            Self::find_scheduled_for_occurrence(pool, calendar_event_id, occurrence_start).await?
        {
            return Ok(existing);
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
        Ok(meeting_id)
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
        assert_eq!(a, b, "same occurrence returns the same scheduled row");

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
        assert_ne!(a, next);

        // The scheduled row is dated to the occurrence start and carries the series key.
        let meta = MeetingsRepository::get_meeting_metadata(&pool, &a)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.origin, "scheduled");
        assert_eq!(meta.calendar_series_key.as_deref(), Some("series-X"));

        // Adoption: found for this occurrence, promoted to recorded, id preserved.
        let found = MeetingsRepository::find_scheduled_for_occurrence(&pool, "evt-1", occ)
            .await
            .unwrap();
        assert_eq!(found.as_deref(), Some(a.as_str()));
        assert!(MeetingsRepository::promote_scheduled_to_recorded(&pool, &a)
            .await
            .unwrap());
        // Promotion is a no-op the second time (already recorded).
        assert!(
            !MeetingsRepository::promote_scheduled_to_recorded(&pool, &a)
                .await
                .unwrap()
        );
        let meta = MeetingsRepository::get_meeting_metadata(&pool, &a)
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
}
