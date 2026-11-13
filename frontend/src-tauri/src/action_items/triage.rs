//! Triage of an action item (specs/0088): which surface it belongs on.

use std::str::FromStr;

use crate::action_items::diff::ResolvedCandidate;

/// Completed items stay mergeable targets for this many days (specs/0088 absorption).
pub const ABSORB_COMPLETED_WITHIN_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Triage {
    /// Inbox: extracted, and the assignee is the owner or nobody.
    Suggested,
    /// On the owner's to-do list (adopted, or typed by hand).
    Listed,
    /// Someone else's item: part of the meeting record, not tracked.
    Record,
}

impl Triage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Triage::Suggested => "suggested",
            Triage::Listed => "listed",
            Triage::Record => "record",
        }
    }
}

impl FromStr for Triage {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "suggested" => Ok(Triage::Suggested),
            "listed" => Ok(Triage::Listed),
            "record" => Ok(Triage::Record),
            other => anyhow::bail!("unknown triage value: {other}"),
        }
    }
}

/// Where a freshly extracted candidate lands: the inbox when it is mine or unassigned,
/// otherwise the record.
pub fn triage_for(c: &ResolvedCandidate) -> Triage {
    if c.assignee_is_self || (c.assignee_person_id.is_none() && c.assignee_raw.is_none()) {
        Triage::Suggested
    } else {
        Triage::Record
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(is_self: bool, person: Option<&str>, raw: Option<&str>) -> ResolvedCandidate {
        ResolvedCandidate {
            description: "x".into(),
            assignee_person_id: person.map(String::from),
            assignee_is_self: is_self,
            assignee_raw: raw.map(String::from),
            due_hint: None,
            due_date: None,
            ..Default::default()
        }
    }

    #[test]
    fn mine_is_suggested() {
        assert_eq!(triage_for(&cand(true, None, None)), Triage::Suggested);
    }

    #[test]
    fn unassigned_is_suggested() {
        assert_eq!(triage_for(&cand(false, None, None)), Triage::Suggested);
    }

    #[test]
    fn rostered_person_is_record() {
        assert_eq!(triage_for(&cand(false, Some("p-1"), None)), Triage::Record);
    }

    #[test]
    fn raw_name_is_record() {
        assert_eq!(triage_for(&cand(false, None, Some("Sam"))), Triage::Record);
    }

    #[test]
    fn round_trips_strings() {
        for t in [Triage::Suggested, Triage::Listed, Triage::Record] {
            assert_eq!(t.as_str().parse::<Triage>().unwrap(), t);
        }
        assert!("bogus".parse::<Triage>().is_err());
    }
}
