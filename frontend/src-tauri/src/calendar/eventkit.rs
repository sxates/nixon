// macOS EventKit reader (specs/0008 P2).
//
// Reads the user's local macOS Calendar — which already aggregates iCloud /
// Google / Exchange / CalDAV accounts — via Apple's EventKit framework, with
// zero network egress from Nixon (privacy-first per CLAUDE.md). We only ever
// *read*: authorization status, an async access request, and upcoming events.
//
// All EventKit calls are `unsafe` Objective-C FFI through `objc2-event-kit`. We
// keep that surface entirely inside this file and hand the rest of the app plain
// Rust types (`UpcomingMeeting`, an access-status string).
//
// IMPORTANT: the OS *crashes* the process on an access request if the bundle's
// Info.plist lacks `NSCalendarsFullAccessUsageDescription` (and the legacy
// `NSCalendarsUsageDescription`). Those strings are injected via
// `src-tauri/Info.plist`, which Tauri merges into the bundled .app's Info.plist.
// A bare `cargo run` dev binary has no Info.plist, so the permission prompt only
// works in a built/bundled .app — see the module-level note in `mod.rs`.

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::calendar::zoom_link::extract_zoom_url;

/// One upcoming calendar event, in the wire shape the frontend consumes.
/// Field names are camelCase on the wire (serde rename) to match existing
/// Tauri command conventions. Since specs/0032 this is also the cross-source
/// shape: when Google is the active source, its cached events are converted
/// into it (`calendar::google::sync`) with a `gcal:`-prefixed `id` and the
/// iCalUID as `external_id`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingMeeting {
    /// Stable EventKit identifier for this event occurrence.
    pub id: String,
    pub title: String,
    /// ISO-8601 (RFC 3339) start instant, UTC.
    pub starts_at: String,
    /// ISO-8601 (RFC 3339) end instant, UTC.
    pub ends_at: String,
    /// Name of the calendar the event belongs to (e.g. "Work", "Personal").
    pub calendar_name: String,
    /// Raw location string, if any.
    pub location: Option<String>,
    /// Extracted Zoom join URL (from url/location/notes), if any.
    pub zoom_url: Option<String>,
    /// EventKit's `calendarItemExternalIdentifier` — stable across syncs/devices
    /// (unlike `eventIdentifier`, which provider-synced calendars may reissue
    /// between reads). Shared by all occurrences of a recurring event, so
    /// consumers must pair it with the occurrence start to key one occurrence
    /// (see `day_agenda`'s dismissal key, specs/0029 WS6.2). None when EventKit
    /// has no external identifier (e.g. an unsynced local item).
    pub external_id: Option<String>,
}

/// One attendee of a calendar event (specs/0010 P2 Task 7), in the camelCase wire
/// shape the frontend consumes. Used to turn the diarized "Speaker N" labels into a
/// pick-list of real people (display name + stable `email` identity key).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    /// Display name (falls back to the email local-part when EventKit has no name).
    pub name: String,
    /// Email parsed from the participant's `mailto:` URL, when present.
    pub email: Option<String>,
    /// True for the local user (the device owner) — the diarization `local`/`You`
    /// speaker, so the UI can keep it separate from the remote pick-list.
    pub is_current_user: bool,
    /// True when this entry is a distribution list / group address rather than a
    /// person (specs/0038 WS3). A DL is excluded from speaker/voiceprint binding
    /// (`seed_from_attendees`) and rendered as a labeled floor; individual
    /// members are folded in separately when Cloud Identity access is available.
    /// EventKit has no group concept, so this is always `false` there.
    #[serde(default)]
    pub is_distribution_list: bool,
    /// A self-contained base64 `data:` image URI for this attendee's profile
    /// photo, when one was cached from the Google domain directory (specs/0038
    /// WS3, ADR-0010 best-effort amendment). `None` degrades to initials in the
    /// UI — the common case for external attendees, orgs that forbid the People
    /// directory read, and every EventKit-sourced attendee (no photos there).
    /// It's a `data:` URI (never a Google URL) so rendering is LOCAL-ONLY and a
    /// Disconnect purge removes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo_data_uri: Option<String>,
}

