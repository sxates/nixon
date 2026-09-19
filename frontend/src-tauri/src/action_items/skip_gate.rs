//! Whether background action-item extraction can be skipped (specs/0053 W3).
//!
//! The Auto outline pass already read the whole reduced meeting, so
//! `has_commitments` is free. Skipping saves an LLM call on meetings that have
//! nothing to extract.
//!
//! A false negative would cost the user their action items, so the gate is
//! deliberately conservative and the skip is never silent: the caller records a
//! skip outcome in the specs/0052 activity registry, and writes NO ledger row,
//! so a manual "Scan again" always re-runs for real.

use crate::database::repositories::summary_outline::StoredOutline;
use crate::summary::outline::{Outline, SectionRole, AUTO_TEMPLATE_ID};

/// True only when every condition holds. Applies to the BACKGROUND spawn only —
/// the manual "Scan again" command is never gated.
pub fn should_skip_background_extraction(
    template_id: &str,
    stored: Option<&StoredOutline>,
    has_user_notes: bool,
) -> bool {
    // 4. Notes are a second extraction source; a commitment can live only there.
    if has_user_notes {
        return false;
    }
    // 1. Only Auto produces the signal.
    if template_id != AUTO_TEMPLATE_ID {
        return false;
    }
    let Some(stored) = stored else {
        return false;
    };
    // 2. The outline says there is nothing to extract.
    if stored.has_commitments {
        return false;
    }
    // 3. Belt and braces: flag and sections must agree.
    match serde_json::from_str::<Outline>(&stored.outline_json) {
        Ok(outline) => !outline
            .sections
            .iter()
            .any(|s| s.role == Some(SectionRole::Commitments)),
        // An unreadable outline is not evidence of absence.
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(has_commitments: bool, with_commitments_section: bool) -> StoredOutline {
        let mut outline = crate::summary::outline::fallback_outline();
        outline.has_commitments = has_commitments;
        if !with_commitments_section {
            outline
                .sections
                .retain(|s| s.role != Some(SectionRole::Commitments));
        }
        StoredOutline {
            outline_json: serde_json::to_string(&outline).unwrap(),
            has_commitments,
            derived_at: "now".to_string(),
        }
    }

    #[test]
    fn skips_a_commitment_free_auto_meeting_with_no_notes() {
        assert!(should_skip_background_extraction(
            AUTO_TEMPLATE_ID,
            Some(&stored(false, false)),
            false
        ));
    }

    /// The condition that protects the specs/0034 recall mitigation: notes are
    /// a SECOND source, so a commitment can exist only there.
    #[test]
    fn never_skips_when_the_meeting_has_notes() {
        assert!(!should_skip_background_extraction(
            AUTO_TEMPLATE_ID,
            Some(&stored(false, false)),
            true
        ));
    }

    #[test]
    fn never_skips_when_the_outline_reports_commitments() {
        assert!(!should_skip_background_extraction(
            AUTO_TEMPLATE_ID,
            Some(&stored(true, true)),
            false
        ));
    }

    /// Isolates condition 2 from condition 3: `has_commitments == true` with the
    /// (contradictory) commitments section already absent must still refuse to skip
    /// on the flag alone, without relying on the section check to also catch it —
    /// unlike `never_skips_when_the_outline_reports_commitments` above, where both
    /// conditions agree and either one alone would pass.
    #[test]
    fn never_skips_on_the_commitments_flag_alone_even_if_the_section_is_missing() {
        assert!(!should_skip_background_extraction(
            AUTO_TEMPLATE_ID,
            Some(&stored(true, false)),
            false
        ));
    }

    /// A fixed template produces no signal, so it can never justify a skip.
    #[test]
    fn never_skips_for_a_fixed_template() {
        assert!(!should_skip_background_extraction(
            "standard_meeting",
            Some(&stored(false, false)),
            false
        ));
    }

    #[test]
    fn never_skips_when_no_outline_is_stored() {
        assert!(!should_skip_background_extraction(
            AUTO_TEMPLATE_ID,
            None,
            false
        ));
    }

    /// Flag and sections disagreeing means we do not trust the flag.
    #[test]
    fn never_skips_when_a_commitments_section_is_present_despite_the_flag() {
        assert!(!should_skip_background_extraction(
            AUTO_TEMPLATE_ID,
            Some(&stored(false, true)),
            false
        ));
    }

    /// An unreadable outline is not evidence of absence.
    #[test]
    fn never_skips_on_a_corrupt_stored_outline() {
        let corrupt = StoredOutline {
            outline_json: "{not json".to_string(),
            has_commitments: false,
            derived_at: "now".to_string(),
        };
        assert!(!should_skip_background_extraction(
            AUTO_TEMPLATE_ID,
            Some(&corrupt),
            false
        ));
    }
}
