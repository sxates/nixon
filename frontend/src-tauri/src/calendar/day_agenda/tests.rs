use super::*;
use crate::database::models::{DateTimeUtc, ManualScheduledRow};

fn rec(id: &str, title: &str, created: DateTime<Utc>, folder: bool) -> MeetingStatusRow {
    MeetingStatusRow {
        id: id.to_string(),
        title: title.to_string(),
        created_at: DateTimeUtc(created),
        folder_path: if folder { Some("/tmp/x".into()) } else { None },
        duration_seconds: Some(600.0),
        has_folder: folder as i64,
        has_transcript: 1,
        has_summary: 0,
        has_speakers: 0,
    }
}

fn evt(id: &str, title: &str, start: DateTime<Utc>) -> UpcomingMeeting {
    UpcomingMeeting {
        id: id.to_string(),
        title: title.to_string(),
        starts_at: start.to_rfc3339(),
        ends_at: (start + Duration::hours(1)).to_rfc3339(),
        calendar_name: "Work".into(),
        location: None,
        zoom_url: None,
        external_id: None,
    }
}

fn evt_ext(id: &str, external_id: &str, title: &str, start: DateTime<Utc>) -> UpcomingMeeting {
    UpcomingMeeting {
        external_id: Some(external_id.to_string()),
        ..evt(id, title, start)
    }
}

#[test]
fn matches_recording_to_event_by_title_and_window() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 15, 0, 0).unwrap();
    let events = vec![evt("ev1", "Standup", t0)];
    // Recording started 10 min late — still the same meeting.
    let recordings = vec![rec(
        "meeting-1",
        "standup",
        t0 + Duration::minutes(10),
        true,
    )];

    let items = build_agenda(
        events,
        recordings,
        vec![],
        &std::collections::HashSet::new(),
        |_, _| vec![],
    );
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.id, "meeting-1");
    assert_eq!(item.meeting_id.as_deref(), Some("meeting-1"));
    assert!(matches!(item.source, AgendaSource::Calendar));
    assert!(item.status.recorded);
    assert!(item.status.transcribed);
}

#[test]
fn unmatched_recording_becomes_standalone_item() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 9, 0, 0).unwrap();
    let events = vec![evt("ev1", "Standup", t0)];
    // Ad-hoc recording with a different title, no event.
    let recordings = vec![rec(
        "meeting-adhoc",
        "Quick chat",
        t0 + Duration::hours(2),
        true,
    )];

    let items = build_agenda(
        events,
        recordings,
        vec![],
        &std::collections::HashSet::new(),
        |_, _| vec![],
    );
    assert_eq!(items.len(), 2);
    // Sorted by start: event (09:00) first, then recording (11:00).
    assert!(matches!(items[0].source, AgendaSource::Calendar));
    assert!(items[0].meeting_id.is_none());
    assert!(matches!(items[1].source, AgendaSource::Recording));
    assert_eq!(items[1].meeting_id.as_deref(), Some("meeting-adhoc"));
}

#[test]
fn far_apart_same_title_does_not_match() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 9, 0, 0).unwrap();
    let events = vec![evt("ev1", "Standup", t0)];
    // Same title but 3 hours later -> outside MATCH_WINDOW -> standalone.
    let recordings = vec![rec("meeting-2", "Standup", t0 + Duration::hours(3), true)];

    let items = build_agenda(
        events,
        recordings,
        vec![],
        &std::collections::HashSet::new(),
        |_, _| vec![],
    );
    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|i| i.meeting_id.is_none())); // event, unmatched
    assert!(items
        .iter()
        .any(|i| i.meeting_id.as_deref() == Some("meeting-2")
            && matches!(i.source, AgendaSource::Recording)));
}

