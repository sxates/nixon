//! Cross-meeting speaker matcher (specs/0016 Phase 1a, ADR-0007 §4/§5).
//!
//! Given the current meeting's per-remote-speaker embeddings and the set of
//! previously-identified speakers (each with a stored CAM++ voiceprint and a known
//! name/email), suggest a name for each current speaker by cosine similarity.
//!
//! This module is **pure**: it does no I/O, no DB access, and no network. Every
//! input is passed in by value/reference; every output is a plain struct. That keeps
//! it unit-testable without a model or audio, and — by construction — guarantees no
//! embedding bytes ever leave the process here (the no-leak invariant is asserted in
//! the tests below). Loading candidates and persisting results is the caller's job
//! (`diarization::commands` / `diarization::pipeline`).
//!
//! ## Matching rules
//! - Compare **only** vectors sharing the same `embedding_model` (cosine across
//!   different models is meaningless — ADR-0007 §4).
//! - For each current remote speaker, score cosine vs every candidate person, taking
//!   each person's *best* sample. Accept the top person iff `best >= TAU_MATCH` AND it
//!   beats the runner-up person by at least `MARGIN`. This rejects both weak matches
//!   and ambiguous "two plausible people" cases — a confident-wrong label is strictly
//!   worse than asking (ADR-0007 §5).
//!
//! ## Confidence tiers + the email-corroboration auto-label gate (specs/0016 1c)
//!
//! Candidates come from two sources: prior per-meeting `speakers` embeddings (1a) and the
//! durable per-person voiceprint **gallery** centroids (1c). A cleared match becomes one
//! of three outcomes:
//!
//! - **auto-label** (`auto_label = true`) — applied without asking. Two routes (both
//!   gallery-only, both require the runner-up margin):
//!   1. *Email-corroborated* (specs/0016 1c): `confidence >= `[`TAU_AUTO_LABEL`] AND a
//!      **calendar-attendee email that agrees** with the matched person's email.
//!   2. *Trusted voiceprint* (specs/0044 WS4): the person's gallery is well-trained
//!      (`gallery_sample_count >= `[`TRUSTED_GALLERY_MIN_SAMPLES`], grown by
//!      enroll-on-confirm) AND `confidence >= `[`TAU_VOICE_AUTO`] — voice alone, no
//!      calendar needed. A high voice-only match on a THIN gallery still only suggests.
//! - **suggest-and-confirm** (`auto_label = false`) — a high voice-only match, OR a
//!   medium match (`>= TAU_MATCH`). Surfaced as a chip the user confirms.
//! - **Speaker N** — below `TAU_MATCH`, or ambiguous → no suggestion at all.
//!
//! Prior-speaker (1a) matches and gallery matches without email corroboration only ever
//! *suggest*; auto-label is exclusively the high+corroborated gallery path.

use serde::Serialize;

use crate::diarization::embedding::{cosine_similarity, embedding_from_bytes};

/// Minimum cosine to the best candidate before we suggest at all. CAM++ cosine on
/// VoxCeleb-trained vectors (per specs/0011); conservative starting point, tunable on
/// the diarization benchmark. Too low → confident-wrong suggestions (the worst UX).
pub const TAU_MATCH: f32 = 0.5;

/// The high-confidence bar a **gallery** match must clear (on top of email corroboration
/// and the margin) to be **auto-labeled** without asking. Strictly higher than the
/// suggest threshold `TAU_MATCH` — auto-label must be more conservative than suggest
/// (ADR-0007 §5), since a silent false-accept is the worst outcome. Benchmark-tunable.
pub const TAU_AUTO_LABEL: f32 = 0.7;

/// The best candidate must beat the runner-up *person* by at least this much, else the
/// match is ambiguous and we suggest nothing. Conservative; benchmark-tunable.
pub const MARGIN: f32 = 0.06;

