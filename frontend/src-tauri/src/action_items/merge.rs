//! Cross-meeting merge routing (specs/0088 W2), pure logic.
//!
//! Extraction now shows the model a numbered TRACKED list and asks it to flag candidates
//! that are the same underlying task (`same_as: n`). This module decides what to do with
//! each candidate: attach a source to an existing item in ANOTHER meeting (or a
//! standalone to-do), keep this meeting's own row untouched, or hand the candidate to the
//! ordinary diff. A token-similarity backstop catches a model that forgot `same_as`, but
//! only against other meetings' items: within one meeting the diff already matches.
//! Persisting the routing is the caller's job.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::diff::{
    content_key, token_jaccard, token_similarity, ResolvedCandidate, FUZZY_MATCH_THRESHOLD,
};
use super::extractor::ALREADY_TRACKED_CAP;
use crate::database::repositories::action_item::ActionItem;

/// One entry of the TRACKED list shown to the model. `description` is already
/// `plain_description`'d: person tokens never reach the prompt or the similarity check.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackedItem {
    pub id: String,
    pub meeting_id: Option<String>,
    pub description: String,
}

/// A source to attach to an existing item.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceAttach {
    pub item_id: String,
    pub context: Option<String>,
    pub phrasing: String,
}

#[derive(Debug, Default)]
pub struct Routed {
    /// Candidates that go through the ordinary diff.
    pub for_diff: Vec<ResolvedCandidate>,
    pub attaches: Vec<SourceAttach>,
    /// `(this meeting's row id, candidate)`: the model said the candidate IS that row. The
    /// diff treats each pair as a forced match under its ordinary pristine/protected rules.
    pub kept: Vec<(String, ResolvedCandidate)>,
}

/// Model output is untrusted: a malformed `same_as` becomes `None` instead of failing the
/// whole reply. Accepts a non-negative integer, an integer-valued float, or a digit string.
pub(crate) fn lenient_same_as<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Number(n)) => n.as_u64().and_then(|u| u32::try_from(u).ok()).or_else(|| {
            n.as_f64()
                .filter(|f| f.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(f))
                .map(|f| f as u32)
        }),
        Some(Value::String(t)) => t.trim().parse::<u32>().ok(),
        _ => None,
    })
}

/// `context` is a string or nothing; any other JSON type becomes `None`.
pub(crate) fn lenient_context<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::String(t)) => Some(t),
        _ => None,
    })
}

fn matches_own_row(cand: &ResolvedCandidate, own: &[ActionItem]) -> bool {
    let key = content_key(&cand.description);
    own.iter().any(|row| {
        row.content_key == key
            || token_similarity(&cand.description, &row.description) >= FUZZY_MATCH_THRESHOLD
    })
}