#[test]
fn attendees_capped_with_full_count() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 9, 0, 0).unwrap();
    let events = vec![evt("ev1", "All Hands", t0)];
    let make = |n: usize| Attendee {
        name: format!("p{n}"),
        email: Some(format!("p{n}@x.com")),
        is_current_user: false,
        is_distribution_list: false,
        photo_data_uri: None,
    };
    let attendees: Vec<Attendee> = (0..8).map(make).collect();
    let items = build_agenda(
        events,
        vec![],
        vec![],
        &std::collections::HashSet::new(),
        |_, _| attendees.clone(),
    );
    assert_eq!(items[0].attendees.len(), MAX_INLINE_ATTENDEES);
    assert_eq!(items[0].attendee_count, 8);
}

#[test]
fn dismissed_unrecorded_event_is_flagged_recorded_one_is_not() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 9, 0, 0).unwrap();
    let events = vec![
        evt("ev-doctor", "Doctor appt", t0),
        evt("ev-rec", "Standup", t0),
    ];
    // "Standup" was recorded; "Doctor appt" was not.
    let recordings = vec![rec("meeting-1", "standup", t0 + Duration::minutes(5), true)];
    let mut dismissed = std::collections::HashSet::new();
    dismissed.insert("ev-doctor".to_string());
    dismissed.insert("ev-rec".to_string()); // dismissing a recorded event must be ignored

    let items = build_agenda(events, recordings, vec![], &dismissed, |_, _| vec![]);
    let doctor = items.iter().find(|i| i.title == "Doctor appt").unwrap();
    let standup = items.iter().find(|i| i.title == "Standup").unwrap();
    assert!(
        doctor.dismissed,
        "an unrecorded dismissed event is flagged dismissed"
    );
    assert!(
        !standup.dismissed,
        "a dismissed id that matched a recording must NOT be hidden (you recorded it)"
    );
}

// -- specs/0029 WS6.2: sync-stable dismissal key ----------------------------

#[test]
fn dismiss_key_prefers_external_identifier_with_occurrence_start() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 9, 0, 0).unwrap();
    let events = vec![evt_ext("ek-abc", "EXT-123", "Dentist", t0)];

    let items = build_agenda(
        events,
        vec![],
        vec![],
        &std::collections::HashSet::new(),
        |_, _| vec![],
    );
    assert_eq!(
        items[0].dismiss_key,
        format!("ext:EXT-123@{}", t0.to_rfc3339())
    );
    // The row id (used for eventWithIdentifier lookups etc.) stays the EventKit id.
    assert_eq!(items[0].id, "ek-abc");
}

#[test]
fn dismiss_key_falls_back_to_event_id_without_external_identifier() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 9, 0, 0).unwrap();
    let items = build_agenda(
        vec![evt("ek-abc", "Dentist", t0)],
        vec![],
        vec![],
        &std::collections::HashSet::new(),
        |_, _| vec![],
    );
    assert_eq!(items[0].dismiss_key, "ek-abc");

    // Empty EventKit id -> synthetic key for both id and dismiss key.
    let items = build_agenda(
        vec![evt("", "Dentist", t0)],
        vec![],
        vec![],
        &std::collections::HashSet::new(),
        |_, _| vec![],
    );
    assert!(items[0].id.starts_with("evt-"));
    assert_eq!(items[0].dismiss_key, items[0].id);
}

#[test]
fn dismissal_matches_either_stable_or_legacy_key() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 9, 0, 0).unwrap();

    // Stored under the NEW stable key -> dismissed, even if the provider
    // reissued the eventIdentifier between reads (id differs, external stable).
    let mut by_stable = std::collections::HashSet::new();
    by_stable.insert(format!("ext:EXT-123@{}", t0.to_rfc3339()));
    let items = build_agenda(
        vec![evt_ext("ek-REISSUED", "EXT-123", "Dentist", t0)],
        vec![],
        vec![],
        &by_stable,
        |_, _| vec![],
    );
    assert!(
        items[0].dismissed,
        "stable-key dismissal survives an eventIdentifier reissue"
    );

    // Stored under the LEGACY eventIdentifier (pre-0029 dismissals) -> still dismissed.
    let mut by_legacy = std::collections::HashSet::new();
    by_legacy.insert("ek-abc".to_string());
    let items = build_agenda(
        vec![evt_ext("ek-abc", "EXT-123", "Dentist", t0)],
        vec![],
        vec![],
        &by_legacy,
        |_, _| vec![],
    );
    assert!(
        items[0].dismissed,
        "legacy-key dismissals keep working (dual-key read)"
    );
}

