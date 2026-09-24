// Tauri commands for macOS calendar access (specs/0008 P2).
//
// Thin wrappers over `calendar::eventkit` that expose calendar status, the
// access prompt, and upcoming meetings to the frontend. Names and the
// camelCase wire shape match existing command conventions (see `api_*`).

use crate::calendar::eventkit::{self, UpcomingMeeting};
use crate::database::repositories::dismissed_calendar_event::DismissedCalendarEventsRepository;

/// Default look-ahead window for upcoming meetings (hours) when the frontend
/// doesn't specify one.
const DEFAULT_WITHIN_HOURS: u32 = 12;

/// Ceiling on `include_started_within_ms`. The in-app start alert asks for 2 minutes; an
/// hour is far past any sane grace and keeps a bad argument from returning the whole day.
const MAX_STARTED_WITHIN_MS: i64 = 60 * 60 * 1000;

/// How far before now the window opens: 0 (today's behaviour) unless the caller asked for
/// recently-started meetings too. Negative values are treated as 0.
fn started_grace_ms(include_started_within_ms: Option<i64>) -> i64 {
    include_started_within_ms
        .unwrap_or(0)
        .clamp(0, MAX_STARTED_WITHIN_MS)
}

/// The `[start, end)` instants of the upcoming window. Pure, for testing.
fn upcoming_window(
    now: chrono::DateTime<chrono::Utc>,
    hours: u32,
    grace_ms: i64,
) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    (
        now - chrono::Duration::milliseconds(grace_ms),
        now + chrono::Duration::hours(i64::from(hours)),
    )
}

/// Current macOS calendar authorization status.
/// One of: "authorized" | "denied" | "notDetermined" | "restricted".
#[tauri::command]
pub async fn api_get_calendar_access_status() -> Result<String, String> {
    let status = eventkit::access_status();
    log::info!("api_get_calendar_access_status -> {status}");
    Ok(status)
}

/// Trigger the macOS calendar permission prompt. Returns whether access was
/// granted. Safe to call repeatedly; the OS only prompts when notDetermined.
#[tauri::command]
pub async fn api_request_calendar_access() -> Result<bool, String> {
    log::info!(
        "api_request_calendar_access called (status before request={})",
        eventkit::access_status()
    );
    let result = eventkit::request_access().await.map_err(|e| {
        format!("Could not request calendar access: {e}. If this persists, grant Nixon calendar access in System Settings > Privacy & Security > Calendars.")
    });
    match &result {
        Ok(granted) => log::info!(
            "api_request_calendar_access -> granted={granted} (status after request={})",
            eventkit::access_status()
        ),
        Err(e) => log::error!("api_request_calendar_access failed: {e}"),
    }
    result
}

/// Upcoming, non-all-day meetings starting from now within `within_hours`
/// (default ~12h), sorted by start time.
///
/// `include_started_within_ms` (specs/0075 W2; frontend `includeStartedWithinMs`) opens the
/// window that far before now, so a meeting that started moments ago is still returned —
/// the in-app "starting now" alert's grace. Omitted, nothing that has started is returned.
///
/// Single active source (specs/0032): while a Google account is connected,
/// ONLY the local Google cache is read (refresh-if-stale first; failures are
/// logged and the last-good cache serves). When not connected, EventKit only —
/// which returns an empty list when calendar access isn't granted (never
/// errors on missing permission).
#[tauri::command]
pub async fn api_get_upcoming_meetings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    within_hours: Option<u32>,
    include_started_within_ms: Option<i64>,
) -> Result<Vec<UpcomingMeeting>, String> {
    let hours = within_hours
        .unwrap_or(DEFAULT_WITHIN_HOURS)
        .clamp(1, 24 * 14);
    let grace_ms = started_grace_ms(include_started_within_ms);
    log::info!("api_get_upcoming_meetings called (within_hours={hours}, grace_ms={grace_ms})");

    let meetings = if crate::calendar::google_is_active_source(&app).await {
        use crate::calendar::google::sync::{sync_if_stale, SyncTrigger};
        sync_if_stale(&app, SyncTrigger::Upcoming).await;
        let (start, end) = upcoming_window(chrono::Utc::now(), hours, grace_ms);
        match crate::calendar::google::sync::db_pool(&app) {
            Some(pool) => {
                crate::calendar::google::sync::cached_upcoming_between(&pool, start, end).await
            }
            None => Vec::new(),
        }
    } else {
        // EventKit reads touch the Objective-C runtime; run off the async
        // executor thread so a slow Calendar store can't stall the tokio worker.
        tokio::task::spawn_blocking(move || eventkit::upcoming_meetings_since(hours, grace_ms))
            .await
            .map_err(|e| format!("Calendar read task failed: {e}"))?
    };

    let total = meetings.len();
    let meetings = drop_dismissed(&app, meetings).await;
    log::info!(
        "api_get_upcoming_meetings -> {} meeting(s) ({} hidden)",
        meetings.len(),
        total - meetings.len()
    );
    Ok(meetings)
}

/// Remove the events the user has hidden from their agenda (specs/0026).
///
/// Added 2026-09-21. This command is what `CalendarAlerts` polls to fire the T-5 prep
/// notification and the T-0 "starting now — Join & Record" notification, and it never read
/// the dismissal set — only `day_agenda` did. So an event you hid from Today (the owner's
/// example: "Lunch") still produced both banners, which is precisely backwards: clearing
/// something off your agenda is how you say you don't want to record it.
///
/// An unresolvable pool means "filter nothing": a notification you didn't want is a far
/// smaller failure than a missing notification for a meeting that is starting.
async fn drop_dismissed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    meetings: Vec<UpcomingMeeting>,
) -> Vec<UpcomingMeeting> {
    let Some(pool) = crate::calendar::google::sync::db_pool(app) else {
        log::warn!("api_get_upcoming_meetings: no DB pool, cannot filter hidden events");
        return meetings;
    };
    let dismissed = match DismissedCalendarEventsRepository::all(&pool).await {
        Ok(set) => set,
        Err(e) => {
            log::warn!("api_get_upcoming_meetings: could not read hidden events ({e}); showing all");
            return meetings;
        }
    };
    if dismissed.is_empty() {
        return meetings;
    }
    meetings
        .into_iter()
        .filter(|m| !crate::calendar::day_agenda::is_event_dismissed(m, &dismissed))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap()
    }

    #[test]
    fn without_a_grace_the_window_opens_at_now() {
        let (start, end) = upcoming_window(now(), 12, started_grace_ms(None));
        assert_eq!(start, now(), "UpcomingMeetings passes nothing: unchanged behaviour");
        assert_eq!(end, now() + chrono::Duration::hours(12));
    }

    #[test]
    fn a_grace_opens_the_window_that_far_before_now() {
        let (start, _) = upcoming_window(now(), 12, started_grace_ms(Some(120_000)));
        assert_eq!(start, now() - chrono::Duration::seconds(120));
    }

    #[test]
    fn a_nonsense_grace_is_clamped() {
        assert_eq!(started_grace_ms(Some(-5_000)), 0);
        assert_eq!(started_grace_ms(Some(i64::MAX)), MAX_STARTED_WITHIN_MS);
    }
}
