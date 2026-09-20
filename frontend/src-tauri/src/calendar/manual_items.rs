//! specs/0069 W3 — manual entries as agenda items.
//!
//! Kept out of `day_agenda.rs` so its matching loop stays untouched: a manual entry is
//! never matched against a calendar event or a recording, it is simply one more row in the
//! sorted day. It is also why these come in pre-shaped — `build_agenda` appends them.

use crate::calendar::day_agenda::{AgendaSource, AgendaStatus, DayAgendaItem};
use crate::database::models::ManualScheduledRow;

pub fn manual_agenda_items(rows: Vec<ManualScheduledRow>) -> Vec<DayAgendaItem> {
    rows.into_iter()
        .map(|row| DayAgendaItem {
            // The row id IS the meeting id, so a click opens its Prep tab with no
            // `api_ensure_scheduled_meeting` round-trip.
            id: row.id.clone(),
            title: row.title,
            start_time: row.created_at.0.to_rfc3339(),
            end_time: row.scheduled_end_at.map(|e| e.0.to_rfc3339()),
            source: AgendaSource::Manual,
            zoom_url: row.join_url,
            attendees: Vec::new(),
            attendee_count: 0,
            meeting_id: Some(row.id),
            status: AgendaStatus::empty(),
            // You added it; the way to get rid of it is to delete it, not to hide it.
            dismissed: false,
            dismiss_key: String::new(),
            series_key: None,
            // specs/0069 W3 Ruling 1 — carried so record-start adoption (`joinAndRecord` →
            // `api_create_meeting`) can match on this and adopt the prep row instead of
            // minting a second meeting and silently losing what the user wrote.
            calendar_event_id: Some(row.calendar_event_id),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::models::DateTimeUtc;

    fn row(id: &str, start: &str, end: Option<&str>, join: Option<&str>) -> ManualScheduledRow {
        ManualScheduledRow {
            id: id.to_string(),
            title: "Call with Sam".to_string(),
            created_at: DateTimeUtc(start.parse().unwrap()),
            scheduled_end_at: end.map(|e| DateTimeUtc(e.parse().unwrap())),
            join_url: join.map(str::to_string),
            calendar_event_id: format!("nixon-manual:{id}"),
        }
    }

    #[test]
    fn a_manual_entry_carries_its_meeting_id_and_is_not_recorded() {
        let items = manual_agenda_items(vec![row(
            "meeting-1",
            "2026-09-20T15:00:00Z",
            Some("2026-09-20T15:30:00Z"),
            Some("https://zoom.us/j/1"),
        )]);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.id, "meeting-1", "its id IS its meeting id — prep opens directly");
        assert_eq!(it.meeting_id.as_deref(), Some("meeting-1"));
        assert!(matches!(it.source, AgendaSource::Manual));
        assert!(!it.status.recorded);
        assert!(!it.dismissed, "you added it; hiding it would be deleting it");
        assert_eq!(it.zoom_url.as_deref(), Some("https://zoom.us/j/1"));
        assert!(it.end_time.is_some());
        assert!(it.attendees.is_empty());
        assert_eq!(it.attendee_count, 0);
        assert_eq!(
            it.calendar_event_id.as_deref(),
            Some("nixon-manual:meeting-1"),
            "record-start adoption matches on this field"
        );
    }

    #[test]
    fn an_entry_with_no_end_time_has_none() {
        let items = manual_agenda_items(vec![row("meeting-2", "2026-09-20T15:00:00Z", None, None)]);
        assert!(items[0].end_time.is_none());
        assert!(items[0].zoom_url.is_none());
    }
}