/// Calendar authorization status, normalized to the strings the frontend expects.
pub fn access_status() -> String {
    #[cfg(target_os = "macos")]
    {
        use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};

        // `authorizationStatusForEntityType:` is a class method and does not need
        // an EKEventStore instance; it is safe to call without a usage string.
        // `EKAuthorizationStatus` is a newtype over NSInteger (a set of
        // associated constants, not a Rust enum), so we compare by value.
        let status = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };

        // Note: the deprecated `Authorized` constant and `FullAccess` share the
        // same raw value on macOS 14+, so checking FullAccess covers both
        // (and avoids the `deprecated` lint). WriteOnly means we can't read
        // events -> treat as denied for our read-only use.
        if status == EKAuthorizationStatus::NotDetermined {
            "notDetermined"
        } else if status == EKAuthorizationStatus::Restricted {
            "restricted"
        } else if status == EKAuthorizationStatus::FullAccess {
            "authorized"
        } else {
            // Denied or WriteOnly -> denied (no read access).
            "denied"
        }
        .to_string()
    }

    #[cfg(not(target_os = "macos"))]
    {
        "denied".to_string()
    }
}

/// Request full calendar access (async). Returns `true` if access was granted.
///
/// Bridges EventKit's completion-handler callback to async Rust via a tokio
/// oneshot channel, so we never block the calling (or main) thread. On macOS
/// 14+ this uses `requestFullAccessToEventsWithCompletion:`; the completion
/// block fires on an arbitrary OS queue and just forwards the `granted` bool.
pub async fn request_access() -> Result<bool> {
    #[cfg(target_os = "macos")]
    {
        use block2::RcBlock;
        use objc2::rc::Retained;
        use objc2::runtime::Bool;
        use objc2_event_kit::EKEventStore;
        use objc2_foundation::NSError;

        log::info!(
            "request_access: triggering EventKit full-access prompt (authorization status before request={})",
            access_status()
        );

        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();

        // The EventKit objects (`EKEventStore`, the completion `RcBlock`) are
        // `!Send`, so they must NOT be held across an `.await`. We confine all
        // FFI to this synchronous block; only the `Send` oneshot receiver
        // crosses the await below. The store + block must outlive this scope
        // (EventKit invokes the completion later, on an arbitrary OS queue), so
        // we deliberately leak them — this command runs at most once per app
        // session, so the leak is negligible and avoids a use-after-free.
        {
            // Wrap the sender so the (possibly-reused) block can `take()` it once.
            let tx = std::cell::Cell::new(Some(tx));
            let store: Retained<EKEventStore> = unsafe { EKEventStore::new() };

            let completion = RcBlock::new(move |granted: Bool, _err: *mut NSError| {
                // This block runs on an arbitrary OS queue when EventKit finishes
                // the prompt; logging here confirms the async completion fired.
                log::info!(
                    "request_access: EventKit completion handler fired (granted={})",
                    granted.as_bool()
                );
                if let Some(sender) = tx.take() {
                    // Receiver may be gone if the caller was cancelled.
                    let _ = sender.send(granted.as_bool());
                }
            });

            // SAFETY: `requestFullAccessToEventsWithCompletion:` is a valid
            // EKEventStore instance method; the block matches the expected
            // `(BOOL granted, NSError *error)` signature. Requires
            // NSCalendarsFullAccessUsageDescription in Info.plist or the OS aborts.
            unsafe {
                store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&completion));
            }

            // Keep both alive past this scope until the OS fires the callback.
            std::mem::forget(store);
            std::mem::forget(completion);
        }

        match rx.await {
            Ok(granted) => {
                log::info!("Calendar access request completed (granted={granted})");
                Ok(granted)
            }
            Err(_) => Err(anyhow!(
                "Calendar access request did not complete (completion handler dropped)"
            )),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(false)
    }
}