/// specs/0044 WS4 — the bar a **well-trained** gallery match must clear to auto-label on
/// voice alone (no calendar corroboration). Strictly higher than [`TAU_AUTO_LABEL`]:
/// dropping the email requirement demands more acoustic confidence in exchange.
pub const TAU_VOICE_AUTO: f32 = 0.80;

/// specs/0044 WS4 — minimum confirmed enrollments (voiceprint samples) behind a gallery
/// centroid before it counts as "well-trained" for voice-only auto-labeling. Below this
/// the centroid is too easy to skew with one bad sample; enroll-on-confirm
/// (`people/enroll.rs`) grows a person past it naturally after a few confirmations.
pub const TRUSTED_GALLERY_MIN_SAMPLES: usize = 3;

/// A name suggestion for one current-meeting speaker, surfaced in the UI as a
/// dismissible chip and carried on the `diarization-complete` event.
///
/// Deliberately carries **no embedding bytes** — only display-safe fields — so it can
/// be serialized to the frontend without leaking biometric data (ADR-0007 §1).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerSuggestion {
    /// The current meeting's speaker key this suggestion is for (`spk_N`).
    pub speaker_key: String,
    /// The suggested display name (the matched person's name).
    pub suggested_name: String,
    /// The matched person's email, when the prior identity carried one.
    pub suggested_email: Option<String>,
    /// The durable person this suggestion resolves to (specs/0016 1b), when the matched
    /// prior speaker was linked to a `people` row. The frontend can one-click
    /// `api_assign_speaker_to_person` with this id; `None` for email/name-only matches.
    pub suggested_person_id: Option<String>,
    /// Best cosine similarity to the matched person (in `[-1, 1]`).
    pub confidence: f32,
    /// Human-readable rationale for the chip, e.g.
    /// "matched Priya's voice from 2 prior meetings".
    pub basis: String,
    /// Whether this suggestion may be applied WITHOUT asking (specs/0016 1c). True only
    /// for a high-confidence **gallery** match whose person is corroborated by a calendar
    /// attendee email (the product-decision auto-label gate). The offline pass applies
    /// auto-labels directly; everything else (`false`) stays a confirm-first chip.
    pub auto_label: bool,
}

/// One candidate voiceprint sample loaded from a prior meeting's identified speaker.
///
/// Several samples can belong to the same person (same `email`, or same name when no
/// email); they are grouped by [`person_key`] so "from N prior meetings" counts and
/// the runner-up margin are computed per *person*, not per sample.
#[derive(Debug, Clone)]
pub struct CandidateSample {
    pub display_name: String,
    pub email: Option<String>,
    /// The durable person this prior speaker was linked to (specs/0016 1b), when assigned
    /// via `api_assign_speaker_to_person`. This is the STRONGEST grouping key: samples
    /// sharing a `person_id` are the same human across meetings even if their email/name
    /// drifted. Falls back to email, then name, when absent.
    pub person_id: Option<String>,
    /// L2-normalized embedding bytes (LE f32) as stored in `speakers.embedding`.
    pub embedding: Vec<u8>,
    /// The model that produced the embedding; only same-model vectors are compared.
    pub embedding_model: Option<String>,
    /// Whether this candidate is a durable voiceprint-**gallery** centroid (specs/0016
    /// 1c) rather than a prior per-meeting speaker embedding (1a). Only gallery matches
    /// are eligible for the auto-label paths.
    pub from_gallery: bool,
    /// specs/0044 WS4: how many stored samples back this gallery centroid (0 for 1a
    /// prior-speaker candidates). Gates the voice-only auto-label tier
    /// ([`TRUSTED_GALLERY_MIN_SAMPLES`]).
    pub gallery_sample_count: usize,
}

impl CandidateSample {
    /// Stable per-person grouping key, strongest first: the durable `person_id` when set
    /// (specs/0016 1b), else the lowercased email, else the (case-folded) display name.
    /// Grouping by `person_id` means a person identified in a prior meeting is counted
    /// once across all their meetings, and the "from N prior meetings" count is per
    /// durable person rather than per email/name spelling.
    fn person_key(&self) -> String {
        if let Some(pid) = self.person_id.as_deref() {
            if !pid.trim().is_empty() {
                return format!("person:{}", pid.trim());
            }
        }
        match self.email.as_deref() {
            Some(e) if !e.trim().is_empty() => format!("email:{}", e.trim().to_lowercase()),
            _ => format!("name:{}", self.display_name.trim().to_lowercase()),
        }
    }
}

