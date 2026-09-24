//! Day Agenda — a persistent, whole-day, unified meeting list (specs/0012).
//!
//! Replaces the ephemeral "upcoming meetings" strip (which dropped events once
//! their start time passed) with a single time-ordered list of TODAY's items:
//! calendar events (optionally linked to a recording) merged with ad-hoc
//! recordings that have no calendar event. Each item carries attendees and a
//! per-meeting processing status so the UI can show progress and offer one-click
//! "Summarize" / "Identify speakers" actions without opening the meeting.
//!
//! Best-effort calendar: if calendar access is denied or EventKit errors, the
//! agenda still returns today's recorded meetings (calendar layer logs + returns
//! empty rather than failing).

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

use crate::calendar::eventkit::{self, Attendee, UpcomingMeeting};
use crate::database::models::MeetingStatusRow;
use crate::database::repositories::dismissed_calendar_event::DismissedCalendarEventsRepository;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::meeting_participant::{
    AttendeePreviewRow, MeetingParticipantsRepository,
};
use crate::state::AppState;

/// Max attendees embedded inline per item (the full count is `attendee_count`).
const MAX_INLINE_ATTENDEES: usize = 5;

/// How far a recorded meeting's start may sit from a calendar event's start and
/// still be considered the same meeting (recordings often begin late). Mirrors the
/// ±-window approach used by `eventkit::event_attendees`, but tighter so two
/// back-to-back calendar events don't both claim the same recording.
const MATCH_WINDOW: Duration = Duration::minutes(90);

/// The source of an agenda item.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgendaSource {
    /// A calendar event (possibly linked to a recorded meeting).
    Calendar,
    /// An ad-hoc recording with no matching calendar event.
    Recording,
    /// A meeting the user added inside Nixon (specs/0069 W3) — a scheduled row with
    /// no calendar event.
    Manual,
}

/// Which calendar the agenda's calendar rows came from (specs/0075 W3). The frontend's
/// anti-flicker fallback (keep the previous calendar rows when a read has none) only
/// makes sense for EventKit, whose cold reads can come back empty; Google reads a local
/// cache and an empty read there is the truth.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CalendarSource {
    EventKit,
    Google,
}

/// `api_get_day_agenda`'s result: the items plus the calendar source they were read from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayAgenda {
    pub items: Vec<DayAgendaItem>,
    pub calendar_source: CalendarSource,
}

/// Per-meeting processing status flags (specs/0012). All four are best-effort
/// derived from the DB in one query (see `MeetingsRepository::get_between_with_status`).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgendaStatus {
    /// Has a recording folder/audio.
    pub recorded: bool,
    /// Has ≥1 transcript segment.
    pub transcribed: bool,
    /// Has a generated summary.
    pub summarized: bool,
    /// Has ≥1 diarized speaker row.
    pub speakers_identified: bool,
}

impl AgendaStatus {
    pub(crate) fn empty() -> Self {
        Self {
            recorded: false,
            transcribed: false,
            summarized: false,
            speakers_identified: false,
        }
    }

    fn from_row(row: &MeetingStatusRow) -> Self {
        Self {
            recorded: row.has_folder != 0,
            transcribed: row.has_transcript != 0,
            summarized: row.has_summary != 0,
            speakers_identified: row.has_speakers != 0,
        }
    }
}

/// One unified agenda item. Serialized camelCase for the frontend; the field
/// names below are the exact wire contract (specs/0012).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayAgendaItem {
    /// Stable key: the recorded `meeting_id` when present, else the calendar
    /// event identifier (or a synthetic `evt-<hash>` fallback).
    pub id: String,
    pub title: String,
    /// ISO-8601 start: the event start, or the recording start for ad-hoc items.
    pub start_time: String,
    /// ISO-8601 end, when known.
    pub end_time: Option<String>,
    pub source: AgendaSource,
    /// Detected join link (Zoom), when present.
    pub zoom_url: Option<String>,
    /// First few attendees (cap `MAX_INLINE_ATTENDEES`); empty for ad-hoc recordings.
    pub attendees: Vec<Attendee>,
    /// Total attendee count (may exceed `attendees.len()`).
    pub attendee_count: u32,
    /// Recorded meeting row id when this item is/links to a recording.
    pub meeting_id: Option<String>,
    pub status: AgendaStatus,
    /// User has hidden this calendar event from the agenda (specs/0026). The frontend
    /// filters these out by default and reveals them under "Show hidden". Always false
    /// for recordings and for calendar items that became recordings (you recorded it).
    pub dismissed: bool,
    /// The key to WRITE when hiding/unhiding this item (specs/0029 WS6.2). Prefers the
    /// sync-stable `calendarItemExternalIdentifier` + occurrence start (EventKit's
    /// `eventIdentifier` can be reissued by provider syncs, orphaning a stored
    /// dismissal); falls back to `id`. Reads match EITHER this key or the legacy
    /// `eventIdentifier`/synthetic key, so pre-existing dismissals keep working.
    pub dismiss_key: String,
    /// Recurring-series key (specs/0036): the event's `external_id` (iCalUID /
    /// calendarItemExternalIdentifier), series-level for both calendar sources. The Today
    /// view threads it into Join & Record and `api_ensure_scheduled_meeting` so recordings
    /// group into their series. None for ad-hoc recordings (no calendar event).
    pub series_key: Option<String>,
    /// The manual row's `calendar_event_id` (`nixon-manual:{uuid}`) — specs/0069 W3.
    /// Record-start adoption (`joinAndRecord` → `api_create_meeting`) matches on this
    /// field to adopt the prep row instead of minting a second meeting. `None` for
    /// calendar and recording items, which have no manual row to adopt.
    pub calendar_event_id: Option<String>,
}

