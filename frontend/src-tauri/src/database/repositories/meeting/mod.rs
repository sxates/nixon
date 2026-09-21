//! The meetings repository, split by concern (specs/0042 WS4):
//! - [`crud`]: single-row create/read/update/delete.
//! - [`manual`]: manually added meetings (specs/0069 W3).
//! - [`scheduled`]: the `scheduled`-origin prep-placeholder lifecycle (specs/0036, 0064 W1).
//! - [`query`]: list shaping, enriched/status rows, pagination, title-based suggestion.
//! - [`series`]: recurring-series keys, manual links, and prior-occurrence matching.
//!
//! The public surface is unchanged: everything is re-exported here so callers keep
//! importing from `crate::database::repositories::meeting`.

mod crud;
mod manual;
mod query;
mod scheduled;
mod series;

pub use manual::{is_manual_event_id, MANUAL_EVENT_PREFIX};
pub use query::{normalize_title, RecentPersonMeeting};
pub use scheduled::{is_per_occurrence_event_id, ScheduledResolution};
pub use series::SeriesLinkedMeeting;

pub struct MeetingsRepository;

/// Shared fixtures for the meeting repository's unit tests (crud/query/series).
#[cfg(test)]
pub(crate) mod test_support {
    use chrono::{DateTime, Utc};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;
    use uuid::Uuid;

    /// Fresh in-memory SQLite brought up through the app's real migration set
    /// (mirrors tests/db_lifecycle.rs, minus the file-based DatabaseManager).
    /// One connection max — each in-memory connection is a separate database.
    pub(crate) async fn memory_db() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
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

    /// Insert a recorded meeting dated to `created_at` (RFC3339) with an optional series key,
    /// then give it a completed summary so it passes the `find_prior_series_occurrences`
    /// content guard.
    pub(crate) async fn recorded_with_summary(
        pool: &SqlitePool,
        title: &str,
        created_at: &str,
        series_key: Option<&str>,
    ) -> String {
        let id = format!("meeting-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, origin, calendar_series_key) \
             VALUES (?, ?, ?, ?, 'recorded', ?)",
        )
        .bind(&id)
        .bind(title)
        .bind(created_at)
        .bind(created_at)
        .bind(series_key)
        .execute(pool)
        .await
        .expect("insert recorded meeting");
        sqlx::query(
            "INSERT INTO summary_processes (meeting_id, status, result, created_at, updated_at) \
             VALUES (?, 'completed', '{\"markdown\":\"x\"}', ?, ?)",
        )
        .bind(&id)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("insert summary");
        id
    }

    pub(crate) fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
}
