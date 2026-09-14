//! `events.list` response parsing and event→cache-row mapping for the Google
//! Calendar sync (specs/0032, extracted from `sync.rs` by specs/0054 W5).
//!
//! Split out so the sync engine's control flow and this wire-format layer can be
//! read — and grown — independently. `sync.rs` is at its specs/0042 file-size
//! ceiling, and W5 needed room to teach this layer the difference between a
//! recurring series MASTER and an expanded occurrence.
//!
//! Everything here is `pub(super)`: it is the Google module's internal wire
//! vocabulary, not a crate-wide API.

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::calendar::zoom_link::extract_zoom_url;
use crate::database::repositories::google_calendar::GoogleCalendarEventRow;

// ---------------------------------------------------------------------------
// events.list — response parsing + event mapping
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct EventsPage {
    pub(super) items: Vec<GcalEvent>,
    pub(super) next_page_token: Option<String>,
    pub(super) next_sync_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct GcalEvent {
    pub(super) id: String,
    pub(super) status: Option<String>,
    pub(super) summary: Option<String>,
    #[serde(rename = "iCalUID")]
    pub(super) ical_uid: Option<String>,
    pub(super) start: Option<GcalEventTime>,
    pub(super) end: Option<GcalEventTime>,
    pub(super) location: Option<String>,
    pub(super) description: Option<String>,
    pub(super) hangout_link: Option<String>,
    pub(super) conference_data: Option<GcalConferenceData>,
    pub(super) organizer: Option<GcalOrganizer>,
    pub(super) attendees: Vec<GcalAttendee>,
    pub(super) updated: Option<String>,
    /// RRULE/EXDATE lines. Present ONLY on a recurring series MASTER — an
    /// expanded occurrence never carries it (specs/0054 W5).
    pub(super) recurrence: Vec<String>,
    /// The master's id, present only on an expanded occurrence.
    pub(super) recurring_event_id: Option<String>,
}

impl GcalEvent {
    pub(super) fn is_cancelled(&self) -> bool {
        self.status.as_deref() == Some("cancelled")
    }

    /// True when this item is a recurring series MASTER rather than one expanded
    /// occurrence (specs/0054 W5).
    ///
    /// `full_sync` requests `singleEvents=true` and therefore only ever sees
    /// occurrences — but the INCREMENTAL page returns whatever changed, and for a
    /// modified recurring series that is the master. Caching it produces a
    /// phantom row at the series' first-occurrence time keyed `gcal:<cal>/<series>`,
    /// which no cancellation can ever delete: cancellations arrive keyed
    /// `<series>_<instanceTs>`. On the reporter's machine 384 of 1,525 cached
    /// rows (25%) were such phantoms.
    pub(super) fn is_series_master(&self) -> bool {
        !self.recurrence.is_empty()
    }

    /// The series this item belongs to — the master's own id, or the parent id
    /// carried by an expanded occurrence. Used for series-scoped deletes.
    pub(super) fn series_id(&self) -> &str {
        self.recurring_event_id.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct GcalEventTime {
    /// Timed events: RFC3339 with offset (e.g. `2026-07-02T09:00:00+02:00`).
    pub(super) date_time: Option<String>,
    /// All-day events: date only (`2026-07-02`).
    pub(super) date: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct GcalConferenceData {
    pub(super) entry_points: Vec<GcalEntryPoint>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct GcalEntryPoint {
    pub(super) uri: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct GcalOrganizer {
    pub(super) email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct GcalAttendee {
    pub(super) email: Option<String>,
    pub(super) display_name: Option<String>,
    pub(super) response_status: Option<String>,
    /// Google's `self` flag — this attendee IS the connected account.
    #[serde(rename = "self")]
    pub(super) is_self: bool,
    /// Rooms/equipment; dropped from the attendee list (spec 0032 mapping).
    pub(super) resource: bool,
    pub(super) organizer: bool,
}

/// The occurrence-unique cache id: `gcal:<calendarId>/<instanceEventId>`.
/// With `singleEvents=true` recurring occurrences already arrive with
/// per-instance ids (`<recurringEventId>_<originalStartTime>`).
pub(super) fn gcal_row_id(calendar_id: &str, event_id: &str) -> String {
    format!("gcal:{calendar_id}/{event_id}")
}

/// Parse a Google event time into `(RFC3339 UTC via chrono to_rfc3339, is_all_day)`.
/// The UTC `to_rfc3339()` format is load-bearing: the `ext:<uid>@<start>`
/// dismissal keys are string-compared against EventKit's output (0029 WS6.2),
/// so Google's zoned `dateTime` MUST be converted to `Utc` first. All-day
/// events (date-only) are stored at UTC midnight and flagged.
pub(super) fn parse_event_time(time: &GcalEventTime) -> Option<(String, bool)> {
    if let Some(dt) = time.date_time.as_deref() {
        return DateTime::parse_from_rfc3339(dt)
            .ok()
            .map(|d| (d.with_timezone(&Utc).to_rfc3339(), false));
    }
    let date = time.date.as_deref()?;
    let naive = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)?;
    Some((Utc.from_utc_datetime(&naive).to_rfc3339(), true))
}

/// Map one non-cancelled `events.list` item to its cache row (spec 0032
/// "Mapping"). `None` when the event has no usable start/end (defensive —
/// Google always supplies them for non-cancelled instances).
///
/// `widen` (RC-1) is the capability gate for DL detection: `true` only when
/// `can_expand_groups == true`, so the broadened, speculative classification runs
/// exactly where Cloud Identity can confirm/correct it — and NEVER on the
/// non-expandable path, where a mis-flag would permanently hide a real person.
/// It's applied at map time (like the floor) so it flags once per synced event
/// rather than re-flagging every enrichment pass (which would re-query cleared
/// people forever).
pub(super) fn map_event(
    calendar_id: &str,
    event: &GcalEvent,
    widen: bool,
) -> Option<GoogleCalendarEventRow> {
    let (starts_at, all_day_start) = event.start.as_ref().and_then(parse_event_time)?;
    let (ends_at, _) = event.end.as_ref().and_then(parse_event_time)?;

    let title = event
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("(No title)")
        .to_string();

    // Zoom link: same extractor as the EventKit path, over every place Google
    // can stash a join URL (location, description, hangoutLink, conference
    // entry points).
    let mut candidates: Vec<&str> = vec![
        event.location.as_deref().unwrap_or(""),
        event.description.as_deref().unwrap_or(""),
        event.hangout_link.as_deref().unwrap_or(""),
    ];
    if let Some(conference) = &event.conference_data {
        candidates.extend(
            conference
                .entry_points
                .iter()
                .filter_map(|ep| ep.uri.as_deref()),
        );
    }
    let zoom_url = extract_zoom_url(candidates);

    // Attendees: rooms (`resource`) dropped; `self` → isCurrentUser (feeds
    // owner identity, spec 0018); DL members appear individually as Google
    // materializes them (the 0027 payoff). A group-shaped entry that Google
    // hasn't materialized is flagged `isDistributionList` so the roster excludes
    // it from person binding (labeled-DL floor) — Cloud Identity later folds in
    // its real members when the org grants access (specs/0038 WS3).
    let attendees_json: Vec<serde_json::Value> = event
        .attendees
        .iter()
        .filter(|a| !a.resource)
        .map(|a| {
            json!({
                "name": a
                    .display_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_string)
                    .or_else(|| a.email.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                "email": a.email,
                "isCurrentUser": a.is_self,
                "responseStatus": a.response_status,
                "isOrganizer": a.organizer,
                "isDistributionList": attendee_is_distribution_list(a, widen),
            })
        })
        .collect();

    let my_response = event
        .attendees
        .iter()
        .find(|a| a.is_self)
        .and_then(|a| a.response_status.clone());

    Some(GoogleCalendarEventRow {
        id: gcal_row_id(calendar_id, &event.id),
        calendar_id: calendar_id.to_string(),
        ical_uid: event.ical_uid.clone().filter(|u| !u.trim().is_empty()),
        title,
        starts_at,
        ends_at,
        is_all_day: all_day_start,
        location: event.location.clone().filter(|l| !l.trim().is_empty()),
        zoom_url,
        organizer_email: event.organizer.as_ref().and_then(|o| o.email.clone()),
        my_response,
        attendees_json: serde_json::to_string(&attendees_json).unwrap_or_else(|_| "[]".to_string()),
        status: event
            .status
            .clone()
            .unwrap_or_else(|| "confirmed".to_string()),
        updated_at: event
            .updated
            .clone()
            .unwrap_or_else(|| Utc::now().to_rfc3339()),
    })
}

/// Heuristic DL detection for the labeled-DL floor (specs/0038 WS3), built
/// unconditionally (no API). A distribution-list invite is group-shaped: it has
/// an email but NO personal display name (Google gives DLs none — in the floor),
/// it isn't the current user, a room resource, or the organizer, and its address
/// reads like a list (`eng-team@`, `all-hands@`, `sales-group@`, …).
///
/// `widen` (RC-1, specs/0038 WS3) is the capability gate. It is `true` ONLY when
/// `can_expand_groups == true` — i.e. Cloud Identity is available to
/// authoritatively CONFIRM (`Group` → fold members) or CORRECT
/// (`NotAGroup` → clear the flag) every guess. On that path we may speculate:
/// the personal-name hard-exclude is RELAXED (Google puts the GROUP's own name in
/// a group invite's `displayName`, so a real group would otherwise be excluded),
/// and the token list is slightly wider (see [`address_looks_like_group`]).
///
/// When `widen == false` (capability off/unknown — e.g. an org that 403s the
/// Groups API) we keep ONLY the ultra-conservative, high-precision floor and
/// NEVER speculate: a flag here is filtered out of the roster/speaker list with
/// no API to correct it, so a false positive would permanently hide a real
/// person. Precision over recall is mandatory on the non-expandable path.
pub(super) fn attendee_is_distribution_list(a: &GcalAttendee, widen: bool) -> bool {
    if a.is_self || a.resource || a.organizer {
        return false;
    }
    // The personal-name exclude is the floor's core safety guard; relaxed only on
    // the expandable path where Cloud Identity corrects a mis-flag.
    if !widen {
        let has_personal_name = a
            .display_name
            .as_deref()
            .map(str::trim)
            .map(|n| !n.is_empty())
            .unwrap_or(false);
        if has_personal_name {
            return false;
        }
    }
    let Some(email) = a.email.as_deref().map(str::trim).filter(|e| !e.is_empty()) else {
        return false;
    };
    let local = email.split('@').next().unwrap_or("");
    address_looks_like_group(local, widen)
}

/// True when an email local-part reads like a group/list rather than a person.
///
/// The BASE list is deliberately HIGH-PRECISION (finding #1): only clearly-group
/// tokens, so it can be trusted even on the non-expandable path where a mis-flag
/// permanently hides a real person. Ambiguous tokens (`staff`, `leads`,
/// `members`, `org`, `dept`, a bare `all`, …) are added ONLY when `widen`
/// (`can_expand_groups == true`), because there Cloud Identity confirms a real
/// group and CLEARS the flag when the API says the address is not a group — a
/// mis-flag is corrected, never a silent drop. Whole-token matching keeps
/// `priya`/`me` out while catching `eng-team`, `sales.group`, `dl-eng`; a small
/// exact-local set catches hyphenated words that tokenize apart (`all-hands`,
/// `no-reply`).
pub(super) fn address_looks_like_group(local: &str, widen: bool) -> bool {
    // Single group words matched as a whole `-._+`-separated token — the
    // high-precision floor, safe on BOTH paths.
    const GROUP_TOKENS: &[&str] = &[
        "team",
        "teams",
        "group",
        "groups",
        "dl",
        "list",
        "lists",
        "everyone",
        "allhands",
        "announce",
        "announcements",
        "noreply",
    ];
    // Group words that split across separators, matched against the full local.
    const GROUP_LOCALS: &[&str] = &["all-hands", "no-reply"];
    // Ambiguous list-ish tokens matched ONLY on the expandable path (`widen`),
    // where a mis-flag is authoritatively cleared by Cloud Identity. NEVER used
    // on the non-expandable path — each of these misflags real people
    // (`john.staff@`, `sarah.leads@`), which would silently hide them there.
    const GROUP_TOKENS_WIDE: &[&str] = &[
        "staff",
        "leads",
        "members",
        "org",
        "dept",
        "all",
        "everybody",
        "distribution",
        "notify",
        "notifications",
        "alerts",
        "committee",
        "council",
        "squad",
        "crew",
    ];

    let lower = local.to_ascii_lowercase();
    if GROUP_LOCALS.contains(&lower.as_str()) {
        return true;
    }
    lower
        .split(['-', '.', '_', '+'])
        .any(|part| GROUP_TOKENS.contains(&part) || (widen && GROUP_TOKENS_WIDE.contains(&part)))
}

#[cfg(test)]
pub(super) mod test_fixtures {
    //! Shared with `sync.rs`'s tests, which still exercise these types
    //! through the sync engine (specs/0054 W5).
    use super::GcalEvent;

    /// A meaty single event for map_event assertions: attendees with self /
    /// resource / a group address, conferenceData Zoom link, organizer.
    pub(in crate::calendar::google) const ATTENDEE_EVENT: &str = r#"{
        "id": "evt-attendees",
        "status": "confirmed",
        "summary": "Roadmap review",
        "iCalUID": "uid-att@google.com",
        "start": { "dateTime": "2026-07-02T15:00:00Z" },
        "end": { "dateTime": "2026-07-02T16:00:00Z" },
        "location": "HQ / 4th floor",
        "description": "agenda attached",
        "conferenceData": { "entryPoints": [ { "uri": "https://company.zoom.us/j/85512345678?pwd=abc" } ] },
        "organizer": { "email": "organizer@example.com" },
        "attendees": [
            { "email": "me@example.com", "displayName": "Me Myself", "self": true, "responseStatus": "accepted" },
            { "email": "organizer@example.com", "displayName": "Orga Nizer", "organizer": true, "responseStatus": "accepted" },
            { "email": "room-4a@resource.calendar.google.com", "displayName": "Room 4A", "resource": true, "responseStatus": "accepted" },
            { "email": "eng-team@example.com", "responseStatus": "needsAction" },
            { "email": "priya@example.com", "displayName": "Priya Patel", "responseStatus": "tentative" }
        ],
        "updated": "2026-07-01T12:00:00Z"
    }"#;

    pub(in crate::calendar::google) fn parse_fixture_event(fixture: &str) -> GcalEvent {
        serde_json::from_str(fixture).expect("fixture parses")
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::{parse_fixture_event, ATTENDEE_EVENT};
    use super::*;

    // --- map_event ---------------------------------------------------------

    #[test]
    fn map_event_builds_prefixed_instance_id_and_utc_times() {
        let event = parse_fixture_event(
            r#"{
                "id": "recur123_20260703T070000Z",
                "summary": "Standup",
                "iCalUID": "uid-recur@google.com",
                "start": { "dateTime": "2026-07-03T09:00:00+02:00" },
                "end": { "dateTime": "2026-07-03T09:15:00+02:00" }
            }"#,
        );
        let row = map_event("primary", &event, false).expect("maps");
        // Recurring instances keep their per-instance id under the gcal: prefix.
        assert_eq!(row.id, "gcal:primary/recur123_20260703T070000Z");
        assert_eq!(row.ical_uid.as_deref(), Some("uid-recur@google.com"));
        // Zoned dateTime is normalized to chrono's UTC to_rfc3339 — the exact
        // string the ext:<uid>@<start> dismissal keys compare against.
        assert_eq!(row.starts_at, "2026-07-03T07:00:00+00:00");
        assert_eq!(row.ends_at, "2026-07-03T07:15:00+00:00");
        assert!(!row.is_all_day);
    }

    #[test]
    fn map_event_detects_all_day_and_titles_fallback() {
        let event = parse_fixture_event(
            r#"{
                "id": "evt-allday",
                "start": { "date": "2026-07-10" },
                "end": { "date": "2026-07-11" }
            }"#,
        );
        let row = map_event("primary", &event, false).expect("maps");
        assert!(row.is_all_day, "date-only start means all-day");
        assert_eq!(row.starts_at, "2026-07-10T00:00:00+00:00");
        assert_eq!(row.title, "(No title)", "missing summary falls back");
    }

    #[test]
    fn map_event_keeps_non_ascii_titles() {
        let event = parse_fixture_event(
            r#"{
                "id": "evt-plain",
                "status": "confirmed",
                "summary": "Übergabe — 打ち合わせ",
                "start": { "dateTime": "2026-07-02T09:00:00+02:00" },
                "end": { "dateTime": "2026-07-02T10:00:00+02:00" }
            }"#,
        );
        let row = map_event("primary", &event, false).expect("maps");
        assert_eq!(row.title, "Übergabe — 打ち合わせ");
    }

    #[test]
    fn map_event_attendees_self_resource_group_and_zoom() {
        let event = parse_fixture_event(ATTENDEE_EVENT);
        let row = map_event("primary", &event, false).expect("maps");

        assert_eq!(
            row.organizer_email.as_deref(),
            Some("organizer@example.com")
        );
        assert_eq!(row.my_response.as_deref(), Some("accepted"));
        // Zoom link pulled out of conferenceData entry points.
        assert_eq!(
            row.zoom_url.as_deref(),
            Some("https://company.zoom.us/j/85512345678?pwd=abc")
        );

        let attendees: Vec<serde_json::Value> =
            serde_json::from_str(&row.attendees_json).expect("valid json");
        // The room resource is dropped; self + organizer + group + person stay.
        assert_eq!(attendees.len(), 4);
        assert!(
            !row.attendees_json.contains("room-4a@"),
            "resource attendees (rooms) must be dropped"
        );

        let me = &attendees[0];
        assert_eq!(me["name"], "Me Myself");
        assert_eq!(me["isCurrentUser"], true);
        assert_eq!(me["responseStatus"], "accepted");
        let organizer = &attendees[1];
        assert_eq!(organizer["isOrganizer"], true);
        assert_eq!(organizer["isCurrentUser"], false);
        // The group address (a DL not yet materialized) rides through as one
        // attendee, flagged so the roster excludes it from person binding.
        let group = &attendees[2];
        assert_eq!(group["name"], "eng-team@example.com");
        assert_eq!(group["email"], "eng-team@example.com");
        assert_eq!(group["responseStatus"], "needsAction");
        assert_eq!(group["isDistributionList"], true, "group-shaped, no name");
        // Real people (self + organizer + Priya) are never flagged.
        assert_eq!(me["isDistributionList"], false);
        assert_eq!(organizer["isDistributionList"], false);
        assert_eq!(attendees[3]["isDistributionList"], false);

        // The cached JSON round-trips into the wire Attendee shape, with the
        // cached photo (by lowercased email) folded onto the matching attendee
        // and everyone else left `None` (→ initials).
        let mut photos = std::collections::HashMap::new();
        photos.insert(
            "priya@example.com".to_string(),
            "data:image/jpeg;base64,priyaface".to_string(),
        );
        let wire = super::super::sync::attendees_from_json(&row.attendees_json, &photos);
        assert_eq!(wire.len(), 4);
        assert!(wire[0].is_current_user);
        assert!(
            wire[2].is_distribution_list,
            "the DL flag survives round-trip"
        );
        assert!(!wire[3].is_distribution_list);
        assert_eq!(wire[3].name, "Priya Patel");
        assert_eq!(wire[3].email.as_deref(), Some("priya@example.com"));
        assert_eq!(
            wire[3].photo_data_uri.as_deref(),
            Some("data:image/jpeg;base64,priyaface"),
            "the cached photo is folded onto the matching attendee by email"
        );
        assert!(
            wire[0].photo_data_uri.is_none(),
            "attendees with no cached photo degrade to initials (None)"
        );
    }

    #[test]
    fn distribution_list_detection_flags_groups_but_not_people() {
        let dl = |email: &str, name: Option<&str>| GcalAttendee {
            email: Some(email.into()),
            display_name: name.map(str::to_string),
            response_status: Some("needsAction".into()),
            is_self: false,
            resource: false,
            organizer: false,
        };
        // --- Floor (non-expandable path, widen=false): high-precision only ---
        // Group-shaped addresses with no personal name are flagged.
        assert!(attendee_is_distribution_list(
            &dl("eng-team@example.com", None),
            false
        ));
        assert!(attendee_is_distribution_list(
            &dl("all-hands@example.com", None),
            false
        ));
        assert!(attendee_is_distribution_list(
            &dl("allhands@example.com", None),
            false
        ));
        assert!(attendee_is_distribution_list(
            &dl("dl-sales@example.com", None),
            false
        ));
        assert!(attendee_is_distribution_list(
            &dl("no-reply@example.com", None),
            false
        ));
        assert!(attendee_is_distribution_list(
            &dl("noreply@example.com", None),
            false
        ));
        assert!(attendee_is_distribution_list(
            &dl("sales.group@example.com", None),
            false
        ));
        // A real person: plain address, no group token.
        assert!(!attendee_is_distribution_list(
            &dl("priya@example.com", None),
            false
        ));
        // High-precision floor (finding #1): ambiguous tokens that appear in real
        // people's addresses are NOT flagged — precision over recall, since Cloud
        // Identity confirms/corrects the label. A false positive would drop a
        // real person from the roster.
        assert!(!attendee_is_distribution_list(
            &dl("john.staff@example.com", None),
            false
        ));
        assert!(!attendee_is_distribution_list(
            &dl("sarah.leads@example.com", None),
            false
        ));
        assert!(!attendee_is_distribution_list(
            &dl("dept-x@example.com", None),
            false
        ));
        assert!(!attendee_is_distribution_list(
            &dl("all@example.com", None),
            false
        ));
        // A named attendee is a person even if the address looks group-y.
        assert!(!attendee_is_distribution_list(
            &dl("team.lead@example.com", Some("Team Lead")),
            false
        ));
        // self / resource / organizer are never distribution lists.
        let mut me = dl("eng-team@example.com", None);
        me.is_self = true;
        assert!(!attendee_is_distribution_list(&me, false));

        // --- (b) RC-1 real-person safety: on the non-expandable path an ambiguous
        // address is NEVER flagged, because a mis-flag here permanently hides a
        // real attendee (no Cloud Identity to correct it). ---
        assert!(!attendee_is_distribution_list(
            &dl("staff@example.com", None),
            false
        ));
        assert!(!attendee_is_distribution_list(
            &dl("committee@example.com", None),
            false
        ));
        // And a NAMED group invite stays a person on the floor (displayName exclude
        // stands) — precision over recall when we can't confirm.
        assert!(!attendee_is_distribution_list(
            &dl("eng-team@example.com", Some("Engineering Team")),
            false
        ));

        // --- (a) RC-1 expandable path (widen=true): a real group invite carrying
        // the GROUP's own displayName IS detected, because Cloud Identity will
        // confirm it (fold members) or correct a mis-flag (clear it). ---
        assert!(
            attendee_is_distribution_list(
                &dl("eng-team@example.com", Some("Engineering Team")),
                true
            ),
            "a named group invite is detected once expansion can confirm/correct it"
        );
        // The slightly-wider token list also catches ambiguous list-ish addresses
        // ONLY here (corrected if wrong).
        assert!(attendee_is_distribution_list(
            &dl("staff@example.com", None),
            true
        ));
        assert!(attendee_is_distribution_list(
            &dl("committee@example.com", None),
            true
        ));
        // self / resource / organizer are STILL never a DL, even widened.
        let mut me_wide = dl("eng-team@example.com", Some("Engineering Team"));
        me_wide.is_self = true;
        assert!(!attendee_is_distribution_list(&me_wide, true));
        // A plain personal address is not a group even widened (no group token).
        assert!(!attendee_is_distribution_list(
            &dl("priya@example.com", Some("Priya")),
            true
        ));
    }
}
