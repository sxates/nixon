//! Series-scoped deletes for the Google Calendar event cache (specs/0054 W5).
//!
//! A second `impl GoogleCalendarRepository` block rather than an edit to
//! `google_calendar.rs`, which is pinned at its current size by the specs/0042
//! file-size ratchet and may only shrink.

use sqlx::{Error as SqlxError, SqlitePool};

use super::google_calendar::GoogleCalendarRepository;

/// Escape SQLite `LIKE` metacharacters so a literal prefix stays literal.
///
/// Load-bearing here, not defensive boilerplate: expanded-occurrence ids are
/// literally `<seriesId>_<timestamp>`, so an UNescaped `_` would be a single-char
/// wildcard and `recur1_%` would also match `recur1X…`. Backslash first, so the
/// escapes we add below are not themselves escaped.
fn escape_like(value: &str) -> String {
    value
        .replace('\\', r"\\")
        .replace('%', r"\%")
        .replace('_', r"\_")
}

impl GoogleCalendarRepository {
    /// Delete every cached row belonging to one recurring series: the master row
    /// (`gcal:<cal>/<series>`) and all expanded occurrences
    /// (`gcal:<cal>/<series>_<instanceTs>`). Returns the number of rows removed.
    ///
    /// Scoped by `calendar_id` as well as the id prefix so a series id that
    /// happens to prefix another calendar's ids cannot reach across.
    pub async fn delete_events_for_series(
        pool: &SqlitePool,
        calendar_id: &str,
        series_id: &str,
    ) -> Result<u64, SqlxError> {
        let exact = format!("gcal:{calendar_id}/{series_id}");
        let instances = format!("{}\\_%", escape_like(&exact));
        let result = sqlx::query(
            r"DELETE FROM google_calendar_events
              WHERE calendar_id = ?
                AND (id = ? OR id LIKE ? ESCAPE '\')",
        )
        .bind(calendar_id)
        .bind(&exact)
        .bind(&instances)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::google_calendar::GoogleCalendarEventRow;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    fn row(id: &str, calendar_id: &str) -> GoogleCalendarEventRow {
        GoogleCalendarEventRow {
            id: id.to_string(),
            calendar_id: calendar_id.to_string(),
            ical_uid: None,
            title: "T".into(),
            starts_at: "2026-07-02T09:00:00+00:00".into(),
            ends_at: "2026-07-02T10:00:00+00:00".into(),
            is_all_day: false,
            location: None,
            zoom_url: None,
            organizer_email: None,
            my_response: Some("accepted".into()),
            attendees_json: "[]".into(),
            status: "confirmed".into(),
            updated_at: "2026-07-01T00:00:00+00:00".into(),
        }
    }

    async fn ids(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar::<_, String>("SELECT id FROM google_calendar_events ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn removes_the_master_and_every_expanded_occurrence() {
        let pool = pool().await;
        for r in [
            row("gcal:primary/recur1", "primary"),
            row("gcal:primary/recur1_20260703T070000Z", "primary"),
            row("gcal:primary/recur1_20260704T070000Z", "primary"),
            row("gcal:primary/other", "primary"),
        ] {
            GoogleCalendarRepository::upsert_event(&pool, &r)
                .await
                .unwrap();
        }

        let removed =
            GoogleCalendarRepository::delete_events_for_series(&pool, "primary", "recur1")
                .await
                .unwrap();

        assert_eq!(removed, 3, "master + two occurrences");
        assert_eq!(ids(&pool).await, vec!["gcal:primary/other".to_string()]);
    }

    /// The `_` in `<series>_<ts>` is a LIKE wildcard unless escaped, so an
    /// unescaped prefix would also delete a DIFFERENT series whose id merely
    /// shares the first characters.
    #[tokio::test]
    async fn does_not_reach_a_series_whose_id_merely_shares_a_prefix() {
        let pool = pool().await;
        for r in [
            row("gcal:primary/recur1_20260703T070000Z", "primary"),
            row("gcal:primary/recur1X_20260703T070000Z", "primary"),
            row("gcal:primary/recur12_20260703T070000Z", "primary"),
        ] {
            GoogleCalendarRepository::upsert_event(&pool, &r)
                .await
                .unwrap();
        }

        let removed =
            GoogleCalendarRepository::delete_events_for_series(&pool, "primary", "recur1")
                .await
                .unwrap();

        assert_eq!(removed, 1, "only the exact series' occurrence");
        assert_eq!(
            ids(&pool).await,
            vec![
                "gcal:primary/recur12_20260703T070000Z".to_string(),
                "gcal:primary/recur1X_20260703T070000Z".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn never_crosses_calendars() {
        let pool = pool().await;
        for r in [
            row("gcal:primary/recur1_20260703T070000Z", "primary"),
            row("gcal:work/recur1_20260703T070000Z", "work"),
        ] {
            GoogleCalendarRepository::upsert_event(&pool, &r)
                .await
                .unwrap();
        }

        let removed =
            GoogleCalendarRepository::delete_events_for_series(&pool, "primary", "recur1")
                .await
                .unwrap();

        assert_eq!(removed, 1);
        assert_eq!(
            ids(&pool).await,
            vec!["gcal:work/recur1_20260703T070000Z".to_string()]
        );
    }
}
