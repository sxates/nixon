//! Recorded meetings within an arbitrary date range, for the month calendar
//! (specs/0054 W3).
//!
//! The meetings list is a single chronological scroll, which stops being a way to
//! find anything once a few months of recordings accumulate. The month grid needs
//! one bounded query per displayed month rather than the whole table, so this wraps
//! the existing [`MeetingsRepository::get_between_with_status`] — the same query the
//! Day Agenda uses — with a caller-supplied range.
//!
//! Recorded meetings ONLY (owner's choice): the Google event cache holds ~1,500
//! rows against dozens of recordings, so merging calendar events would bury the
//! recordings the grid exists to surface.

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Runtime};

use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;

/// One recorded meeting as a month-grid cell renders it. Deliberately smaller than
/// the dashboard's `Meeting`: the grid shows a title chip and needs to know whether
/// the meeting is worth opening, nothing more.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonthMeeting {
    pub id: String,
    pub title: String,
    /// RFC3339 UTC. The frontend buckets by LOCAL day, so it must not be
    /// pre-formatted here.
    pub created_at: String,
    pub duration_seconds: Option<f64>,
    /// Drives the "has something to read" affordance on the chip.
    pub has_summary: bool,
}

/// Parse an ISO-8601 bound, naming which one failed so a frontend bug is
/// diagnosable from the error alone.
fn parse_bound(label: &str, value: &str) -> Result<DateTime<Utc>, String> {
    value
        .parse::<DateTime<Utc>>()
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| format!("Invalid {label} bound '{value}': {e}"))
}

/// Recorded meetings whose `created_at` falls in `[start, end)`.
///
/// The frontend passes the UTC instants bracketing a displayed month in the
/// user's local timezone, so month boundaries land correctly regardless of offset.
#[tauri::command]
pub async fn api_get_meetings_in_range<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    start: String,
    end: String,
) -> Result<Vec<MonthMeeting>, String> {
    let start_utc = parse_bound("start", &start)?;
    let end_utc = parse_bound("end", &end)?;
    if end_utc <= start_utc {
        return Err(format!("Range end '{end}' must be after start '{start}'"));
    }

    let pool = state.db_manager.pool();
    let rows = MeetingsRepository::get_between_with_status(pool, start_utc, end_utc)
        .await
        .map_err(|e| format!("Failed to load meetings for the range: {e}"))?;

    log::info!(
        "api_get_meetings_in_range -> {} meeting(s) in [{start}, {end})",
        rows.len()
    );

    Ok(rows
        .into_iter()
        .map(|r| MonthMeeting {
            id: r.id,
            title: r.title,
            created_at: r.created_at.0.to_rfc3339(),
            duration_seconds: r.duration_seconds,
            has_summary: r.has_summary != 0,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bound_accepts_rfc3339_and_names_the_failing_bound() {
        assert!(parse_bound("start", "2026-08-01T00:00:00+00:00").is_ok());
        assert!(parse_bound("start", "2026-08-01T00:00:00Z").is_ok());
        // A local-offset instant normalizes to UTC rather than being rejected.
        let offset = parse_bound("end", "2026-08-01T00:00:00-07:00").unwrap();
        assert_eq!(offset.to_rfc3339(), "2026-08-01T07:00:00+00:00");

        let err = parse_bound("end", "not-a-date").unwrap_err();
        assert!(
            err.contains("end"),
            "the error must name which bound: {err}"
        );
        assert!(err.contains("not-a-date"));
    }

    /// A date-only string is NOT a valid instant — catching it here beats a silent
    /// empty month in the UI.
    #[test]
    fn parse_bound_rejects_a_date_without_a_time() {
        assert!(parse_bound("start", "2026-08-01").is_err());
    }
}
