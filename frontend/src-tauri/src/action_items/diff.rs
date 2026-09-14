//! The protected/pristine action-item diff engine (specs/0034).
//!
//! Summaries get regenerated constantly (template change, notes edit, language change,
//! "Transcribe now" rewrite), so re-extraction must NEVER wipe-and-reload. This module
//! computes a pure, unit-testable diff between the freshly extracted candidates and the
//! rows already in `action_items`:
//!
//! - **Protected** rows (`source = 'manual'` OR `user_edited = 1` OR `status != 'open'`)
//!   are invisible walls: a candidate matching one is DROPPED (no duplicate, no update);
//!   an unmatched protected row is kept forever.
//! - **Pristine** rows (extracted + open + untouched) are machine-owned: a matching
//!   candidate updates them in place (id/`created_at`/`status` preserved); an unmatched
//!   pristine row is deleted — exactly how regeneration already treats the summary
//!   markdown itself.
//!
//! Identity is a normalized-content fingerprint (`content_key`) with a fuzzy fallback —
//! max(token-set Jaccard, overlap coefficient) ≥ [`FUZZY_MATCH_THRESHOLD`] — to absorb
//! LLM rephrasings, both symmetric ("send the deck to Alice" ↔ "send deck to Alice") and
//! subset-style ("send the presentation deck to participants" → "send the deck", the
//! 2026-07 template-switch dogfood incident). The per-meeting extraction ledger's
//! `summary_fingerprint` ([`extraction_fingerprint`]) makes re-running against an
//! unchanged summary a no-op.
//!
//! Same pattern as 0029's speaker overrides ("machine may only rewrite machine-owned,
//! pristine derivatives"); promote to an ADR when a third feature needs it.

use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;

use crate::database::repositories::action_item::ActionItem;

/// Minimum fuzzy similarity ([`similarity`]: max of token-set Jaccard and the overlap
/// coefficient, over normalized words) for two descriptions to be considered the same
/// task. Tuning note (specs/0034 Risks): too low merges distinct tasks, too high
/// duplicates rephrased ones; boundary behavior is pinned by tests with fixture pairs
/// from real regenerations (the 2026-07 template-switch incident).
pub const FUZZY_MATCH_THRESHOLD: f64 = 0.7;

/// The overlap coefficient only applies when the SMALLER token set has at least this many
/// tokens. A single-token description ("the", "deck") is a subset of nearly everything —
/// overlap 1.0 against any superset — so single-token comparisons fall back to plain
/// Jaccard. Two tokens is the smallest set that still expresses a task (verb + object,
/// "send deck"), which is exactly the subset-style compression a template switch produces.
const MIN_OVERLAP_TOKENS: usize = 2;

/// Stable FNV-1a fingerprint of a text — same scheme as `summary/service.rs`'s
/// `stable_text_fingerprint` (deliberately duplicated per specs/0034: "extract the helper
/// into a shared location or duplicate the 10 lines"). Not cryptographic; collision odds
/// are irrelevant at per-meeting scale and the `:len` suffix guards degenerate cases.
pub fn stable_text_fingerprint(text: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}:{}", hash, text.len())
}

/// Normalizes a description for identity purposes: NFC, lowercase, punctuation stripped
/// (non-alphanumeric → space, so "deck/slides" still splits into two tokens), whitespace
/// collapsed. This is the input to both [`content_key`] and the Jaccard token sets.
pub fn normalize_description(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    let mapped: String = nfc
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The diff identity of a description: fingerprint of its normalized form. Stored in
/// `action_items.content_key` and recomputed whenever the description changes.
pub fn content_key(description: &str) -> String {
    stable_text_fingerprint(&normalize_description(description))
}

/// Fingerprint of the extraction INPUT (summary markdown + user notes), stored in the
/// `action_item_extractions` ledger. When it matches the current input, re-extraction is
/// skipped entirely (idempotence — acceptance #4).
pub fn extraction_fingerprint(summary_markdown: &str, user_notes: Option<&str>) -> String {
    match user_notes {
        Some(notes) if !notes.trim().is_empty() => {
            stable_text_fingerprint(&format!("{summary_markdown}\n---USER-NOTES---\n{notes}"))
        }
        _ => stable_text_fingerprint(summary_markdown),
    }
}

/// A candidate AFTER owner resolution (`extractor::resolve_candidates`): assignee strings
/// have been turned into a `people` FK, the self flag, or a raw display fallback. This is
/// the write-shape the diff and the repository share.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCandidate {
    pub description: String,
    pub assignee_person_id: Option<String>,
    pub assignee_is_self: bool,
    pub assignee_raw: Option<String>,
    pub due_hint: Option<String>,
    /// Structured ISO-8601 date (YYYY-MM-DD), best-effort filled from `due_hint` when it
    /// is unambiguously a date (specs/0038 WS1.a). `None` on the common path — the user
    /// sets this via the row's date control, which goes through `update_content`.
    pub due_date: Option<String>,
}