/// Read upcoming, non-all-day events that start within `within_hours` from now,
/// sorted by start. Returns an empty list (never an error) if access isn't
/// authorized, so callers degrade gracefully.
///
/// NOTE: we deliberately do NOT pre-gate on `access_status()` here. Right after
/// the user grants access, the class-method authorization status lags for a short
/// window within the same process (a known EventKit propagation delay), so gating
/// would spuriously return empty on the first read post-grant. Instead we let the
/// predicate query be the source of truth: a genuinely-unauthorized store simply
/// yields no events (it never crashes and never prompts), so this stays correct
/// when denied while removing the post-grant race.
pub fn upcoming_meetings(within_hours: u32) -> Vec<UpcomingMeeting> {
    #[cfg(target_os = "macos")]
    {
        match read_events(within_hours) {
            Ok(mut events) => {
                events.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));
                events
            }
            Err(e) => {
                log::error!("Failed to read calendar events: {e}");
                Vec::new()
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = within_hours;
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn read_events(within_hours: u32) -> Result<Vec<UpcomingMeeting>> {
    use objc2::rc::Retained;
    use objc2_event_kit::EKEventStore;
    use objc2_foundation::NSDate;

    // A FRESH store sees the current authorization (important right after a grant,
    // when the class-method status still lags). No pre-gate on the class status:
    // the predicate query itself is authoritative — an unauthorized store simply
    // returns no events, never crashing or prompting (see `upcoming_meetings`).
    let store: Retained<EKEventStore> = unsafe { EKEventStore::new() };

    let window_secs = (within_hours as f64) * 3600.0;
    // `NSDate::now()` is the reference instant; build [now, now + window].
    let start: Retained<NSDate> = NSDate::now();
    let end: Retained<NSDate> = NSDate::dateWithTimeIntervalSinceNow(window_secs);

    let now_interval = start.timeIntervalSince1970();

    // Only events from now forward (the predicate is inclusive of in-progress
    // events that started slightly before `now`; upcoming wants future starts only).
    map_events_in_window(&store, &start, &end, Some(now_interval))
}

/// Read ALL non-all-day events whose start falls within today's LOCAL calendar
/// day (local 00:00 → next local 00:00), INCLUDING events whose start time has
/// already passed (meetings that started late must not drop off the agenda —
/// this is the whole point of specs/0012's Day Agenda). Sorted by start.
///
/// Returns an empty list (never an error) when calendar access isn't authorized,
/// so the agenda degrades gracefully to recordings-only.
///
/// NOTE: no pre-gate on `access_status()` — see `upcoming_meetings` for why. The
/// predicate query is the source of truth, which removes the post-grant status
/// lag race (the first agenda refresh after Connect would otherwise be empty).
pub fn today_meetings() -> Vec<UpcomingMeeting> {
    #[cfg(target_os = "macos")]
    {
        use chrono::{Duration, Local, TimeZone};
        // The local-day window [midnight today, midnight tomorrow), as absolute
        // instants — then read via the shared bounded reader.
        let now_local = Local::now();
        let start_local = now_local
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| Local.from_local_datetime(&naive).single());
        match start_local {
            Some(start) => meetings_between(
                start.with_timezone(&chrono::Utc),
                (start + Duration::days(1)).with_timezone(&chrono::Utc),
            ),
            None => {
                log::error!("Could not build local midnight (DST boundary); no today events");
                Vec::new()
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Calendar events whose start falls within `[start_utc, end_utc)`, read from
/// EventKit (all calendars). This is the date-parameterized reader behind the
/// multi-day agenda (`specs/0038` WS4) — `today_meetings` is the today-window
/// special case. Best-effort: empty when access is denied or on error.
pub fn meetings_between(
    start_utc: chrono::DateTime<chrono::Utc>,
    end_utc: chrono::DateTime<chrono::Utc>,
) -> Vec<UpcomingMeeting> {
    #[cfg(target_os = "macos")]
    {
        match read_events_between(start_utc, end_utc) {
            Ok(mut events) => {
                events.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));
                events
            }
            Err(e) => {
                log::error!("Failed to read calendar events for the requested day: {e}");
                Vec::new()
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (start_utc, end_utc);
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn read_events_between(
    start_utc: chrono::DateTime<chrono::Utc>,
    end_utc: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<UpcomingMeeting>> {
    use objc2::rc::Retained;
    use objc2_event_kit::EKEventStore;
    use objc2_foundation::NSDate;

    // Fresh store + no class-status pre-gate (the predicate query is authoritative);
    // this is what lets a freshly-granted session read events immediately despite
    // the class-method authorization-status propagation lag.
    let store: Retained<EKEventStore> = unsafe { EKEventStore::new() };

    // Absolute-instant window [start, end) supplied by the caller — the local-day
    // bounds for the requested date (today, or a navigated day, WS4). The EventKit
    // predicate matches over absolute instants, so passing UTC bounds is correct.
    let start_unix = start_utc.timestamp() as f64;
    let end_unix = end_utc.timestamp() as f64;

    let start: Retained<NSDate> = NSDate::dateWithTimeIntervalSince1970(start_unix);
    let end: Retained<NSDate> = NSDate::dateWithTimeIntervalSince1970(end_unix);

    // No "min start" filter: we deliberately keep events whose start already
    // passed today, but constrain to events that actually START within the day
    // (the predicate also returns multi-day events overlapping the window).
    map_events_in_window(&store, &start, &end, None).map(|events| {
        events
            .into_iter()
            .filter(|e| {
                // Keep only events whose START instant is within [start, end).
                e.starts_at
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .map(|dt| {
                        let ts = dt.timestamp() as f64;
                        ts >= start_unix && ts < end_unix
                    })
                    .unwrap_or(false)
            })
            .collect()
    })
}

/// Shared event→`UpcomingMeeting` mapping over a predicate window. When
/// `min_start_interval` is `Some`, events starting before that instant are
/// skipped (used by `upcoming_meetings` to exclude already-started events);
/// `None` keeps all non-all-day events in the window (used by `today_meetings`).
#[cfg(target_os = "macos")]
fn map_events_in_window(
    store: &objc2_event_kit::EKEventStore,
    start: &objc2_foundation::NSDate,
    end: &objc2_foundation::NSDate,
    min_start_interval: Option<f64>,
) -> Result<Vec<UpcomingMeeting>> {
    // `predicateForEventsWithStartDate:endDate:calendars:` with nil calendars =>
    // search all calendars (all accounts in macOS Calendar).
    let predicate =
        unsafe { store.predicateForEventsWithStartDate_endDate_calendars(start, end, None) };
    let events = unsafe { store.eventsMatchingPredicate(&predicate) };

    let mut out = Vec::with_capacity(events.len());
    for event in events.iter() {
        // Skip all-day events — they aren't meetings.
        if unsafe { event.isAllDay() } {
            continue;
        }

        let start_date = unsafe { event.startDate() };
        let end_date = unsafe { event.endDate() };

        let start_interval = start_date.timeIntervalSince1970();
        if let Some(min) = min_start_interval {
            if start_interval < min {
                continue;
            }
        }

        let starts_at = match unix_interval_to_rfc3339(start_interval) {
            Some(s) => s,
            None => continue,
        };
        let ends_at = match unix_interval_to_rfc3339(end_date.timeIntervalSince1970()) {
            Some(s) => s,
            None => continue,
        };

        let title = unsafe { event.title() }.to_string();

        let id = unsafe { event.eventIdentifier() }
            .map(|s| s.to_string())
            // Fallback id so the frontend always has a stable-ish key.
            .unwrap_or_else(|| format!("{title}@{starts_at}"));

        // Stable-across-syncs identifier for dismissal keying (specs/0029 WS6.2).
        // `calendarItemExternalIdentifier` survives provider re-syncs that can
        // reissue `eventIdentifier`; it is shared across occurrences of a
        // recurring event, so key consumers pair it with the occurrence start.
        let external_id = unsafe { event.calendarItemExternalIdentifier() }
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty());

        // Diagnostic for key-stability across reads (specs/0029 WS6.2): compare
        // these lines between two agenda refreshes to confirm whether a
        // provider reissued `eventIdentifier` for the same event.
        log::debug!(
            "calendar event key: id={id} external={external_id:?} title={title:?} start={starts_at}"
        );

        let calendar_name = unsafe { event.calendar() }
            .map(|c| unsafe { c.title() }.to_string())
            .unwrap_or_else(|| "Calendar".to_string());

        let location = unsafe { event.location() }.map(|s| s.to_string());
        let notes = unsafe { event.notes() }.map(|s| s.to_string());
        let url = unsafe { event.URL() }
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string());

        let zoom_url = extract_zoom_url([
            url.as_deref().unwrap_or(""),
            location.as_deref().unwrap_or(""),
            notes.as_deref().unwrap_or(""),
        ]);

        out.push(UpcomingMeeting {
            id,
            title,
            starts_at,
            ends_at,
            calendar_name,
            location,
            zoom_url,
            external_id,
        });
    }

    Ok(dedup_by_external_id(out))
}

/// Collapse duplicate reads of the SAME event visible on several calendars
/// (specs/0041 WS5): the same invite synced to two calendars yields two
/// `EKEvent`s sharing a `calendarItemExternalIdentifier` (the iCalUID). Events
/// sharing `external_id` AND `starts_at` collapse to the first one seen; events
/// without an external identifier are never collapsed (an unsynced local item
/// has no cross-calendar identity). Recurring occurrences share the external id
/// but differ in start, so they all survive. The dismissal key is
/// `ext:{external_id}@{starts_at}` — identical for every duplicate — so stored
/// dismissals apply to whichever copy survives.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn dedup_by_external_id(events: Vec<UpcomingMeeting>) -> Vec<UpcomingMeeting> {
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    events
        .into_iter()
        .filter(|e| match &e.external_id {
            Some(ext) if !ext.trim().is_empty() => seen.insert((ext.clone(), e.starts_at.clone())),
            _ => true,
        })
        .collect()
}

/// Attendees of the calendar event linked to a recorded meeting (specs/0010 P2).
///
/// Meetings aren't persisted with a calendar event id, so we re-locate the event by
/// searching a window around the meeting's start instant and matching on title
/// (case-insensitive, trimmed). `started_at` is the recording-start instant
/// (RFC-3339, the meeting's `created_at`). Returns an empty list (never an error)
/// when access isn't granted or no event matches, so callers degrade gracefully.
///
/// NOTE: no `access_status()` pre-gate — the predicate query is authoritative
/// (an unauthorized store returns no events), which keeps attendees populating
/// correctly even on the first read right after a grant (see `upcoming_meetings`).
pub fn event_attendees(title: &str, started_at: &str) -> Vec<Attendee> {
    #[cfg(target_os = "macos")]
    {
        match read_event_attendees(title, started_at) {
            Ok(attendees) => attendees,
            Err(e) => {
                log::error!("Failed to read calendar attendees: {e}");
                Vec::new()
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, started_at);
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn read_event_attendees(title: &str, started_at: &str) -> Result<Vec<Attendee>> {
    use chrono::{DateTime, Utc};
    use objc2::rc::Retained;
    use objc2_event_kit::EKEventStore;
    use objc2_foundation::NSDate;

    let started: DateTime<Utc> = started_at
        .parse()
        .map_err(|e| anyhow!("invalid meeting start instant '{started_at}': {e}"))?;

    // Fresh store + no class-status pre-gate; the predicate query is authoritative
    // (an unauthorized store yields no events), avoiding the post-grant status lag.
    let store: Retained<EKEventStore> = unsafe { EKEventStore::new() };

    // Search a generous window centred on the recording start so we still find the
    // event when recording began a little before/after the scheduled time.
    let started_unix =
        started.timestamp() as f64 + (started.timestamp_subsec_nanos() as f64) / 1_000_000_000.0;
    const WINDOW_SECS: f64 = 4.0 * 3600.0;
    let start: Retained<NSDate> = NSDate::dateWithTimeIntervalSince1970(started_unix - WINDOW_SECS);
    let end: Retained<NSDate> = NSDate::dateWithTimeIntervalSince1970(started_unix + WINDOW_SECS);

    let predicate =
        unsafe { store.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None) };
    let events = unsafe { store.eventsMatchingPredicate(&predicate) };

    let want = title.trim().to_lowercase();

    // Pick the title-matching event whose start is closest to the recording start.
    let mut best: Option<(f64, Retained<objc2_event_kit::EKEvent>)> = None;
    for event in events.iter() {
        let ev_title = unsafe { event.title() }.to_string();
        if ev_title.trim().to_lowercase() != want {
            continue;
        }
        let ev_start = unsafe { event.startDate() }.timeIntervalSince1970();
        let delta = (ev_start - started_unix).abs();
        if best.as_ref().map(|(d, _)| delta < *d).unwrap_or(true) {
            best = Some((delta, event));
        }
    }

    let Some((_, event)) = best else {
        log::info!("event_attendees: no calendar event matched title '{title}'");
        return Ok(Vec::new());
    };

    Ok(attendees_from_event(&event))
}

/// Attendees of the calendar event with the given EventKit identifier (specs/0015).
///
/// Exact lookup by the stable EventKit event id (the same id exposed as
/// `UpcomingMeeting.id`), replacing the brittle title+start-instant re-location in
/// `event_attendees` for meetings that persisted a `calendar_event_id`. Returns an
/// empty list (never an error) when access isn't granted or the event no longer
/// exists, so callers degrade gracefully (and can fall back to the title+time path).
///
/// NOTE: no `access_status()` pre-gate — a fresh, unauthorized store simply returns
/// no event, matching the rest of this module (see `upcoming_meetings`).
pub fn event_attendees_by_id(event_id: &str) -> Vec<Attendee> {
    #[cfg(target_os = "macos")]
    {
        match read_event_attendees_by_id(event_id) {
            Ok(attendees) => attendees,
            Err(e) => {
                log::error!("Failed to read calendar attendees by id: {e}");
                Vec::new()
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = event_id;
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn read_event_attendees_by_id(event_id: &str) -> Result<Vec<Attendee>> {
    use objc2::rc::Retained;
    use objc2_event_kit::EKEventStore;
    use objc2_foundation::NSString;

    if event_id.trim().is_empty() {
        return Err(anyhow!("empty calendar event id"));
    }

    // Fresh store + no class-status pre-gate; an unauthorized store returns no event
    // (it never crashes or prompts), avoiding the post-grant status lag.
    let store: Retained<EKEventStore> = unsafe { EKEventStore::new() };

    let identifier = NSString::from_str(event_id);
    // `eventWithIdentifier:` returns the first occurrence matching the id, or nil.
    let Some(event) = (unsafe { store.eventWithIdentifier(&identifier) }) else {
        log::info!("event_attendees_by_id: no calendar event found for id '{event_id}'");
        return Ok(Vec::new());
    };

    Ok(attendees_from_event(&event))
}

/// Map an `EKEvent`'s participants into the wire `Attendee` shape (name/email/
/// isCurrentUser). Shared by the title+time and event-id lookup paths.
#[cfg(target_os = "macos")]
fn attendees_from_event(event: &objc2_event_kit::EKEvent) -> Vec<Attendee> {
    let mut out = Vec::new();
    if let Some(attendees) = unsafe { event.attendees() } {
        for participant in attendees.iter() {
            let email = unsafe { participant.URL() }
                .absoluteString()
                .and_then(|s| mailto_email(&s.to_string()));
            let name = unsafe { participant.name() }
                .map(|s| s.to_string())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| email.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let is_current_user = unsafe { participant.isCurrentUser() };
            out.push(Attendee {
                name,
                email,
                is_current_user,
                is_distribution_list: false, // EventKit has no group concept
                photo_data_uri: None,        // EventKit has no directory photos
            });
        }
    }
    out
}

/// Extract the bare email from a participant URL, which EventKit returns as a
/// `mailto:` URI (e.g. `mailto:priya@example.com`). Returns None for non-mailto URLs.
#[cfg(target_os = "macos")]
fn mailto_email(url: &str) -> Option<String> {
    url.strip_prefix("mailto:")
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
}

/// Convert a Unix timestamp (seconds since 1970, fractional) to an RFC-3339 /
/// ISO-8601 UTC string. Returns None if the instant is out of range.
#[cfg(target_os = "macos")]
fn unix_interval_to_rfc3339(secs: f64) -> Option<String> {
    use chrono::{DateTime, Utc};
    let whole = secs.trunc() as i64;
    let nanos = ((secs.fract()) * 1_000_000_000.0).round() as u32;
    DateTime::<Utc>::from_timestamp(whole, nanos).map(|dt| dt.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evt(id: &str, external_id: Option<&str>, calendar: &str, start: &str) -> UpcomingMeeting {
        UpcomingMeeting {
            id: id.to_string(),
            title: "Standup".to_string(),
            starts_at: start.to_string(),
            ends_at: "2026-07-08T10:00:00+00:00".to_string(),
            calendar_name: calendar.to_string(),
            location: None,
            zoom_url: None,
            external_id: external_id.map(str::to_string),
        }
    }

    // -- specs/0041 WS5: same invite on two calendars renders once ------------

    #[test]
    fn dedup_collapses_shared_external_id_and_start_keeping_first() {
        let t = "2026-07-08T09:00:00+00:00";
        let events = vec![
            evt("ek-work", Some("UID-1"), "Work", t),
            evt("ek-team", Some("UID-1"), "Team", t),
            evt("ek-other", Some("UID-2"), "Work", t),
        ];
        let out = dedup_by_external_id(events);
        assert_eq!(
            out.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["ek-work", "ek-other"],
            "shared UID+start collapses to the first copy; distinct UIDs survive"
        );
    }

    #[test]
    fn dedup_keeps_recurring_occurrences_sharing_a_uid() {
        let events = vec![
            evt(
                "ek-1",
                Some("UID-RECUR"),
                "Work",
                "2026-07-08T09:00:00+00:00",
            ),
            evt(
                "ek-2",
                Some("UID-RECUR"),
                "Work",
                "2026-07-08T14:00:00+00:00",
            ),
        ];
        assert_eq!(
            dedup_by_external_id(events).len(),
            2,
            "same series, different occurrence starts — both stay"
        );
    }

    #[test]
    fn dedup_never_touches_events_without_an_external_id() {
        let t = "2026-07-08T09:00:00+00:00";
        let events = vec![
            evt("ek-1", None, "Work", t),
            evt("ek-2", None, "Team", t),
            evt("ek-3", Some("  "), "Team", t), // blank ⇒ treated as no identity
        ];
        assert_eq!(
            dedup_by_external_id(events).len(),
            3,
            "no cross-calendar identity ⇒ nothing to collapse"
        );
    }
}