/// Local-day bounds as UTC instants `[start, end)` (midnight→midnight in the
/// system local timezone) for a given day. `date` is a strict local `YYYY-MM-DD`;
/// `None` means today (preserving the original `today_local_bounds` behaviour).
/// Used to constrain the recordings query and to pin item start-times to the day.
fn local_bounds_for(date: Option<&str>) -> anyhow::Result<(DateTime<Utc>, DateTime<Utc>)> {
    let day = match date {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|e| anyhow::anyhow!("invalid agenda date '{s}' (expected YYYY-MM-DD): {e}"))?,
        None => Local::now().date_naive(),
    };
    let start_naive = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("could not build local midnight"))?;
    let start_local = Local
        .from_local_datetime(&start_naive)
        .single()
        .ok_or_else(|| anyhow::anyhow!("ambiguous/absent local midnight (DST boundary)"))?;
    let end_local = start_local + Duration::days(1);
    Ok((
        start_local.with_timezone(&Utc),
        end_local.with_timezone(&Utc),
    ))
}

/// Synthetic, stable id for a calendar-only item whose EventKit identifier is
/// empty (defensive — EventKit normally supplies one).
fn synthetic_event_id(title: &str, start: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    start.hash(&mut hasher);
    format!("evt-{:016x}", hasher.finish())
}

/// Sync-stable dismissal key for a calendar event (specs/0029 WS6.2): the
/// `calendarItemExternalIdentifier` (stable across provider re-syncs, unlike
/// `eventIdentifier`) paired with the occurrence start (the external id is shared
/// by every occurrence of a recurring event, so alone it would hide them all).
/// None when EventKit supplied no external identifier — callers fall back to the
/// legacy `eventIdentifier`/synthetic key.
///
/// `pub(crate)` since specs/0032: Google-sourced items carry the same iCalUID in
/// `external_id` and the same UTC `to_rfc3339` start format, so this key is
/// identical across sources — a dismissal survives switching the active source
/// (EventKit ↔ Google). `google::sync` unit-tests that equivalence.
pub(crate) fn stable_dismiss_key(event: &UpcomingMeeting) -> Option<String> {
    event
        .external_id
        .as_deref()
        .map(|ext| format!("ext:{ext}@{}", event.starts_at))
}

/// Both keys a dismissal for this event may be stored under.
///
/// `.0` is the sync-stable key preferred for NEW dismissals ([`stable_dismiss_key`]); `.1`
/// is the legacy key — the event's own identifier, as the agenda exposes it for an
/// unrecorded calendar row, or a synthetic hash when the provider gave no id. A read must
/// accept EITHER, or a dismissal made before the stable key existed silently stops working
/// (specs/0029 WS6.2). When there is no external identifier both are the legacy key.
pub(crate) fn dismissal_keys(event: &UpcomingMeeting) -> (String, String) {
    let legacy = if event.id.trim().is_empty() {
        synthetic_event_id(&event.title, &event.starts_at)
    } else {
        event.id.clone()
    };
    let stable = stable_dismiss_key(event).unwrap_or_else(|| legacy.clone());
    (stable, legacy)
}

