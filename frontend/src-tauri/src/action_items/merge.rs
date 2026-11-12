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

use super::diff::{token_similarity, ResolvedCandidate, FUZZY_MATCH_THRESHOLD};
use super::extractor::ALREADY_TRACKED_CAP;

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
    /// This meeting's own rows the model said it re-mentioned: they count as matched.
    pub kept_ids: HashSet<String>,
}

/// Route each candidate. `same_as` is 1-based into `tracked` as rendered (the prompt
/// shows at most [`ALREADY_TRACKED_CAP`] entries, so numbers past that are invalid).
/// A target in another meeting (or standalone) gets a source attached; a target in this
/// meeting is kept and the candidate dropped; an invalid number is treated as absent. With
/// no valid `same_as`, the best similarity at or above [`FUZZY_MATCH_THRESHOLD`] against
/// other meetings' items attaches. At most one attach per item: the first wins and later
/// candidates for the same item are dropped.
pub fn route_candidates(
    meeting_id: &str,
    candidates: Vec<ResolvedCandidate>,
    tracked: &[TrackedItem],
) -> Routed {
    let shown = &tracked[..tracked.len().min(ALREADY_TRACKED_CAP)];
    let mut routed = Routed::default();
    let mut attached: HashSet<&str> = HashSet::new();

    for cand in candidates {
        let target = cand
            .same_as
            .and_then(|n| (n as usize).checked_sub(1))
            .and_then(|i| shown.get(i))
            .or_else(|| {
                shown
                    .iter()
                    .filter(|t| t.meeting_id.as_deref() != Some(meeting_id))
                    .map(|t| (t, token_similarity(&cand.description, &t.description)))
                    .filter(|(_, sim)| *sim >= FUZZY_MATCH_THRESHOLD)
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(t, _)| t)
            });
        match target {
            None => routed.for_diff.push(cand),
            Some(t) if t.meeting_id.as_deref() == Some(meeting_id) => {
                routed.kept_ids.insert(t.id.clone());
            }
            Some(t) => {
                if attached.insert(t.id.as_str()) {
                    routed.attaches.push(SourceAttach {
                        item_id: t.id.clone(),
                        context: cand.context.clone().filter(|c| !c.trim().is_empty()),
                        phrasing: cand.description,
                    });
                }
            }
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

    #[test]
    fn same_as_other_meeting_attaches() {
        let t = [tracked("a", Some("m-other"), "Renew the vendor contract")];
        let r = route_candidates(
            "m-1",
            vec![cand("Sort out the vendor paperwork", Some(1))],
            &t,
        );
        assert!(r.for_diff.is_empty() && r.kept_ids.is_empty());
        assert_eq!(r.attaches.len(), 1);
        assert_eq!(r.attaches[0].item_id, "a");
        assert_eq!(r.attaches[0].phrasing, "Sort out the vendor paperwork");
        assert_eq!(r.attaches[0].context.as_deref(), Some("Priya raised it"));
        // A standalone to-do is a valid target too.
        let t = [tracked("s", None, "Renew the vendor contract")];
        let r = route_candidates("m-1", vec![cand("Do the paperwork", Some(1))], &t);
        assert_eq!(r.attaches[0].item_id, "s");
    }

    #[test]
    fn same_as_this_meeting_keeps_target() {
        let t = [tracked("own", Some("m-1"), "Book the venue")];
        let r = route_candidates("m-1", vec![cand("Reserve the venue", Some(1))], &t);
        assert!(r.for_diff.is_empty() && r.attaches.is_empty());
        assert!(r.kept_ids.contains("own"));
    }

    #[test]
    fn same_as_out_of_range_falls_through() {
        let t = [tracked("a", Some("m-other"), "Renew the vendor contract")];
        for n in [0, 2, 99] {
            let r = route_candidates("m-1", vec![cand("Plan the offsite agenda", Some(n))], &t);
            assert_eq!(r.for_diff.len(), 1, "same_as {n}");
            assert!(r.attaches.is_empty() && r.kept_ids.is_empty());
        }
        // Invalid number, but a strong textual match to another meeting still attaches.
        let r = route_candidates("m-1", vec![cand("Renew the vendor contract", Some(0))], &t);
        assert_eq!(r.attaches.len(), 1);
    }

    #[test]
    fn jaccard_backstop_only_against_other_meetings() {
        let t = [
            tracked("own", Some("m-1"), "Send the budget summary to finance"),
            tracked("far", Some("m-2"), "Send the budget summary to finance"),
        ];
        let r = route_candidates(
            "m-1",
            vec![cand("Send the budget summary to finance", None)],
            &t,
        );
        assert_eq!(r.attaches.len(), 1);
        assert_eq!(r.attaches[0].item_id, "far");
        // Only this meeting's own row matches: no attach, the diff handles it.
        let r = route_candidates(
            "m-1",
            vec![cand("Send the budget summary to finance", None)],
            &t[..1],
        );
        assert!(r.attaches.is_empty() && r.kept_ids.is_empty());
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
        );
        assert_eq!(r.attaches.len(), 1);
        assert_eq!(r.attaches[0].phrasing, "Renew the vendor contract");
        assert!(r.for_diff.is_empty());
    }
}
