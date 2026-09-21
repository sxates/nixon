//! Fixture dataset types (specs/0059). Content lives in `fixtures/demo/` and is embedded
//! at compile time so the seeder never depends on the working directory.
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct FixturePerson {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureSpeaker {
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub is_local: bool,
    #[serde(default)]
    pub person_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureSegment {
    pub id: String,
    #[serde(default)]
    pub speaker: Option<String>,
    /// "microphone" | "system" | "mixed"
    pub channel: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureActionItem {
    pub description: String,
    /// `"self"`, a person id, or null (unassigned)
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub due_hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureMeeting {
    pub id: String,
    pub title: String,
    /// Re-based at seed time: 0 = today, 1 = yesterday, …
    pub days_ago: u32,
    /// "HH:MM" local wall-clock start
    pub time_of_day: String,
    pub duration_seconds: u32,
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub participants: Vec<String>,
    pub speakers: Vec<FixtureSpeaker>,
    pub segments: Vec<FixtureSegment>,
    #[serde(default)]
    pub summary_markdown: Option<String>,
    #[serde(default)]
    pub notes_markdown: Option<String>,
    #[serde(default)]
    pub action_items: Vec<FixtureActionItem>,
}

/// A meeting the user added inside Nixon rather than one a recording produced — a
/// `scheduled`-origin row with no transcript, seeded straight into the day timeline so
/// Today shows upcoming, not-yet-recorded entries even with no calendar connected
/// (specs/0069 W3; see `database/repositories/meeting/manual.rs`).
#[derive(Debug, Clone, Deserialize)]
pub struct FixtureManualMeeting {
    pub id: String,
    pub title: String,
    /// Re-based at seed time: 0 = today, 1 = yesterday, …
    pub days_ago: u32,
    /// "HH:MM" local wall-clock start
    pub time_of_day: String,
    pub duration_minutes: u32,
    #[serde(default)]
    pub join_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Dataset {
    pub people: Vec<FixturePerson>,
    pub meetings: Vec<FixtureMeeting>,
    #[serde(default)]
    pub manual_meetings: Vec<FixtureManualMeeting>,
}

const PEOPLE: &str = include_str!("../../fixtures/demo/people.json");
const MANUAL_MEETINGS: &str = include_str!("../../fixtures/demo/manual_meetings.json");
const MEETINGS: [&str; 6] = [
    include_str!("../../fixtures/demo/meetings/01-product-sync.json"),
    include_str!("../../fixtures/demo/meetings/02-one-on-one.json"),
    include_str!("../../fixtures/demo/meetings/03-steerco.json"),
    include_str!("../../fixtures/demo/meetings/04-standup.json"),
    include_str!("../../fixtures/demo/meetings/05-unprocessed.json"),
    include_str!("../../fixtures/demo/meetings/06-design-review.json"),
];

pub fn load_embedded() -> Result<Dataset, String> {
    let people: Vec<FixturePerson> =
        serde_json::from_str(PEOPLE).map_err(|e| format!("people.json: {e}"))?;
    let mut meetings = Vec::with_capacity(MEETINGS.len());
    for (i, raw) in MEETINGS.iter().enumerate() {
        let m: FixtureMeeting =
            serde_json::from_str(raw).map_err(|e| format!("meeting file #{}: {e}", i + 1))?;
        meetings.push(m);
    }
    let manual_meetings: Vec<FixtureManualMeeting> =
        serde_json::from_str(MANUAL_MEETINGS).map_err(|e| format!("manual_meetings.json: {e}"))?;
    Ok(Dataset {
        people,
        meetings,
        manual_meetings,
    })
}

/// Every rule the seeder relies on. Returns all violations, not just the first.
pub fn validate(ds: &Dataset) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    let people: std::collections::HashSet<&str> = ds.people.iter().map(|p| p.id.as_str()).collect();
    let mut seen_ids = std::collections::HashSet::new();
    for m in &ds.meetings {
        if !seen_ids.insert(m.id.as_str()) {
            errs.push(format!("{}: duplicate meeting id", m.id));
        }
        for p in &m.participants {
            if !people.contains(p.as_str()) {
                errs.push(format!("{}: unknown participant {p}", m.id));
            }
        }
        let keys: std::collections::HashSet<&str> =
            m.speakers.iter().map(|s| s.key.as_str()).collect();
        for s in &m.speakers {
            if let Some(pid) = &s.person_id {
                if !people.contains(pid.as_str()) {
                    errs.push(format!(
                        "{}: speaker {} references unknown person {pid}",
                        m.id, s.key
                    ));
                }
            }
        }
        let mut last_end = -1.0_f64;
        for seg in &m.segments {
            if let Some(k) = &seg.speaker {
                if k != "unknown" && !keys.contains(k.as_str()) {
                    errs.push(format!(
                        "{}: segment {} uses unknown speaker key {k}",
                        m.id, seg.id
                    ));
                }
            }
            if !matches!(seg.channel.as_str(), "microphone" | "system" | "mixed") {
                errs.push(format!(
                    "{}: segment {} bad channel {}",
                    m.id, seg.id, seg.channel
                ));
            }
            if seg.start < last_end || seg.end <= seg.start {
                errs.push(format!("{}: segment {} timing not monotonic", m.id, seg.id));
            }
            if seg.end > m.duration_seconds as f64 {
                errs.push(format!("{}: segment {} ends after duration", m.id, seg.id));
            }
            last_end = seg.end;
        }
        for ai in &m.action_items {
            if let Some(a) = &ai.assignee {
                if a != "self" && !people.contains(a.as_str()) {
                    errs.push(format!("{}: action item assignee {a} unknown", m.id));
                }
            }
        }
    }
    // Manual entries land in the same `meetings` table (specs/0069 W3), so an id collision
    // with a recorded meeting — or between two manual entries — would silently overwrite one
    // insert with the other at seed time instead of failing loudly here.
    for m in &ds.manual_meetings {
        if !seen_ids.insert(m.id.as_str()) {
            errs.push(format!("{}: duplicate meeting id", m.id));
        }
        if chrono::NaiveTime::parse_from_str(&m.time_of_day, "%H:%M").is_err() {
            errs.push(format!(
                "{}: bad time_of_day {}",
                m.id, m.time_of_day
            ));
        }
        if m.duration_minutes == 0 {
            errs.push(format!("{}: duration_minutes must be nonzero", m.id));
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> Dataset {
        Dataset {
            people: vec![FixturePerson {
                id: "person-a".into(),
                display_name: "A".into(),
                email: Some("a@halden.example".into()),
                role: None,
            }],
            meetings: vec![FixtureMeeting {
                id: "demo-01".into(),
                title: "T".into(),
                days_ago: 0,
                time_of_day: "14:00".into(),
                duration_seconds: 60,
                template_id: None,
                participants: vec!["person-a".into()],
                speakers: vec![FixtureSpeaker {
                    key: "local".into(),
                    display_name: "You".into(),
                    is_local: true,
                    person_id: None,
                }],
                segments: vec![FixtureSegment {
                    id: "seg_1".into(),
                    speaker: Some("local".into()),
                    channel: "microphone".into(),
                    start: 0.0,
                    end: 3.0,
                    text: "hi".into(),
                }],
                summary_markdown: None,
                notes_markdown: None,
                action_items: vec![],
            }],
            manual_meetings: vec![],
        }
    }

    #[test]
    fn embedded_dataset_loads_and_validates() {
        let ds = load_embedded().expect("embedded fixtures parse");
        assert_eq!(ds.meetings.len(), 6);
        assert_eq!(ds.people.len(), 7);
        assert_eq!(ds.manual_meetings.len(), 3);
        validate(&ds).expect("embedded fixtures valid");
    }

    #[test]
    fn validate_rejects_a_manual_id_that_collides_with_a_recorded_meeting() {
        let mut ds = minimal();
        ds.manual_meetings.push(FixtureManualMeeting {
            id: "demo-01".into(),
            title: "Dup".into(),
            days_ago: 0,
            time_of_day: "15:00".into(),
            duration_minutes: 30,
            join_url: None,
        });
        let errs = validate(&ds).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("duplicate meeting id")), "{errs:?}");
    }

    #[test]
    fn validate_rejects_a_bad_manual_time_of_day() {
        let mut ds = minimal();
        ds.manual_meetings.push(FixtureManualMeeting {
            id: "demo-99".into(),
            title: "Bad time".into(),
            days_ago: 0,
            time_of_day: "not-a-time".into(),
            duration_minutes: 30,
            join_url: None,
        });
        let errs = validate(&ds).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("bad time_of_day")), "{errs:?}");
    }

    #[test]
    fn validate_rejects_unknown_speaker_key() {
        let mut ds = minimal();
        ds.meetings[0].segments[0].speaker = Some("spk_9".into());
        let errs = validate(&ds).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("spk_9")), "{errs:?}");
    }

    #[test]
    fn validate_rejects_non_monotonic_timing() {
        let mut ds = minimal();
        ds.meetings[0].segments.push(FixtureSegment {
            id: "seg_2".into(),
            speaker: None,
            channel: "mixed".into(),
            start: 2.0,
            end: 4.0,
            text: "x".into(),
        });
        let errs = validate(&ds).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("monotonic")), "{errs:?}");
    }

    #[test]
    fn validate_rejects_unknown_participant_and_end_past_duration() {
        let mut ds = minimal();
        ds.meetings[0].participants.push("person-zzz".into());
        ds.meetings[0].segments[0].end = 61.0;
        let errs = validate(&ds).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("person-zzz")));
        assert!(errs.iter().any(|e| e.contains("duration")));
    }
}