/// Has the user hidden this calendar event from their agenda (specs/0026)?
///
/// Extracted from `build_agenda` on 2026-09-21 because it had exactly one copy and needed
/// three: `api_get_upcoming_meetings` (which drives the T-5 prep and T-0 join notifications)
/// and `prep_jobs::upcoming_for_horizon` (which spends an LLM call per event generating a
/// prep brief) both read the raw calendar and never consulted the dismissal set at all. So
/// hiding "Lunch" removed it from Today and still bought you two banners and a summary.
///
/// Note this asks ONLY about the dismissal keys. The agenda additionally requires that the
/// event matched no recording — recording something means you wanted it — but that is a fact
/// only the agenda builder has, so it stays a condition at its call site. Both new callers
/// look at strictly upcoming events, which by definition have no recording yet.
pub(crate) fn is_event_dismissed(
    event: &UpcomingMeeting,
    dismissed: &std::collections::HashSet<String>,
) -> bool {
    let (stable, legacy) = dismissal_keys(event);
    dismissed.contains(&stable) || dismissed.contains(&legacy)
}

/// Build today's unified agenda. Pure given its inputs, so it is unit-testable
/// without EventKit/DB: takes today's calendar events and today's recorded
/// meetings (with status), plus an attendee fetcher invoked only for calendar
/// items. Returns items sorted by start ascending.
fn build_agenda(
    events: Vec<UpcomingMeeting>,
    recordings: Vec<MeetingStatusRow>,
    manual: Vec<DayAgendaItem>,
    dismissed: &std::collections::HashSet<String>,
    mut fetch_attendees: impl FnMut(&str, &str) -> Vec<Attendee>,
) -> Vec<DayAgendaItem> {
    // Track which recordings have been claimed by a calendar event so the
    // leftover ones become standalone "recording" items.
    let mut claimed = vec![false; recordings.len()];

    let mut items: Vec<DayAgendaItem> = Vec::with_capacity(events.len() + recordings.len());

    for event in events {
        // Match a recorded meeting by title (case-insensitive, trimmed) within the
        // ±MATCH_WINDOW around the event start; pick the closest unclaimed one.
        let event_start = event.starts_at.parse::<DateTime<Utc>>().ok();
        let want_title = event.title.trim().to_lowercase();

        let mut best: Option<(i64, usize)> = None; // (abs delta secs, index)
        if let Some(ev_start) = event_start {
            for (i, rec) in recordings.iter().enumerate() {
                if claimed[i] {
                    continue;
                }
                if rec.title.trim().to_lowercase() != want_title {
                    continue;
                }
                let delta = (rec.created_at.0 - ev_start).num_seconds().abs();
                if delta > MATCH_WINDOW.num_seconds() {
                    continue;
                }
                if best.map(|(d, _)| delta < d).unwrap_or(true) {
                    best = Some((delta, i));
                }
            }
        }

        let (meeting_id, status) = if let Some((_, i)) = best {
            claimed[i] = true;
            (
                Some(recordings[i].id.clone()),
                AgendaStatus::from_row(&recordings[i]),
            )
        } else {
            (None, AgendaStatus::empty())
        };

        let all_attendees = fetch_attendees(&event.title, &event.starts_at);
        let attendee_count = all_attendees.len() as u32;
        let attendees: Vec<Attendee> = all_attendees
            .into_iter()
            .take(MAX_INLINE_ATTENDEES)
            .collect();

        // `.1` is the legacy key — the event's own identifier (the id the agenda exposes
        // for an unrecorded calendar row), independent of whether it later matched a
        // recording. `.0` is the sync-stable key written for NEW dismissals.
        let (dismiss_key, event_key) = dismissal_keys(&event);

        // Only an UNrecorded calendar item can be "dismissed" — if you recorded it, you
        // want it. That half of the rule lives here rather than in `is_event_dismissed`,
        // because only the agenda knows whether an event claimed a recording.
        let is_dismissed = meeting_id.is_none() && is_event_dismissed(&event, dismissed);

        let id = meeting_id.clone().unwrap_or_else(|| event_key.clone());

        items.push(DayAgendaItem {
            id,
            title: event.title,
            start_time: event.starts_at,
            end_time: Some(event.ends_at),
            source: AgendaSource::Calendar,
            zoom_url: event.zoom_url,
            attendees,
            attendee_count,
            meeting_id,
            status,
            dismissed: is_dismissed,
            dismiss_key,
            series_key: event.external_id,
            calendar_event_id: None,
        });
    }

    // Unclaimed recordings → standalone ad-hoc "recording" items.
    for (i, rec) in recordings.into_iter().enumerate() {
        if claimed[i] {
            continue;
        }
        let start = rec.created_at.0;
        let end_time = rec
            .duration_seconds
            .filter(|d| *d > 0.0)
            .map(|d| (start + Duration::milliseconds((d * 1000.0) as i64)).to_rfc3339());

        items.push(DayAgendaItem {
            id: rec.id.clone(),
            title: rec.title.clone(),
            start_time: start.to_rfc3339(),
            end_time,
            source: AgendaSource::Recording,
            zoom_url: None,
            attendees: Vec::new(),
            attendee_count: 0,
            meeting_id: Some(rec.id.clone()),
            status: AgendaStatus::from_row(&rec),
            dismissed: false,
            // Recordings can't be dismissed; keep the field truthful anyway.
            dismiss_key: rec.id.clone(),
            series_key: None,
            calendar_event_id: None,
        });
    }

    items.extend(manual);

    items.sort_by(|a, b| a.start_time.cmp(&b.start_time));
    items
}

