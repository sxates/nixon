//! `dismissed_calendar_events` table access (specs/0026) — the set of calendar events the user
//! has hidden from the home agenda.
//!
//! Keyed by the EventKit event id exactly as the agenda surfaces it (`DayAgendaItem.id` for an
//! unrecorded calendar row, or the synthetic `evt-<hash>` when EventKit gives no id), so a
//! dismissal from the UI round-trips by the same key the agenda builder checks. Idempotent
//! add/remove; mirrors the sibling repos (returns `SqlxError`, command layer maps to strings).

use std::collections::HashSet;

use chrono::Utc;
use sqlx::{Error as SqlxError, SqlitePool};

pub struct DismissedCalendarEventsRepository;

impl DismissedCalendarEventsRepository {
    /// Hide an event. Idempotent (re-dismissing refreshes `dismissed_at`).
    pub async fn dismiss(pool: &SqlitePool, event_id: &str) -> Result<(), SqlxError> {
        if event_id.trim().is_empty() {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO dismissed_calendar_events (event_id, dismissed_at) VALUES (?, ?) \
             ON CONFLICT(event_id) DO UPDATE SET dismissed_at = excluded.dismissed_at",
        )
        .bind(event_id)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Un-hide an event. Idempotent (no-op if it wasn't dismissed).
    pub async fn undismiss(pool: &SqlitePool, event_id: &str) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM dismissed_calendar_events WHERE event_id = ?")
            .bind(event_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// All dismissed event ids, as a set for O(1) membership while building the agenda.
    pub async fn all(pool: &SqlitePool) -> Result<HashSet<String>, SqlxError> {
        let rows =
            sqlx::query_scalar::<_, String>("SELECT event_id FROM dismissed_calendar_events")
                .fetch_all(pool)
                .await?;
        Ok(rows.into_iter().collect())
    }
}