/// Route each candidate. `same_as` is 1-based into `tracked` as rendered (the prompt
/// shows at most [`ALREADY_TRACKED_CAP`] entries, so numbers past that are invalid).
/// A target in another meeting (or standalone) gets a source attached; a target in this
/// meeting becomes a forced match carrying the candidate; an invalid number is treated as
/// absent. With no valid `same_as`, and only when none of this meeting's `own` rows
/// matches the candidate (those belong to the diff), the best plain Jaccard at or above
/// [`FUZZY_MATCH_THRESHOLD`] against other meetings' items attaches. At most one attach or
/// forced match per item: a later candidate for the same item goes to the diff.
pub fn route_candidates(
    meeting_id: &str,
    candidates: Vec<ResolvedCandidate>,
    tracked: &[TrackedItem],
    own: &[ActionItem],
) -> Routed {
    let shown = &tracked[..tracked.len().min(ALREADY_TRACKED_CAP)];
    let mut routed = Routed::default();
    let mut used: HashSet<&str> = HashSet::new();

    for cand in candidates {
        let target = cand
            .same_as
            .and_then(|n| (n as usize).checked_sub(1))
            .and_then(|i| shown.get(i))
            .or_else(|| {
                if matches_own_row(&cand, own) {
                    return None;
                }
                shown
                    .iter()
                    .filter(|t| t.meeting_id.as_deref() != Some(meeting_id))
                    .map(|t| (t, token_jaccard(&cand.description, &t.description)))
                    .filter(|(_, sim)| *sim >= FUZZY_MATCH_THRESHOLD)
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(t, _)| t)
            });
        match target {
            Some(t) if used.insert(t.id.as_str()) => {
                if t.meeting_id.as_deref() == Some(meeting_id) {
                    routed.kept.push((t.id.clone(), cand));
                } else {
                    routed.attaches.push(SourceAttach {
                        item_id: t.id.clone(),
                        context: cand.context.clone().filter(|c| !c.trim().is_empty()),
                        phrasing: cand.description,
                    });
                }
            }
            _ => routed.for_diff.push(cand),
        }
    }
    routed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracked(id: &str, meeting: Option<&str>, text: &str) -> TrackedItem {
        TrackedItem {
            id: id.into(),
            meeting_id: meeting.map(String::from),
            description: text.into(),
        }
    }

    fn cand(text: &str, same_as: Option<u32>) -> ResolvedCandidate {
        ResolvedCandidate {
            description: text.into(),
            same_as,
            context: Some("Priya raised it".into()),
            ..Default::default()
        }
    }

    fn own_row(text: &str) -> ActionItem {
        ActionItem {
            id: "own".into(),
            meeting_id: Some("m-1".into()),
            description: text.into(),
            assignee_person_id: None,
            assignee_is_self: false,
            assignee_raw: None,
            due_hint: None,
            due_date: None,
            status: "open".into(),
            source: "extracted".into(),
            user_edited: false,
            content_key: content_key(text),
            created_at: "2026-07-01T00:00:00Z".into(),
            updated_at: "2026-07-01T00:00:00Z".into(),
            completed_at: None,
            sort_order: None,
            triage: "suggested".into(),
            adopted_at: None,
            notes_markdown: None,
        }
    }

    #[test]
    fn same_as_other_meeting_attaches() {
        let t = [tracked("a", Some("m-other"), "Renew the vendor contract")];
        let r = route_candidates(
            "m-1",
            vec![cand("Sort out the vendor paperwork", Some(1))],
            &t,
            &[],
        );
        assert!(r.for_diff.is_empty() && r.kept.is_empty());
        assert_eq!(r.attaches.len(), 1);
        assert_eq!(r.attaches[0].item_id, "a");
        assert_eq!(r.attaches[0].phrasing, "Sort out the vendor paperwork");
        assert_eq!(r.attaches[0].context.as_deref(), Some("Priya raised it"));
        // A standalone to-do is a valid target too.
        let t = [tracked("s", None, "Renew the vendor contract")];
        let r = route_candidates("m-1", vec![cand("Do the paperwork", Some(1))], &t, &[]);
        assert_eq!(r.attaches[0].item_id, "s");
    }

    #[test]
    fn same_as_this_meeting_forces_a_match_carrying_the_candidate() {
        let t = [tracked("own", Some("m-1"), "Book the venue")];
        let r = route_candidates("m-1", vec![cand("Reserve the venue", Some(1))], &t, &[]);
        assert!(r.for_diff.is_empty() && r.attaches.is_empty());
        assert_eq!(r.kept.len(), 1);
        assert_eq!(r.kept[0].0, "own");
        assert_eq!(r.kept[0].1.description, "Reserve the venue");
    }

    #[test]
    fn same_as_out_of_range_falls_through() {
        let t = [tracked("a", Some("m-other"), "Renew the vendor contract")];
        for n in [0, 2, 99] {
            let r = route_candidates(
                "m-1",
                vec![cand("Plan the offsite agenda", Some(n))],
                &t,
                &[],
            );
            assert_eq!(r.for_diff.len(), 1, "same_as {n}");
            assert!(r.attaches.is_empty() && r.kept.is_empty());
        }
        // Invalid number, but a strong textual match to another meeting still attaches.
        let r = route_candidates(
            "m-1",
            vec![cand("Renew the vendor contract", Some(0))],
            &t,
            &[],
        );
        assert_eq!(r.attaches.len(), 1);
    }

    #[test]
    fn jaccard_backstop_only_against_other_meetings() {
        let text = "Send the budget summary to finance";
        let t = [
            tracked("own", Some("m-1"), text),
            tracked("far", Some("m-2"), text),
        ];
        // No own rows loaded: the other meeting's item attaches.
        let r = route_candidates("m-1", vec![cand(text, None)], &t, &[]);
        assert_eq!(r.attaches.len(), 1);
        assert_eq!(r.attaches[0].item_id, "far");
        // This meeting's own row matches too: the diff owns the candidate, no attach.
        let r = route_candidates("m-1", vec![cand(text, None)], &t, &[own_row(text)]);
        assert!(r.attaches.is_empty() && r.kept.is_empty());
        assert_eq!(r.for_diff.len(), 1);
        // Only this meeting's row is tracked: nothing to attach to.
        let r = route_candidates("m-1", vec![cand(text, None)], &t[..1], &[]);
        assert!(r.attaches.is_empty());
        assert_eq!(r.for_diff.len(), 1);
    }

    #[test]
    fn backstop_does_not_treat_a_short_title_as_a_subset_match() {
        let t = [tracked("a", Some("m-2"), "Call Sam")];
        let r = route_candidates(
            "m-1",
            vec![cand("Call Sam about the invoice dispute", None)],
            &t,
            &[],
        );
        assert!(r.attaches.is_empty());
        assert_eq!(r.for_diff.len(), 1);
    }

    #[test]
    fn one_attach_per_item() {
        let t = [tracked("a", Some("m-other"), "Renew the vendor contract")];
        let r = route_candidates(
            "m-1",
            vec![
                cand("Renew the vendor contract", Some(1)),
                cand("Chase the vendor renewal", Some(1)),
            ],
            &t,
            &[],
        );
        assert_eq!(r.attaches.len(), 1);
        assert_eq!(r.attaches[0].phrasing, "Renew the vendor contract");
        assert_eq!(r.for_diff.len(), 1);
        assert_eq!(r.for_diff[0].description, "Chase the vendor renewal");
    }
}