/// A current-meeting remote speaker to match: its key + decoded embedding + model.
struct QuerySpeaker {
    speaker_key: String,
    embedding: Vec<f32>,
    embedding_model: Option<String>,
}

/// Per-person aggregate over its candidate samples.
struct PersonScore {
    display_name: String,
    email: Option<String>,
    person_id: Option<String>,
    best_cosine: f32,
    meeting_count: usize,
    /// Whether the BEST-scoring sample for this person came from the voiceprint gallery
    /// (1c) rather than a prior per-meeting speaker (1a). Gates auto-label eligibility.
    best_from_gallery: bool,
    /// Sample count behind the best-scoring sample's gallery centroid (0 for 1a).
    best_gallery_sample_count: usize,
}

/// Run the matcher. `current` is `(speaker_key, embedding_bytes, embedding_model)` for
/// the meeting's REMOTE speakers; `candidates` is the prior-art set (prior speakers +
/// gallery centroids). Returns one [`SpeakerSuggestion`] per current speaker that clears
/// `TAU_MATCH` + `MARGIN`.
///
/// `corroborating_emails` is the set of calendar-attendee emails for THIS meeting,
/// lowercased — used only by the auto-label gate (a gallery match is auto-labeled iff its
/// matched person's email is in this set). Pass `&[]` to disable auto-label entirely
/// (everything becomes suggest-and-confirm). 1a callers that don't supply gallery
/// candidates can use [`match_speakers`].
///
/// Pure — see the module docs. Bad/short byte blobs are skipped defensively.
pub fn match_speakers_with_gallery(
    current: &[(String, Vec<u8>, Option<String>)],
    candidates: &[CandidateSample],
    corroborating_emails: &[String],
) -> Vec<SpeakerSuggestion> {
    // Decode the query speakers once.
    let queries: Vec<QuerySpeaker> = current
        .iter()
        .filter_map(|(key, bytes, model)| match embedding_from_bytes(bytes) {
            Ok(v) if !v.is_empty() => Some(QuerySpeaker {
                speaker_key: key.clone(),
                embedding: v,
                embedding_model: model.clone(),
            }),
            _ => None,
        })
        .collect();

    let mut out = Vec::new();
    for q in &queries {
        if let Some(s) = best_suggestion_for(q, candidates, corroborating_emails) {
            out.push(s);
        }
    }
    out
}

/// Convenience wrapper for the 1a path (prior-speaker candidates only, no auto-label):
/// runs [`match_speakers_with_gallery`] with no corroborating emails, so every result is
/// suggest-and-confirm.
pub fn match_speakers(
    current: &[(String, Vec<u8>, Option<String>)],
    candidates: &[CandidateSample],
) -> Vec<SpeakerSuggestion> {
    match_speakers_with_gallery(current, candidates, &[])
}