/// An unmatched candidate → new `action_items` row (`source = 'extracted'`).
#[derive(Debug, Clone)]
pub struct CandidateInsert {
    pub candidate: ResolvedCandidate,
    pub content_key: String,
}

/// A candidate matched to a PRISTINE row → in-place rewrite of that row's content
/// (id, `created_at`, `status` preserved).
#[derive(Debug, Clone)]
pub struct PristineUpdate {
    /// Id of the existing pristine row being rewritten.
    pub id: String,
    pub candidate: ResolvedCandidate,
    pub content_key: String,
}

/// The full apply-plan produced by [`compute_diff`]; persisted atomically by
/// `ActionItemsRepository::replace_extracted` (one transaction incl. the ledger upsert).
#[derive(Debug, Default)]
pub struct ExtractionDiff {
    pub inserts: Vec<CandidateInsert>,
    pub updates: Vec<PristineUpdate>,
    /// Pristine rows the new summary no longer supports — machine-owned, so deleted.
    pub delete_ids: Vec<String>,
}

impl ExtractionDiff {
    pub fn is_empty(&self) -> bool {
        self.inserts.is_empty() && self.updates.is_empty() && self.delete_ids.is_empty()
    }
}

/// A row the machine may NOT touch: manual, user-edited, or no longer open (completed /
/// dismissed). Everything else is pristine (machine-owned).
///
/// Since the 2026-07 incident fix, EVERY user status change also sets `user_edited = 1`
/// (`ActionItemsRepository::set_status`), so the `status != 'open'` arm is belt-and-
/// braces for status — it still independently protects rows completed/dismissed before
/// that change shipped.
pub fn is_protected(item: &ActionItem) -> bool {
    item.source == "manual" || item.user_edited || item.status != "open"
}

/// Token set of a normalized description, for fuzzy matching.
fn token_set(normalized: &str) -> HashSet<String> {
    normalized.split_whitespace().map(str::to_string).collect()
}

/// Fuzzy similarity between two normalized token sets, the max of:
/// - **Jaccard** (|∩| / |∪|) — symmetric rephrasing distance ("send the deck to alice" ↔
///   "send deck to alice");
/// - the **overlap coefficient** (|∩| / min(|A|, |B|)) — subset-style rephrasings, where
///   regenerating with a different template compresses the description: the 2026-07
///   dogfood pairs "send the presentation deck to participants" → "send the deck"
///   (Jaccard 0.50, overlap 1.0) and "order pizza for the meeting" → "order the pizza"
///   (Jaccard 0.60, overlap 1.0) must match the user's protected rows, not duplicate them.
///
/// Degenerate-subset guard: the overlap coefficient only applies when the smaller set has
/// ≥ [`MIN_OVERLAP_TOKENS`] tokens (see its doc) — otherwise plain Jaccard. 0.0 when
/// either set is empty (an empty description must never fuzzy-match anything — exact
/// `content_key` equality still applies).
fn similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = (a.len() + b.len()) as f64 - intersection;
    let jaccard = intersection / union;

    let min_len = a.len().min(b.len());
    if min_len < MIN_OVERLAP_TOKENS {
        return jaccard;
    }
    let overlap = intersection / min_len as f64;
    jaccard.max(overlap)
}

