// Calendar corroboration for a browser call (specs/0074 W6, task 26).
//
// A browser holding the microphone is only offered as a meeting when a timed calendar event
// is in progress, and never for an event the user hid from their agenda (specs/0026). The
// events come from the single active source (specs/0032): the local Google cache while an
// account is connected, EventKit otherwise. Neither path triggers a sync, a network call or a
// permission prompt: EventKit returns nothing without access, and the Google cache is
// whatever the background sync last wrote.
//
// Reads are cached for `CACHE_TTL`, and the monitor asks only while a browser is on the
// microphone, so an idle machine never touches the calendar from here.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Runtime};

use crate::calendar::day_agenda::is_event_dismissed;
use crate::calendar::eventkit::{self, UpcomingMeeting};
use crate::database::repositories::dismissed_calendar_event::DismissedCalendarEventsRepository;

/// A call may be joined this long before its event starts.
const EARLY_JOIN: chrono::Duration = chrono::Duration::minutes(10);

/// How far back to look for an event that started earlier and is still running.
const LOOKBACK: chrono::Duration = chrono::Duration::hours(3);

/// How long one calendar read serves the 3-second poll.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Pure: is a non-hidden event in progress at `now` (from 10 minutes before its start to its
/// end)? All-day events never reach here: both calendar readers drop them.
pub fn event_in_progress(
    events: &[UpcomingMeeting],
    dismissed: &HashSet<String>,
    now: DateTime<Utc>,
) -> bool {
    events.iter().any(|e| {
        let (Some(start), Some(end)) = (parse(&e.starts_at), parse(&e.ends_at)) else {
            return false;
        };
        start - EARLY_JOIN <= now && now <= end && !is_event_dismissed(e, dismissed)
    })
}

fn parse(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// The monitor's calendar reader, with a short cache.
#[derive(Default)]
pub struct CalendarProbe {
    cached: Option<(Instant, Vec<UpcomingMeeting>, HashSet<String>)>,
}

impl CalendarProbe {
    /// Is a matching calendar event in progress right now? Best effort: any read failure is
    /// "no", which means no prompt for a browser call, never a wrong one.
    pub async fn event_now<R: Runtime>(&mut self, app: &AppHandle<R>) -> bool {
        let fresh = matches!(&self.cached, Some((at, _, _)) if at.elapsed() < CACHE_TTL);
        if !fresh {
            let (events, dismissed) = read(app).await;
            self.cached = Some((Instant::now(), events, dismissed));
        }
        let Some((_, events, dismissed)) = &self.cached else {
            return false;
        };
        event_in_progress(events, dismissed, Utc::now())
    }
}

async fn read<R: Runtime>(app: &AppHandle<R>) -> (Vec<UpcomingMeeting>, HashSet<String>) {
    let now = Utc::now();
    let (start, end) = (now - LOOKBACK, now + EARLY_JOIN);
    let pool = crate::calendar::google::sync::db_pool(app);
    let events = if crate::calendar::google_is_active_source(app).await {
        match &pool {
            Some(pool) => {
                crate::calendar::google::sync::cached_upcoming_between(pool, start, end).await
            }
            None => Vec::new(),
        }
    } else {
        tokio::task::spawn_blocking(move || eventkit::meetings_between(start, end))
            .await
            .unwrap_or_default()
    };
    let dismissed = match &pool {
        Some(pool) => DismissedCalendarEventsRepository::all(pool)
            .await
            .unwrap_or_else(|e| {
                log::warn!("meeting detect: could not read hidden events ({e})");
                HashSet::new()
            }),
        None => HashSet::new(),
    };
    (events, dismissed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, start: &str, end: &str) -> UpcomingMeeting {
        UpcomingMeeting {
            id: id.to_string(),
            title: "Weekly sync".to_string(),
            starts_at: start.to_string(),
            ends_at: end.to_string(),
            calendar_name: "Work".to_string(),
            location: None,
            zoom_url: None,
            external_id: None,
        }
    }

    fn at(s: &str) -> DateTime<Utc> {
        parse(s).unwrap()
    }

    const START: &str = "2026-09-22T15:00:00+00:00";
    const END: &str = "2026-09-22T15:30:00+00:00";

    #[test]
    fn an_event_in_progress_counts() {
        let events = [event("e1", START, END)];
        let none = HashSet::new();
        assert!(event_in_progress(
            &events,
            &none,
            at("2026-09-22T15:10:00Z")
        ));
        assert!(event_in_progress(
            &events,
            &none,
            at("2026-09-22T15:30:00Z")
        ));
    }

    #[test]
    fn joining_up_to_ten_minutes_early_counts() {
        let events = [event("e1", START, END)];
        let none = HashSet::new();
        assert!(event_in_progress(
            &events,
            &none,
            at("2026-09-22T14:50:00Z")
        ));
        assert!(!event_in_progress(
            &events,
            &none,
            at("2026-09-22T14:49:00Z")
        ));
    }

    #[test]
    fn a_finished_event_does_not_count() {
        let events = [event("e1", START, END)];
        let none = HashSet::new();
        assert!(!event_in_progress(
            &events,
            &none,
            at("2026-09-22T15:31:00Z")
        ));
    }

    #[test]
    fn a_hidden_event_does_not_count() {
        let events = [event("e1", START, END)];
        let hidden: HashSet<String> = ["e1".to_string()].into();
        assert!(!event_in_progress(
            &events,
            &hidden,
            at("2026-09-22T15:10:00Z")
        ));
    }

    #[test]
    fn a_hidden_event_does_not_hide_another_in_progress() {
        let events = [event("e1", START, END), event("e2", START, END)];
        let hidden: HashSet<String> = ["e1".to_string()].into();
        assert!(event_in_progress(
            &events,
            &hidden,
            at("2026-09-22T15:10:00Z")
        ));
    }

    #[test]
    fn an_unparseable_event_does_not_count() {
        let events = [event("e1", "not a date", END)];
        assert!(!event_in_progress(
            &events,
            &HashSet::new(),
            at("2026-09-22T15:10:00Z")
        ));
    }
}