/// Score one query speaker against all candidates, grouped by person, and return a
/// suggestion iff the best person clears `TAU_MATCH` and beats the runner-up person by
/// `MARGIN`. Only candidates sharing the query's `embedding_model` are considered.
fn best_suggestion_for(
    q: &QuerySpeaker,
    candidates: &[CandidateSample],
    corroborating_emails: &[String],
) -> Option<SpeakerSuggestion> {
    use std::collections::HashMap;

    let mut by_person: HashMap<String, PersonScore> = HashMap::new();

    for c in candidates {
        // Same-model only (ADR-0007 §4). Both None also counts as "same" (legacy rows
        // written before the model tag existed are CAM++ by construction).
        if c.embedding_model != q.embedding_model {
            continue;
        }
        let cand_vec = match embedding_from_bytes(&c.embedding) {
            Ok(v) if !v.is_empty() => v,
            _ => continue,
        };
        let cos = cosine_similarity(&q.embedding, &cand_vec);
        let pkey = c.person_key();
        let entry = by_person.entry(pkey).or_insert_with(|| PersonScore {
            display_name: c.display_name.clone(),
            email: c.email.clone(),
            person_id: c.person_id.clone(),
            best_cosine: f32::MIN,
            meeting_count: 0,
            best_from_gallery: c.from_gallery,
            best_gallery_sample_count: c.gallery_sample_count,
        });
        // A gallery centroid is one durable artifact, not a per-meeting occurrence; only
        // prior-speaker samples count toward the "from N prior meetings" tally.
        if !c.from_gallery {
            entry.meeting_count += 1;
        }
        if cos > entry.best_cosine {
            entry.best_cosine = cos;
            // Prefer the name/email/person of the sample that scored best.
            entry.display_name = c.display_name.clone();
            entry.email = c.email.clone();
            entry.person_id = c.person_id.clone();
            entry.best_from_gallery = c.from_gallery;
            entry.best_gallery_sample_count = c.gallery_sample_count;
        }
    }

    // Rank people by their best cosine.
    let mut ranked: Vec<&PersonScore> = by_person.values().collect();
    ranked.sort_by(|a, b| {
        b.best_cosine
            .partial_cmp(&a.best_cosine)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let best = ranked.first()?;
    if best.best_cosine < TAU_MATCH {
        return None;
    }
    // Ambiguity guard: a close runner-up person means "not confident enough".
    if let Some(runner_up) = ranked.get(1) {
        if best.best_cosine - runner_up.best_cosine < MARGIN {
            return None;
        }
    }

    // Auto-label gate (product decision, refines ADR-0007 §5): ALL of —
    //   1. the winning sample is a GALLERY centroid (1c),
    //   2. confidence clears the higher TAU_AUTO_LABEL bar,
    //   3. a calendar-attendee email AGREES with the matched person's email.
    // A voice-only high match (no corroborating email) must NOT auto-label.
    let email_corroborated = best
        .email
        .as_deref()
        .map(|e| {
            let e = e.trim().to_lowercase();
            !e.is_empty() && corroborating_emails.iter().any(|c| c == &e)
        })
        .unwrap_or(false);
    let email_auto =
        best.best_from_gallery && best.best_cosine >= TAU_AUTO_LABEL && email_corroborated;
    // specs/0044 WS4: a WELL-TRAINED gallery (enough confirmed enrollments) auto-labels
    // on voice alone past the higher TAU_VOICE_AUTO bar — no calendar needed.
    let trusted_voice_auto = best.best_from_gallery
        && best.best_gallery_sample_count >= TRUSTED_GALLERY_MIN_SAMPLES
        && best.best_cosine >= TAU_VOICE_AUTO;
    let auto_label = email_auto || trusted_voice_auto;

    let basis = if best.best_from_gallery {
        if email_auto {
            format!(
                "recognized {}'s voice (confirmed by calendar)",
                best.display_name
            )
        } else if trusted_voice_auto {
            format!("recognized {}'s well-trained voiceprint", best.display_name)
        } else {
            format!("looks like {} (from the voice gallery)", best.display_name)
        }
    } else {
        let n = best.meeting_count;
        let plural = if n == 1 { "meeting" } else { "meetings" };
        format!(
            "matched {}'s voice from {} prior {}",
            best.display_name, n, plural
        )
    };

    Some(SpeakerSuggestion {
        speaker_key: q.speaker_key.clone(),
        suggested_name: best.display_name.clone(),
        suggested_email: best.email.clone(),
        suggested_person_id: best.person_id.clone(),
        confidence: best.best_cosine,
        basis,
        auto_label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::embedding::{embedding_to_bytes, l2_normalize};

    const MODEL: &str = "3dspeaker_campplus_sv_en_voxceleb_16k";

    fn bytes(v: &[f32]) -> Vec<u8> {
        embedding_to_bytes(&l2_normalize(v))
    }

    fn cand(name: &str, email: Option<&str>, v: &[f32]) -> CandidateSample {
        CandidateSample {
            display_name: name.to_string(),
            email: email.map(|s| s.to_string()),
            person_id: None,
            embedding: bytes(v),
            embedding_model: Some(MODEL.to_string()),
            from_gallery: false,
            gallery_sample_count: 0,
        }
    }

    /// A candidate linked to a durable person (specs/0016 1b).
    fn cand_person(name: &str, email: Option<&str>, person_id: &str, v: &[f32]) -> CandidateSample {
        CandidateSample {
            display_name: name.to_string(),
            email: email.map(|s| s.to_string()),
            person_id: Some(person_id.to_string()),
            embedding: bytes(v),
            embedding_model: Some(MODEL.to_string()),
            from_gallery: false,
            gallery_sample_count: 0,
        }
    }

    /// A durable voiceprint-GALLERY centroid candidate (specs/0016 1c). `samples` is
    /// the enrollment count behind the centroid (specs/0044 WS4 trusted-gallery gate).
    fn cand_gallery(
        name: &str,
        email: Option<&str>,
        person_id: &str,
        v: &[f32],
        samples: usize,
    ) -> CandidateSample {
        CandidateSample {
            display_name: name.to_string(),
            email: email.map(|s| s.to_string()),
            person_id: Some(person_id.to_string()),
            embedding: bytes(v),
            embedding_model: Some(MODEL.to_string()),
            from_gallery: true,
            gallery_sample_count: samples,
        }
    }

    #[test]
    fn clear_match_suggests() {
        // Query is (almost) identical to Priya's voiceprint, far from anyone else.
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let other = vec![0.0, 1.0, 0.0, 0.0];
        let candidates = vec![
            cand("Priya", Some("priya@example.com"), &priya),
            cand("Sam", Some("sam@example.com"), &other),
        ];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

        let suggestions = match_speakers(&current, &candidates);
        assert_eq!(suggestions.len(), 1);
        let s = &suggestions[0];
        assert_eq!(s.speaker_key, "spk_0");
        assert_eq!(s.suggested_name, "Priya");
        assert_eq!(s.suggested_email.as_deref(), Some("priya@example.com"));
        assert!(s.confidence > 0.99, "confidence was {}", s.confidence);
        assert!(s.basis.contains("Priya"));
    }

    #[test]
    fn ambiguous_two_close_candidates_does_not_suggest() {
        // Two DIFFERENT people both sit ~equally close to the query → ambiguous.
        // Query bisects them; both score the same cosine, runner-up margin fails.
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, -1.0, 0.0, 0.0];
        let query = vec![1.0, 0.0, 0.0, 0.0]; // equidistant from a and b
        let candidates = vec![
            cand("Alex", Some("alex@example.com"), &a),
            cand("Blair", Some("blair@example.com"), &b),
        ];
        let current = vec![("spk_0".to_string(), bytes(&query), Some(MODEL.to_string()))];

        let suggestions = match_speakers(&current, &candidates);
        assert!(
            suggestions.is_empty(),
            "ambiguous match should not suggest, got {suggestions:?}"
        );
    }

    #[test]
    fn cross_model_candidates_are_ignored() {
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        // Same vector but a DIFFERENT embedding model → must not be compared.
        let candidates = vec![CandidateSample {
            display_name: "Priya".to_string(),
            email: Some("priya@example.com".to_string()),
            person_id: None,
            embedding: bytes(&priya),
            embedding_model: Some("some_other_model_v2".to_string()),
            from_gallery: false,
            gallery_sample_count: 0,
        }];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

        let suggestions = match_speakers(&current, &candidates);
        assert!(
            suggestions.is_empty(),
            "cross-model candidate must be ignored, got {suggestions:?}"
        );
    }

    #[test]
    fn weak_match_below_tau_does_not_suggest() {
        // Near-orthogonal: cosine well under TAU_MATCH.
        let query = vec![1.0, 0.0, 0.0, 0.0];
        let far = vec![0.05, 1.0, 0.0, 0.0];
        let candidates = vec![cand("Priya", Some("priya@example.com"), &far)];
        let current = vec![("spk_0".to_string(), bytes(&query), Some(MODEL.to_string()))];

        assert!(match_speakers(&current, &candidates).is_empty());
    }

    #[test]
    fn meeting_count_groups_by_person() {
        // Same person across two prior meetings → "2 prior meetings".
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let priya2 = vec![0.99, 0.01, 0.0, 0.0]; // same person, slight drift
        let candidates = vec![
            cand("Priya", Some("priya@example.com"), &priya),
            cand("Priya", Some("priya@example.com"), &priya2),
        ];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

        let suggestions = match_speakers(&current, &candidates);
        assert_eq!(suggestions.len(), 1);
        assert!(
            suggestions[0].basis.contains("2 prior meetings"),
            "basis was {:?}",
            suggestions[0].basis
        );
    }

    #[test]
    fn groups_by_person_id_across_differing_email_spellings() {
        // The SAME durable person across two prior meetings, but the two speaker rows
        // carry different email spellings (e.g. an alias vs. primary). Grouping by
        // person_id must collapse them into ONE candidate → "2 prior meetings", and the
        // suggestion must carry the person_id for one-click assign.
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let priya2 = vec![0.99, 0.01, 0.0, 0.0]; // same person, slight drift
        let candidates = vec![
            cand_person("Priya", Some("priya@example.com"), "person-123", &priya),
            cand_person(
                "Priya R.",
                Some("priya.r@example.com"),
                "person-123",
                &priya2,
            ),
        ];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

        let suggestions = match_speakers(&current, &candidates);
        assert_eq!(suggestions.len(), 1);
        let s = &suggestions[0];
        assert_eq!(s.suggested_person_id.as_deref(), Some("person-123"));
        assert!(
            s.basis.contains("2 prior meetings"),
            "person grouping should count both meetings, basis was {:?}",
            s.basis
        );
    }

    #[test]
    fn person_id_grouping_beats_name_fallback() {
        // Two candidates with the SAME display name but DIFFERENT person_ids must NOT be
        // collapsed by the name fallback — they are distinct people. Here each is far from
        // the other so the closer one wins cleanly, and it carries its own person_id.
        let here = vec![1.0, 0.0, 0.0, 0.0];
        let there = vec![0.0, 1.0, 0.0, 0.0];
        let candidates = vec![
            cand_person("Sam", None, "person-aaa", &here),
            cand_person("Sam", None, "person-bbb", &there),
        ];
        let current = vec![("spk_0".to_string(), bytes(&here), Some(MODEL.to_string()))];

        let suggestions = match_speakers(&current, &candidates);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(
            suggestions[0].suggested_person_id.as_deref(),
            Some("person-aaa")
        );
    }

    #[test]
    fn suggestions_default_to_confirm_not_auto_label() {
        // A clear prior-speaker (1a) match suggests, but never auto-labels.
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let candidates = vec![cand("Priya", Some("priya@example.com"), &priya)];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];
        let s = &match_speakers(&current, &candidates)[0];
        assert!(!s.auto_label, "1a prior-speaker match must not auto-label");
    }

    #[test]
    fn high_gallery_with_email_corroboration_auto_labels() {
        // A high-confidence GALLERY match whose person's email is a calendar attendee →
        // the ONE auto-label path (high voice + email agree).
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let candidates = vec![cand_gallery(
            "Priya",
            Some("priya@example.com"),
            "person-1",
            &priya,
            1, // thin gallery: the email route needs no training (specs/0044 WS4)
        )];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];
        let emails = vec!["priya@example.com".to_string()];

        let suggestions = match_speakers_with_gallery(&current, &candidates, &emails);
        assert_eq!(suggestions.len(), 1);
        let s = &suggestions[0];
        assert!(
            s.auto_label,
            "high gallery + corroborating email must auto-label (conf {})",
            s.confidence
        );
        assert!(s.confidence >= TAU_AUTO_LABEL);
        assert_eq!(s.suggested_person_id.as_deref(), Some("person-1"));
    }

    #[test]
    fn voice_only_high_match_on_thin_gallery_does_not_auto_label() {
        // Pre-0044 product gate, now scoped to THIN galleries: a near-perfect voice
        // match with too few confirmed enrollments (< TRUSTED_GALLERY_MIN_SAMPLES)
        // must NOT auto-label without a corroborating email — it suggests.
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let candidates = vec![cand_gallery(
            "Priya",
            Some("priya@example.com"),
            "person-1",
            &priya,
            TRUSTED_GALLERY_MIN_SAMPLES - 1,
        )];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

        // No corroborating emails at all.
        let s = &match_speakers_with_gallery(&current, &candidates, &[])[0];
        assert!(
            !s.auto_label,
            "voice-only high gallery match must suggest, not auto-label"
        );
        assert!(s.confidence > 0.99, "still a confident suggestion");

        // A DIFFERENT attendee email also does not corroborate.
        let other = vec!["someone-else@example.com".to_string()];
        let s2 = &match_speakers_with_gallery(&current, &candidates, &other)[0];
        assert!(!s2.auto_label, "non-matching email must not corroborate");
    }

    #[test]
    fn trusted_gallery_auto_labels_on_voice_alone() {
        // specs/0044 WS4: >= TRUSTED_GALLERY_MIN_SAMPLES confirmed enrollments and a
        // cosine past TAU_VOICE_AUTO auto-label with NO calendar corroboration.
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let candidates = vec![cand_gallery(
            "Priya",
            Some("priya@example.com"),
            "person-1",
            &priya,
            TRUSTED_GALLERY_MIN_SAMPLES,
        )];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

        let s = &match_speakers_with_gallery(&current, &candidates, &[])[0];
        assert!(
            s.auto_label,
            "trusted gallery + high cosine must auto-label voice-only, got {s:?}"
        );
        assert!(s.confidence >= TAU_VOICE_AUTO);
        assert!(s.basis.contains("well-trained"), "basis was {}", s.basis);
    }

    #[test]
    fn trusted_gallery_between_the_two_bars_still_suggests() {
        // Voice-only demands the HIGHER bar: a trusted gallery at a cosine in
        // [TAU_AUTO_LABEL, TAU_VOICE_AUTO) — enough for the email route — must still
        // only suggest without corroboration.
        let target = vec![1.0, 0.0];
        // cos with [1,0] = 0.75 after normalize: between 0.70 and 0.80.
        let query = vec![0.75, (1.0f32 - 0.75 * 0.75).sqrt()];
        let candidates = vec![cand_gallery(
            "Priya",
            Some("priya@example.com"),
            "person-1",
            &target,
            TRUSTED_GALLERY_MIN_SAMPLES + 2,
        )];
        let current = vec![("spk_0".to_string(), bytes(&query), Some(MODEL.to_string()))];

        let suggestions = match_speakers_with_gallery(&current, &candidates, &[]);
        assert_eq!(suggestions.len(), 1);
        let s = &suggestions[0];
        assert!(
            s.confidence >= TAU_AUTO_LABEL && s.confidence < TAU_VOICE_AUTO,
            "conf {}",
            s.confidence
        );
        assert!(
            !s.auto_label,
            "below TAU_VOICE_AUTO must not voice-auto-label"
        );
    }

    #[test]
    fn prior_speaker_1a_candidates_never_voice_auto_label() {
        // The trusted tier is gallery-only: a 1a prior-speaker sample can never
        // auto-label regardless of cosine (its gallery_sample_count is 0).
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let candidates = vec![cand_person(
            "Priya",
            Some("priya@example.com"),
            "person-1",
            &priya,
        )];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

        let s = &match_speakers_with_gallery(&current, &candidates, &[])[0];
        assert!(!s.auto_label, "1a matches are always confirm-first");
    }

    #[test]
    fn medium_gallery_match_with_email_still_suggests() {
        // A gallery match that clears TAU_MATCH but NOT TAU_AUTO_LABEL stays a suggestion
        // even with a corroborating email (the higher bar isn't met). Construct a query
        // at a cosine in [TAU_MATCH, TAU_AUTO_LABEL).
        let target = vec![1.0, 0.0]; // person voiceprint
                                     // angle θ with cos θ ≈ 0.6 (between 0.5 and 0.7).
        let query = vec![0.6, 0.8]; // cos with [1,0] = 0.6 after normalize
        let candidates = vec![cand_gallery(
            "Priya",
            Some("priya@example.com"),
            "person-1",
            &target,
            5, // trusted-size gallery — cosine below both auto bars still only suggests
        )];
        let current = vec![("spk_0".to_string(), bytes(&query), Some(MODEL.to_string()))];
        let emails = vec!["priya@example.com".to_string()];

        let suggestions = match_speakers_with_gallery(&current, &candidates, &emails);
        assert_eq!(suggestions.len(), 1);
        let s = &suggestions[0];
        assert!(
            s.confidence >= TAU_MATCH && s.confidence < TAU_AUTO_LABEL,
            "conf {}",
            s.confidence
        );
        assert!(
            !s.auto_label,
            "medium match must not auto-label even with email"
        );
    }

    /// No-leak / no-I/O invariant: the matcher is a pure function over its inputs and
    /// its output carries no embedding bytes. We assert this structurally — the
    /// function takes only owned/borrowed value inputs (no handles, no pool, no app),
    /// returns only display-safe fields, and produces identical output on repeated
    /// calls with identical inputs (no hidden state / side-channel).
    #[test]
    fn matcher_is_pure_and_leaks_no_embeddings() {
        let priya = vec![1.0, 0.0, 0.0, 0.0];
        let candidates = vec![cand("Priya", Some("priya@example.com"), &priya)];
        let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

        let first = match_speakers(&current, &candidates);
        let second = match_speakers(&current, &candidates);
        assert_eq!(first, second, "pure function must be deterministic");

        // The serialized suggestion must not contain any embedding/voiceprint field.
        let json = serde_json::to_string(&first).unwrap();
        assert!(
            !json.contains("embedding"),
            "leaked embedding field: {json}"
        );
        assert!(
            !json.contains("voiceprint"),
            "leaked voiceprint field: {json}"
        );
        // And no raw float-blob array leaked under a renamed key.
        assert!(json.contains("speakerKey"));
        assert!(json.contains("confidence"));

        // The auto-label (gallery) path carries an embedding INTERNALLY as a candidate,
        // but the outward suggestion must STILL serialize with no embedding bytes — this
        // is the regulated payload (ADR-0007 §1). Assert it explicitly for the gallery
        // case too, since it's the one that triggers an auto-applied label.
        let emails = vec!["priya@example.com".to_string()];
        let gallery = vec![cand_gallery(
            "Priya",
            Some("priya@example.com"),
            "person-1",
            &priya,
            1,
        )];
        let auto = match_speakers_with_gallery(&current, &gallery, &emails);
        assert!(auto[0].auto_label);
        // Serialize the single suggestion object (not the Vec) so the array-leak check
        // below isn't tripped by the Vec's own brackets.
        let auto_json = serde_json::to_string(&auto[0]).unwrap();
        assert!(
            !auto_json.contains("embedding"),
            "gallery suggestion leaked embedding: {auto_json}"
        );
        assert!(
            !auto_json.contains("voiceprint"),
            "gallery suggestion leaked voiceprint: {auto_json}"
        );
        // No float array (a centroid would serialize as `[..,..,..]`) survived into the
        // wire payload — the suggestion object has only scalar/string fields.
        assert!(
            !auto_json.contains('['),
            "gallery suggestion leaked an array (possible vector): {auto_json}"
        );
    }
}