/// Computes the regeneration-safe diff between the rows already stored for a meeting and
/// the freshly extracted candidates.
///
/// Matching per candidate, in order (each existing row absorbs at most one candidate):
/// 1. exact `content_key` equality against ALL existing rows — protected first;
/// 2. fuzzy [`similarity`] (max of token-set Jaccard and overlap coefficient) ≥
///    [`FUZZY_MATCH_THRESHOLD`] against still-unmatched rows (best similarity wins; a
///    tie prefers the protected row, the safer absorption).
///
/// Apply rules: candidate ↔ protected → candidate dropped; candidate ↔ pristine → update
/// in place; unmatched candidate → insert; unmatched pristine → delete; unmatched
/// protected → kept, untouched, forever (it simply never appears in the plan).
///
/// No-duplicates guarantees (backing the DB's partial unique index on
/// `(meeting_id, content_key) WHERE source = 'extracted'`):
/// - candidates are DEDUPED by `content_key` up front (an LLM that ignores the "merge
///   repeated mentions" instruction can't produce two identical inserts);
/// - a candidate whose exact key sits on an already-matched PROTECTED row (that row
///   absorbed an earlier near-duplicate) is dropped, not inserted — the content is
///   already represented by a row the user owns. Matched PRISTINE rows don't block:
///   they are being rewritten to their own candidate's key, so their old key is vacated.
pub fn compute_diff(existing: &[ActionItem], candidates: &[ResolvedCandidate]) -> ExtractionDiff {
    struct Entry<'a> {
        item: &'a ActionItem,
        protected: bool,
        tokens: HashSet<String>,
        matched: bool,
    }

    let mut entries: Vec<Entry> = existing
        .iter()
        .map(|item| Entry {
            item,
            protected: is_protected(item),
            tokens: token_set(&normalize_description(&item.description)),
            matched: false,
        })
        .collect();

    let mut diff = ExtractionDiff::default();
    let mut seen_keys: HashSet<String> = HashSet::new();

    for candidate in candidates {
        let key = content_key(&candidate.description);
        // Dedup within the run: a repeated candidate (same normalized content) is the
        // same task — the first occurrence already produced its insert/update/drop.
        if !seen_keys.insert(key.clone()) {
            continue;
        }
        let tokens = token_set(&normalize_description(&candidate.description));

        // (a) exact content_key — protected rows first, so a user-touched row absorbs the
        // candidate before an identical pristine row could claim it.
        let exact = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.matched && e.item.content_key == key)
            .max_by_key(|(_, e)| e.protected)
            .map(|(i, _)| i);

        // A matched PROTECTED row already carries this exact key (it absorbed an earlier
        // near-duplicate) → this candidate duplicates content the user owns; drop it.
        // Inserting would both duplicate the UI and violate the partial unique index.
        if exact.is_none()
            && entries
                .iter()
                .any(|e| e.matched && e.protected && e.item.content_key == key)
        {
            continue;
        }

        // (b) fuzzy fallback: best similarity ≥ threshold among still-unmatched rows;
        // ties prefer protected (dropping the candidate is always safe; a wrong pristine
        // update is not).
        let matched_idx = exact.or_else(|| {
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.matched)
                .map(|(i, e)| (i, similarity(&tokens, &e.tokens), e.protected))
                .filter(|(_, sim, _)| *sim >= FUZZY_MATCH_THRESHOLD)
                .max_by(|(_, sim_a, prot_a), (_, sim_b, prot_b)| {
                    sim_a
                        .partial_cmp(sim_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(prot_a.cmp(prot_b))
                })
                .map(|(i, _, _)| i)
        });

        match matched_idx {
            Some(i) => {
                entries[i].matched = true;
                if !entries[i].protected {
                    diff.updates.push(PristineUpdate {
                        id: entries[i].item.id.clone(),
                        candidate: candidate.clone(),
                        content_key: key,
                    });
                }
                // Protected match → drop the candidate: the row stays exactly as the
                // user left it and no duplicate is created.
            }
            None => diff.inserts.push(CandidateInsert {
                candidate: candidate.clone(),
                content_key: key,
            }),
        }
    }

    // Unmatched PRISTINE rows are machine-owned content the new summary no longer
    // supports → delete. Unmatched protected rows are kept, untouched.
    diff.delete_ids = entries
        .iter()
        .filter(|e| !e.matched && !e.protected)
        .map(|e| e.item.id.clone())
        .collect();

    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(description: &str) -> ResolvedCandidate {
        ResolvedCandidate {
            description: description.to_string(),
            assignee_person_id: None,
            assignee_is_self: false,
            assignee_raw: None,
            due_hint: None,
            due_date: None,
        }
    }

    /// An existing row as the repository would return it. `content_key` is computed from
    /// the description, mirroring how rows are written.
    fn row(
        id: &str,
        description: &str,
        status: &str,
        source: &str,
        user_edited: bool,
    ) -> ActionItem {
        ActionItem {
            id: id.to_string(),
            meeting_id: Some("m1".to_string()),
            description: description.to_string(),
            assignee_person_id: None,
            assignee_is_self: false,
            assignee_raw: None,
            due_hint: None,
            due_date: None,
            status: status.to_string(),
            source: source.to_string(),
            user_edited,
            content_key: content_key(description),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            updated_at: "2026-07-01T00:00:00Z".to_string(),
            completed_at: None,
            sort_order: None,
        }
    }

    fn pristine(id: &str, description: &str) -> ActionItem {
        row(id, description, "open", "extracted", false)
    }

    // ---- normalization + content_key ------------------------------------------------

    #[test]
    fn normalize_strips_punctuation_case_and_whitespace() {
        assert_eq!(
            normalize_description("  Send the DECK, to Alice!!  by   Friday. "),
            "send the deck to alice by friday"
        );
    }

    #[test]
    fn normalize_applies_nfc() {
        // "é" as combining sequence (e + U+0301) vs precomposed U+00E9 → same normal form.
        let combining = "re\u{0301}sume the sync";
        let precomposed = "r\u{e9}sume the sync";
        assert_eq!(
            normalize_description(combining),
            normalize_description(precomposed)
        );
        assert_eq!(content_key(combining), content_key(precomposed));
    }

    #[test]
    fn content_key_is_stable_and_normalization_insensitive() {
        assert_eq!(
            content_key("Send the deck to Alice"),
            content_key("send   the deck, to ALICE!")
        );
        assert_ne!(content_key("send the deck"), content_key("book the room"));
    }

    // ---- ledger fingerprint (verification row 2) --------------------------------------

    #[test]
    fn extraction_fingerprint_changes_with_summary_or_notes() {
        let fp = extraction_fingerprint("## Summary\n- do x", Some("my notes"));
        // Identical input → identical fingerprint (the ledger skip condition).
        assert_eq!(
            fp,
            extraction_fingerprint("## Summary\n- do x", Some("my notes"))
        );
        // Any change to summary OR notes invalidates the ledger.
        assert_ne!(
            fp,
            extraction_fingerprint("## Summary\n- do y", Some("my notes"))
        );
        assert_ne!(
            fp,
            extraction_fingerprint("## Summary\n- do x", Some("other notes"))
        );
        assert_ne!(fp, extraction_fingerprint("## Summary\n- do x", None));
    }

    #[test]
    fn extraction_fingerprint_ignores_blank_notes() {
        assert_eq!(
            extraction_fingerprint("summary", None),
            extraction_fingerprint("summary", Some("   "))
        );
    }

    // ---- verification row 1: fresh insert --------------------------------------------

    #[test]
    fn fresh_meeting_inserts_all_candidates() {
        let candidates = vec![
            candidate("Send the deck to Alice"),
            candidate("Book the conference room"),
            candidate("Follow up with legal"),
        ];
        let diff = compute_diff(&[], &candidates);
        assert_eq!(diff.inserts.len(), 3);
        assert!(diff.updates.is_empty());
        assert!(diff.delete_ids.is_empty());
        assert_eq!(
            diff.inserts[0].content_key,
            content_key("Send the deck to Alice")
        );
    }

    // ---- identical re-run at the diff level: no-op ------------------------------------

    #[test]
    fn identical_candidates_update_pristine_rows_in_place_not_duplicate() {
        // Even without the ledger short-circuit, re-running the same candidates against
        // the rows they created must produce zero inserts and zero deletes.
        let existing = vec![pristine("ai-1", "Send the deck to Alice")];
        let diff = compute_diff(&existing, &[candidate("Send the deck to Alice")]);
        assert!(
            diff.inserts.is_empty(),
            "no duplicate on identical fingerprint"
        );
        assert!(diff.delete_ids.is_empty());
        assert_eq!(diff.updates.len(), 1);
        assert_eq!(diff.updates[0].id, "ai-1");
    }

    // ---- verification rows 3–5: protected rows absorb, zero duplicates ---------------

    #[test]
    fn completed_row_absorbs_exact_candidate_without_duplicate() {
        let existing = vec![row(
            "ai-a",
            "Send the deck to Alice",
            "completed",
            "extracted",
            false,
        )];
        let diff = compute_diff(&existing, &[candidate("Send the deck to Alice")]);
        assert!(
            diff.is_empty(),
            "protected row untouched, candidate dropped: {diff:?}"
        );
    }

    #[test]
    fn completed_row_absorbs_reworded_candidate_above_threshold() {
        // Verification row 3: reworded ≥ 0.7 similarity still lands on the completed row.
        let existing = vec![row(
            "ai-a",
            "send the quarterly deck to alice by friday",
            "completed",
            "extracted",
            false,
        )];
        // 6 of 8 tokens shared, union 8 → 6/8 = 0.75 ≥ 0.7.
        let diff = compute_diff(&existing, &[candidate("send the quarterly deck to alice")]);
        assert!(
            diff.is_empty(),
            "reworded candidate absorbed by protected row: {diff:?}"
        );
    }

    #[test]
    fn user_edited_row_absorbs_candidate_with_original_phrasing() {
        // Verification row 4: user edited B's text; the new summary still contains the
        // ORIGINAL phrasing. The edited row is protected — but does its content_key still
        // match? No (text changed) — so this exercises the Jaccard path when close, or a
        // plain insert-shield via similarity. Pin the safe behavior for a near-identical
        // edit ("please" added).
        let existing = vec![row(
            "ai-b",
            "send the deck to alice please",
            "open",
            "extracted",
            true, // user_edited → protected
        )];
        // Original phrasing: 5 shared tokens, union 6 → 0.833 ≥ 0.7 → absorbed.
        let diff = compute_diff(&existing, &[candidate("send the deck to alice")]);
        assert!(
            diff.is_empty(),
            "edited row absorbs its original phrasing: {diff:?}"
        );
    }

    #[test]
    fn dismissed_row_stays_dismissed_and_blocks_resurrection() {
        // Verification row 5: dismissing must be persistent — the candidate reappearing
        // in the new summary is dropped, not re-inserted.
        let existing = vec![row(
            "ai-c",
            "Follow up with legal",
            "dismissed",
            "extracted",
            false,
        )];
        let diff = compute_diff(&existing, &[candidate("Follow up with legal")]);
        assert!(
            diff.is_empty(),
            "dismissed row blocks resurrection: {diff:?}"
        );
    }

    // ---- verification row 6: pristine delete when omitted ----------------------------

    #[test]
    fn pristine_row_omitted_from_new_summary_is_deleted() {
        let existing = vec![pristine("ai-d", "Book the conference room")];
        let diff = compute_diff(&existing, &[candidate("Totally unrelated new task")]);
        assert_eq!(diff.delete_ids, vec!["ai-d".to_string()]);
        assert_eq!(diff.inserts.len(), 1);
        assert!(diff.updates.is_empty());
    }

    #[test]
    fn protected_row_omitted_from_new_summary_is_kept() {
        let existing = vec![row(
            "ai-p",
            "Book the conference room",
            "completed",
            "extracted",
            false,
        )];
        let diff = compute_diff(&existing, &[]);
        assert!(
            diff.is_empty(),
            "unmatched protected row kept forever: {diff:?}"
        );
    }

    // ---- verification row 7: pristine update in place --------------------------------

    #[test]
    fn pristine_row_rephrased_above_threshold_updates_in_place() {
        let existing = vec![pristine("ai-e", "send the deck to alice by friday")];
        let mut new = candidate("send deck to alice by friday"); // drop "the": ∩6/∪7 ≈ 0.857
        new.due_hint = Some("friday".to_string());
        let diff = compute_diff(&existing, &[new.clone()]);
        assert!(diff.inserts.is_empty());
        assert!(diff.delete_ids.is_empty());
        assert_eq!(diff.updates.len(), 1);
        assert_eq!(
            diff.updates[0].id, "ai-e",
            "same row id — created_at/status preserved by apply"
        );
        assert_eq!(diff.updates[0].candidate, new);
        assert_eq!(
            diff.updates[0].content_key,
            content_key("send deck to alice by friday")
        );
    }

    // ---- verification row 8: manual rows untouched ------------------------------------

    #[test]
    fn manual_row_is_never_updated_deleted_or_duplicated() {
        let existing = vec![row(
            "ai-m",
            "Water the office plants",
            "open",
            "manual",
            false,
        )];

        // Candidate matching the manual row → dropped.
        let diff = compute_diff(&existing, &[candidate("Water the office plants")]);
        assert!(
            diff.is_empty(),
            "manual row absorbs matching candidate: {diff:?}"
        );

        // Re-run omitting it entirely → still untouched (not deleted).
        let diff = compute_diff(&existing, &[candidate("Ship the release")]);
        assert!(diff.delete_ids.is_empty(), "manual row survives omission");
        assert_eq!(diff.inserts.len(), 1);
    }

    // ---- boundary behavior at the 0.7 threshold ---------------------------------------

    #[test]
    fn jaccard_exactly_at_threshold_matches() {
        // Existing: 8 tokens {a..h}. Candidate: {a..g, x, y} (9 tokens).
        // Intersection 7, union 10 → Jaccard exactly 0.7 → matches (≥ is inclusive).
        // (Overlap 7/8 = 0.875 agrees; the max can only strengthen a Jaccard match.)
        let existing = vec![pristine(
            "ai-x",
            "alpha bravo charlie delta echo foxtrot golf hotel",
        )];
        let diff = compute_diff(
            &existing,
            &[candidate(
                "alpha bravo charlie delta echo foxtrot golf xray yankee",
            )],
        );
        assert_eq!(diff.updates.len(), 1, "0.7 exactly is a match");
        assert_eq!(diff.updates[0].id, "ai-x");
        assert!(diff.inserts.is_empty());
        assert!(diff.delete_ids.is_empty());
    }

    #[test]
    fn both_metrics_below_threshold_does_not_match() {
        // Intersection 5 over two 8-token sets: Jaccard 5/11 ≈ 0.455, overlap 5/8 =
        // 0.625 — BOTH below 0.7 → treated as a different task: candidate inserted,
        // pristine row deleted. (∩ 6 would no longer suffice: overlap 6/8 = 0.75.)
        let existing = vec![pristine(
            "ai-x",
            "alpha bravo charlie delta echo foxtrot golf hotel",
        )];
        let diff = compute_diff(
            &existing,
            &[candidate(
                "alpha bravo charlie delta echo victor whiskey xray",
            )],
        );
        assert!(
            diff.updates.is_empty(),
            "0.625 overlap / 0.455 Jaccard must not match"
        );
        assert_eq!(diff.inserts.len(), 1);
        assert_eq!(diff.delete_ids, vec!["ai-x".to_string()]);
    }

    #[test]
    fn overlap_exactly_at_threshold_matches() {
        // Candidate 10 tokens, existing 12 tokens, intersection 7: overlap 7/10 = 0.7
        // exactly (≥ is inclusive) while Jaccard is only 7/15 ≈ 0.467 — pins that the
        // overlap coefficient alone can carry a match at the boundary.
        let existing = vec![pristine(
            "ai-x",
            "alpha bravo charlie delta echo foxtrot golf hotel india juliett kilo lima",
        )];
        let diff = compute_diff(
            &existing,
            &[candidate(
                "alpha bravo charlie delta echo foxtrot golf whiskey xray yankee",
            )],
        );
        assert_eq!(diff.updates.len(), 1, "overlap 0.7 exactly is a match");
        assert!(diff.inserts.is_empty());
        assert!(diff.delete_ids.is_empty());
    }

    #[test]
    fn overlap_just_below_threshold_does_not_match() {
        // 3-token candidate sharing 2 tokens: overlap 2/3 ≈ 0.667 < 0.7, Jaccard 2/6 ≈
        // 0.333 → a different task, even though most of the candidate is contained.
        let existing = vec![pristine("ai-x", "order pizza for the meeting")];
        let diff = compute_diff(&existing, &[candidate("bring the pizza")]);
        assert!(diff.updates.is_empty(), "overlap 0.667 must not match");
        assert_eq!(diff.inserts.len(), 1);
        assert_eq!(diff.delete_ids, vec!["ai-x".to_string()]);
    }

    // ---- overlap coefficient: subset rephrasings + the single-token guard --------------

    #[test]
    fn two_token_subset_matches_via_overlap() {
        // The smallest legitimate subset: a 2-token candidate fully contained in the
        // existing description (overlap 2/2 = 1.0, Jaccard 2/6 ≈ 0.333) → update in place.
        let existing = vec![pristine("ai-e", "send the quarterly deck to alice")];
        let diff = compute_diff(&existing, &[candidate("send deck")]);
        assert_eq!(
            diff.updates.len(),
            1,
            "min-size-2 subset matches via overlap"
        );
        assert_eq!(diff.updates[0].id, "ai-e");
        assert!(diff.inserts.is_empty());
        assert!(diff.delete_ids.is_empty());
    }

    #[test]
    fn single_token_subset_falls_back_to_jaccard_and_does_not_match() {
        // Pathological subset: the single token "the" is contained in almost every
        // description (overlap would be 1/1 = 1.0). The guard drops to Jaccard
        // (1/4 = 0.25) → no match, no false absorption by the protected row.
        let existing = vec![row(
            "ai-room",
            "set up the room",
            "completed",
            "extracted",
            false,
        )];
        let diff = compute_diff(&existing, &[candidate("the")]);
        assert!(diff.updates.is_empty());
        assert_eq!(
            diff.inserts.len(),
            1,
            "single-token candidate is its own (new) task"
        );
        assert!(diff.delete_ids.is_empty(), "protected row untouched");
    }

    // ---- 2026-07 dogfood incident: template-switch rephrasings pinned verbatim ---------
    //
    // Regenerating with a DIFFERENT template compressed every action item beyond the old
    // Jaccard-only matcher; the candidates failed to match the user's protected rows and
    // inserted as duplicates, doubling the list. These tests pin the real pairs.

    #[test]
    fn incident_deck_pair_absorbed_by_user_edited_row() {
        // "Send the presentation deck to participants" (user-edited) → "Send the deck":
        // Jaccard 3/6 = 0.50 (old matcher: duplicate), overlap 3/3 = 1.0 → absorbed.
        let existing = vec![row(
            "ai-deck",
            "Send the presentation deck to participants",
            "open",
            "extracted",
            true, // user_edited → protected
        )];
        let diff = compute_diff(&existing, &[candidate("Send the deck")]);
        assert!(
            diff.is_empty(),
            "deck pair must be absorbed, not duplicated: {diff:?}"
        );
    }

    #[test]
    fn incident_pizza_pair_absorbed_by_user_edited_row() {
        // "Order pizza for the meeting" (user-edited) → "Order the pizza":
        // Jaccard 3/5 = 0.60 (old matcher: duplicate), overlap 3/3 = 1.0 → absorbed.
        let existing = vec![row(
            "ai-pizza",
            "Order pizza for the meeting",
            "open",
            "extracted",
            true,
        )];
        let diff = compute_diff(&existing, &[candidate("Order the pizza")]);
        assert!(
            diff.is_empty(),
            "pizza pair must be absorbed, not duplicated: {diff:?}"
        );
    }

    #[test]
    fn incident_room_pair_is_below_token_threshold_by_design() {
        // "Set up the room" (completed) → "Look at the room": Jaccard 2/6 ≈ 0.33,
        // overlap 2/4 = 0.5 — INTENTIONALLY below threshold at the token level (only
        // "the room" is shared; a token matcher loose enough to equate set-up with
        // look-at would merge genuinely distinct tasks). This pair is covered by the
        // prompt layer instead: the protected row's description rides in the extraction
        // prompt's ALREADY TRACKED section, so the model suppresses the rephrasing
        // before the diff ever sees it (see extractor.rs + mod.rs::run_extraction).
        // At the diff level the candidate inserts; the protected row stays untouched.
        let existing = vec![row(
            "ai-room",
            "Set up the room",
            "completed",
            "extracted",
            false,
        )];
        let diff = compute_diff(&existing, &[candidate("Look at the room")]);
        assert_eq!(
            diff.inserts.len(),
            1,
            "token level alone cannot bridge this pair"
        );
        assert!(diff.updates.is_empty());
        assert!(diff.delete_ids.is_empty(), "completed row untouched");
    }

    // ---- matcher preferences -----------------------------------------------------------

    #[test]
    fn exact_match_prefers_protected_over_pristine() {
        // Same content on a completed row AND a pristine row: the protected row absorbs
        // the candidate; the pristine duplicate is then unmatched → deleted. Zero dupes.
        let existing = vec![
            pristine("ai-pristine", "Send the deck to Alice"),
            row(
                "ai-done",
                "Send the deck to Alice",
                "completed",
                "extracted",
                false,
            ),
        ];
        let diff = compute_diff(&existing, &[candidate("Send the deck to Alice")]);
        assert!(diff.inserts.is_empty());
        assert!(
            diff.updates.is_empty(),
            "protected row wins the exact match"
        );
        assert_eq!(diff.delete_ids, vec!["ai-pristine".to_string()]);
    }

    #[test]
    fn duplicate_candidates_are_deduped_to_one_insert() {
        // The LLM ignored "merge repeated mentions": two candidates with the same
        // normalized content must yield ONE insert (the unique-index invariant).
        let diff = compute_diff(
            &[],
            &[
                candidate("Send the deck to Alice"),
                candidate("send the deck, to ALICE!"), // same content_key
            ],
        );
        assert_eq!(diff.inserts.len(), 1, "duplicate candidate deduped");

        // Same for a duplicate that would otherwise double-update a pristine row's key.
        let existing = vec![pristine("ai-1", "Send the deck to Alice")];
        let diff = compute_diff(
            &existing,
            &[
                candidate("Send the deck to Alice"),
                candidate("send the deck to alice"),
            ],
        );
        assert_eq!(diff.updates.len(), 1);
        assert!(
            diff.inserts.is_empty(),
            "duplicate never becomes a second row"
        );
    }

    #[test]
    fn candidate_duplicating_a_matched_protected_row_is_dropped() {
        // A dismissed row absorbs a fuzzy near-duplicate first; a later candidate with
        // the row's EXACT key must be dropped, not inserted (it would collide with the
        // kept protected row under the (meeting_id, content_key) unique index).
        let existing = vec![row(
            "ai-p",
            "send the deck",
            "dismissed",
            "extracted",
            false,
        )];
        let diff = compute_diff(
            &existing,
            &[
                candidate("send the deck please"), // fuzzy (3/4 = 0.75) → absorbs ai-p
                candidate("send the deck"),        // exact key of the now-matched ai-p
            ],
        );
        assert!(
            diff.is_empty(),
            "duplicate of user-owned content dropped: {diff:?}"
        );
    }

    #[test]
    fn candidate_with_vacated_pristine_key_still_inserts() {
        // A pristine row is rewritten to a NEW key by its fuzzy match; a later candidate
        // carrying the row's ORIGINAL key is legitimate new content (the old key is
        // vacated) and must insert.
        let existing = vec![pristine("ai-1", "send the deck")];
        let diff = compute_diff(
            &existing,
            &[
                candidate("send the deck please"), // fuzzy → rewrites ai-1's key
                candidate("send the deck"),        // ai-1's original key, now vacated
            ],
        );
        assert_eq!(diff.updates.len(), 1);
        assert_eq!(diff.inserts.len(), 1, "vacated key is insertable");
        assert!(diff.delete_ids.is_empty());
    }

    #[test]
    fn each_existing_row_absorbs_at_most_one_candidate() {
        // Two near-identical candidates, one pristine row: the first updates it, the
        // second becomes a fresh insert (the row is consumed).
        let existing = vec![pristine("ai-1", "send the deck to alice")];
        let diff = compute_diff(
            &existing,
            &[
                candidate("send the deck to alice"),
                candidate("send the deck to alice today"),
            ],
        );
        assert_eq!(diff.updates.len(), 1);
        assert_eq!(diff.inserts.len(), 1);
        assert!(diff.delete_ids.is_empty());
    }

    #[test]
    fn empty_candidate_list_deletes_only_pristine_rows() {
        let existing = vec![
            pristine("ai-1", "task one"),
            row("ai-2", "task two", "completed", "extracted", false),
            row("ai-3", "task three", "open", "manual", false),
        ];
        let diff = compute_diff(&existing, &[]);
        assert_eq!(diff.delete_ids, vec!["ai-1".to_string()]);
        assert!(diff.inserts.is_empty());
        assert!(diff.updates.is_empty());
    }

    #[test]
    fn empty_descriptions_never_fuzzy_match() {
        let existing = vec![pristine("ai-1", "!!!")]; // normalizes to ""
        let diff = compute_diff(&existing, &[candidate("...")]); // also normalizes to ""
                                                                 // Exact content_key of "" matches "" — that's identity, allowed; assert that the
                                                                 // EXACT path (not a spurious fuzzy path) is what matched.
        assert_eq!(diff.updates.len(), 1);
        // But an empty candidate vs a real row must not match at all.
        let existing = vec![pristine("ai-2", "real task here")];
        let diff = compute_diff(&existing, &[candidate("...")]);
        assert_eq!(diff.inserts.len(), 1);
        assert_eq!(diff.delete_ids, vec!["ai-2".to_string()]);
    }
}