// -- specs/0041 WS5: iCalUID dedup + dismissal interplay ---------------------

#[test]
fn dismissal_still_applies_after_duplicate_events_collapse() {
    let t0 = Utc.with_ymd_and_hms(2026, 7, 8, 9, 0, 0).unwrap();
    // The same invite read from two calendars (same external id + start,
    // different event ids) — the read-path dedup collapses them to one.
    let duplicates = vec![
        evt_ext("ek-work", "UID-1", "Standup", t0),
        evt_ext("ek-team", "UID-1", "Standup", t0),
    ];
    let events = crate::calendar::eventkit::dedup_by_external_id(duplicates);
    assert_eq!(events.len(), 1, "one bubble per invite");

    // A dismissal stored under the stable `ext:` key (written BEFORE the
    // dedup existed, possibly via the copy that got dropped) still hides
    // the survivor: the key depends only on the shared UID + start.
    let mut dismissed = std::collections::HashSet::new();
    dismissed.insert(format!("ext:UID-1@{}", t0.to_rfc3339()));
    let items = build_agenda(events, vec![], vec![], &dismissed, |_, _| vec![]);
    assert_eq!(items.len(), 1);
    assert!(
        items[0].dismissed,
        "dismissals keep working across the duplicate collapse"
    );
}

#[test]
fn recurring_occurrences_share_external_id_but_get_distinct_dismiss_keys() {
    let t0 = Utc.with_ymd_and_hms(2026, 6, 25, 9, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 6, 25, 14, 0, 0).unwrap();
    let events = vec![
        evt_ext("ek-1", "EXT-RECUR", "Standup", t0),
        evt_ext("ek-2", "EXT-RECUR", "Standup", t1),
    ];

    let items = build_agenda(
        events,
        vec![],
        vec![],
        &std::collections::HashSet::new(),
        |_, _| vec![],
    );
    assert_ne!(
        items[0].dismiss_key, items[1].dismiss_key,
        "hiding one occurrence must not hide the whole series"
    );
}

/// specs/0069 W3 — a calendar event and a manual entry with the SAME title and start.
/// The manual one must not be mistaken for a recording of the event, and the event must
/// not come back marked recorded. This is the invariant `get_between_with_status`'s
/// `origin <> 'scheduled'` exclusion exists to protect.
#[test]
fn manual_entries_are_appended_and_sorted_but_never_matched() {
    let t0 = Utc.with_ymd_and_hms(2026, 9, 20, 15, 0, 0).unwrap();
    let events = vec![evt("ev1", "Call with Sam", t0)];
    let manual = crate::calendar::manual_items::manual_agenda_items(vec![ManualScheduledRow {
        id: "meeting-manual".to_string(),
        title: "Call with Sam".to_string(),
        created_at: DateTimeUtc(t0),
        scheduled_end_at: Some(DateTimeUtc(t0 + Duration::minutes(30))),
        join_url: None,
        calendar_event_id: "nixon-manual:test-uuid".to_string(),
    }]);

    let items = build_agenda(
        events,
        vec![],
        manual,
        &std::collections::HashSet::new(),
        |_, _| vec![],
    );

    assert_eq!(items.len(), 2, "two rows, not one merged row");
    let calendar_item = items
        .iter()
        .find(|i| matches!(i.source, AgendaSource::Calendar))
        .expect("the calendar event is still its own row");
    assert!(!calendar_item.status.recorded);
    assert!(calendar_item.meeting_id.is_none());
    assert!(items.windows(2).all(|w| w[0].start_time <= w[1].start_time));
}