/// Today's unified Day Agenda (specs/0012): calendar events + recorded meetings,
/// merged and matched, with attendees + processing status, sorted by start time.
///
/// Best-effort: calendar access denied or EventKit errors still return today's
/// recorded meetings (the calendar layer logs and returns empty).
#[tauri::command]
pub async fn api_get_day_agenda<R: Runtime>(
    app: AppHandle<R>,
    date: Option<String>,
) -> Result<DayAgenda, String> {
    // `None`/empty means today, so existing call sites that pass no date behave unchanged.
    let date = date.filter(|s| !s.trim().is_empty());
    log::info!("api_get_day_agenda called (date={date:?})");

    let (start_utc, end_utc) = local_bounds_for(date.as_deref())
        .map_err(|e| format!("Could not compute the requested date range: {e}"))?;

    // Recorded meetings created within the day window (one query, status flags derived inline).
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    let recordings = MeetingsRepository::get_between_with_status(pool, start_utc, end_utc)
        .await
        .map_err(|e| format!("Failed to load the day's meetings: {e}"))?;

    // Events the user has hidden (specs/0026). Best-effort: a lookup failure must not break the
    // agenda, so fall back to "nothing dismissed".
    let dismissed = DismissedCalendarEventsRepository::all(pool)
        .await
        .unwrap_or_else(|e| {
            log::error!("Failed to load dismissed calendar events (showing all): {e}");
            std::collections::HashSet::new()
        });

    // Today's calendar events from the single active source (specs/0032):
    // while a Google account is connected, ONLY its local cache is read
    // (refreshed when stale; sync failures are logged and the last-good cache
    // serves) — no EventKit call, no macOS Calendar permission dependency.
    // When not connected, EventKit only (best-effort; empty when access
    // denied). EventKit reads touch the Objective-C runtime, so they run off
    // the async executor.
    let google_active = crate::calendar::google_is_active_source(&app).await;
    let events = if google_active {
        use crate::calendar::google::sync::{sync_if_stale, SyncTrigger};
        sync_if_stale(&app, SyncTrigger::Agenda).await;
        crate::calendar::google::sync::cached_upcoming_between(pool, start_utc, end_utc).await
    } else {
        tokio::task::spawn_blocking(move || eventkit::meetings_between(start_utc, end_utc))
            .await
            .unwrap_or_else(|e| {
                log::error!("Calendar read task failed: {e}; agenda will use recordings only");
                Vec::new()
            })
    };

    // Attendees are only fetched for calendar items, and each fetch may be an
    // EventKit read, so do them off-thread too. We pre-fetch per unique
    // (title, start) to keep `build_agenda` synchronous and testable. Items
    // are routed by their id: Google-sourced items (`gcal:` ids) read their
    // attendees from the local cache row — no EventKit round-trip, no network
    // (specs/0032) — while EventKit items use the EventKit reader.
    let mut attendee_cache: std::collections::HashMap<(String, String), Vec<Attendee>> =
        std::collections::HashMap::new();
    // Load the cached attendee-photo map ONCE for the whole build (finding #9):
    // threading it into the per-event Google reader avoids a full photo-table
    // scan per event. Cheap and empty when photos were never fetched.
    let photo_map = crate::calendar::google::sync::cached_photo_map(pool).await;
    for event in &events {
        let key = (event.title.clone(), event.starts_at.clone());
        if attendee_cache.contains_key(&key) {
            continue;
        }
        let attendees = if event.id.starts_with("gcal:") {
            crate::calendar::google::sync::cached_attendees_for_event_id_with_photos(
                pool, &event.id, &photo_map,
            )
            .await
        } else {
            let (title, started_at) = key.clone();
            tokio::task::spawn_blocking(move || eventkit::event_attendees(&title, &started_at))
                .await
                .unwrap_or_else(|e| {
                    log::error!("Attendee read task failed: {e}");
                    Vec::new()
                })
        };
        attendee_cache.insert(key, attendees);
    }

    // specs/0069 W3 — meetings the user added inside Nixon. A SEPARATE query on purpose:
    // `get_between_with_status` excludes every `scheduled` row so a prep placeholder can
    // never be merged into its own calendar event and mark it recorded. These rows join the
    // agenda without going near that matching loop.
    let manual = MeetingsRepository::get_manual_scheduled_between(pool, start_utc, end_utc)
        .await
        .map(crate::calendar::manual_items::manual_agenda_items)
        .unwrap_or_else(|e| {
            log::error!("Failed to load manually added meetings (continuing): {e}");
            Vec::new()
        });

    let items = build_agenda(events, recordings, manual, &dismissed, |title, start| {
        attendee_cache
            .get(&(title.to_string(), start.to_string()))
            .cloned()
            .unwrap_or_default()
    });

    // A meeting recorded without an invite has no calendar attendees, but it may well have a
    // roster — the people named on it, the same rows All Meetings shows faces from.
    let mut items = items;
    match MeetingParticipantsRepository::attendee_previews(pool).await {
        Ok(rows) => fill_roster_attendees(&mut items, rows),
        Err(e) => log::warn!("Failed to load meeting rosters for the agenda (continuing): {e}"),
    }

    log::info!("api_get_day_agenda -> {} item(s)", items.len());
    Ok(DayAgenda {
        items,
        calendar_source: if google_active {
            CalendarSource::Google
        } else {
            CalendarSource::EventKit
        },
    })
}

/// Give every agenda item that belongs to a meeting but carries no calendar attendees its
/// meeting roster instead. Calendar attendees win where both exist: the invite is the
/// meeting's own guest list, and it is what the rest of the row was matched on.
///
/// Owner report 2026-09-23: Today drew no faces for a recording with three named
/// participants, while All Meetings drew them — Today only ever read calendar invites.
fn fill_roster_attendees(items: &mut [DayAgendaItem], rows: Vec<AttendeePreviewRow>) {
    let mut rosters: std::collections::HashMap<String, (Vec<Attendee>, u32)> =
        std::collections::HashMap::new();
    for row in rows {
        let entry = rosters
            .entry(row.meeting_id)
            .or_insert_with(|| (Vec::new(), row.total.max(0) as u32));
        entry.0.push(Attendee {
            name: row.display_name,
            email: row.email,
            is_current_user: row.is_current_user != 0,
            is_distribution_list: false,
            photo_data_uri: None,
        });
    }
    for item in items.iter_mut() {
        if !item.attendees.is_empty() {
            continue;
        }
        let Some(meeting_id) = item.meeting_id.as_deref() else {
            continue;
        };
        if let Some((attendees, total)) = rosters.get(meeting_id) {
            item.attendees = attendees
                .iter()
                .take(MAX_INLINE_ATTENDEES)
                .cloned()
                .collect();
            item.attendee_count = *total;
        }
    }
}

/// Hide a calendar event from the agenda (specs/0026). `event_id` is the agenda item's id for
/// an unrecorded calendar row (the EventKit event id, or the synthetic `evt-<hash>`).
#[tauri::command]
pub async fn api_dismiss_calendar_event<R: Runtime>(
    app: AppHandle<R>,
    event_id: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    DismissedCalendarEventsRepository::dismiss(state.db_manager.pool(), &event_id)
        .await
        .map_err(|e| format!("Failed to dismiss calendar event: {e}"))
}

/// Un-hide a previously dismissed calendar event (specs/0026).
#[tauri::command]
pub async fn api_undismiss_calendar_event<R: Runtime>(
    app: AppHandle<R>,
    event_id: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    DismissedCalendarEventsRepository::undismiss(state.db_manager.pool(), &event_id)
        .await
        .map_err(|e| format!("Failed to undismiss calendar event: {e}"))
}

/// List the ids of all dismissed calendar events (specs/0026).
#[tauri::command]
pub async fn api_list_dismissed_calendar_events<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<String>, String> {
    let state = app.state::<AppState>();
    DismissedCalendarEventsRepository::all(state.db_manager.pool())
        .await
        .map(|set| set.into_iter().collect())
        .map_err(|e| format!("Failed to list dismissed calendar events: {e}"))
}

#[cfg(test)]
mod tests;