// -- `is_event_dismissed`: the predicate two more callers now share ----------
//
// Extracted 2026-09-21 because `api_get_upcoming_meetings` (which drives the T-5 prep and
// T-0 join notifications) and `prep_jobs::upcoming_for_horizon` (an LLM call per event)
// both read the raw calendar and never consulted the dismissal set. Hiding "Lunch" removed
// it from Today and still bought two banners and a prep brief.

#[test]
fn is_event_dismissed_honours_the_legacy_key() {
    let t0 = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
    let event = evt("ev-lunch", "Lunch", t0);
    let mut dismissed = std::collections::HashSet::new();
    dismissed.insert("ev-lunch".to_string());
    assert!(is_event_dismissed(&event, &dismissed));
}

#[test]
fn is_event_dismissed_honours_the_stable_key() {
    let t0 = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
    let event = evt_ext("ev-lunch", "ical-lunch", "Lunch", t0);
    let mut dismissed = std::collections::HashSet::new();
    dismissed.insert(format!("ext:ical-lunch@{}", t0.to_rfc3339()));
    assert!(is_event_dismissed(&event, &dismissed));
}

/// The case that rots silently: a dismissal written before the stable key existed, stored
/// under the event identifier, on an event that now HAS an external id. A read that only
/// checked the stable key would quietly start notifying again.
#[test]
fn is_event_dismissed_honours_a_legacy_key_on_an_event_with_an_external_id() {
    let t0 = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
    let event = evt_ext("ek-abc", "ical-lunch", "Lunch", t0);
    let mut dismissed = std::collections::HashSet::new();
    dismissed.insert("ek-abc".to_string());
    assert!(
        is_event_dismissed(&event, &dismissed),
        "a pre-0029 dismissal must keep working"
    );
}

#[test]
fn is_event_dismissed_uses_the_synthetic_key_when_the_provider_gave_no_id() {
    let t0 = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
    let event = evt("", "Lunch", t0);
    let mut dismissed = std::collections::HashSet::new();
    dismissed.insert(synthetic_event_id("Lunch", &t0.to_rfc3339()));
    assert!(is_event_dismissed(&event, &dismissed));
}

#[test]
fn is_event_dismissed_is_false_for_an_event_nobody_hid() {
    let t0 = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
    let event = evt_ext("ev-standup", "ical-standup", "Standup", t0);
    let mut dismissed = std::collections::HashSet::new();
    dismissed.insert("ev-lunch".to_string());
    assert!(!is_event_dismissed(&event, &dismissed));
}

/// A recurring event's dismissal is per-occurrence: the external id is shared by the whole
/// series, so hiding Monday's stand-up must not hide Tuesday's.
#[test]
fn is_event_dismissed_does_not_leak_across_occurrences_of_a_series() {
    let mon = Utc.with_ymd_and_hms(2026, 9, 21, 9, 0, 0).unwrap();
    let tue = Utc.with_ymd_and_hms(2026, 9, 22, 9, 0, 0).unwrap();
    let mut dismissed = std::collections::HashSet::new();
    dismissed.insert(format!("ext:ical-standup@{}", mon.to_rfc3339()));

    assert!(is_event_dismissed(
        &evt_ext("occ-mon", "ical-standup", "Standup", mon),
        &dismissed
    ));
    assert!(!is_event_dismissed(
        &evt_ext("occ-tue", "ical-standup", "Standup", tue),
        &dismissed
    ));
}

#[test]
fn dismissal_keys_reports_stable_then_legacy() {
    let t0 = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
    let (stable, legacy) = dismissal_keys(&evt_ext("ek-abc", "ical-x", "Lunch", t0));
    assert_eq!(stable, format!("ext:ical-x@{}", t0.to_rfc3339()));
    assert_eq!(legacy, "ek-abc");

    // With no external id both are the legacy key, so a caller can treat them uniformly.
    let (stable, legacy) = dismissal_keys(&evt("ek-abc", "Lunch", t0));
    assert_eq!(stable, "ek-abc");
    assert_eq!(legacy, "ek-abc");
}
