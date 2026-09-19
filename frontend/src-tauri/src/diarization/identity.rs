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
//!   3. *Repeatedly recognized* (specs/0064 W2): the match is a prior-speaker (1a) sample
//!      linked to a durable person, seen across at least
//!      [`crate::diarization::auto_label::AUTO_PRIOR_MEETINGS_MIN`] distinct prior meetings,
//!      at `confidence >= `[`TAU_VOICE_AUTO`]. Repetition is this route's corroboration.
//!
//! A gallery match without email corroboration, and any match below those bars, only ever
//! *suggests*.

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
    // specs/0064 W2: a prior-speaker (1a) match recognized across enough DISTINCT prior
    // meetings, at the same acoustic bar the voice-only gallery route demands, is applied
    // too — it must resolve to a durable person, since that link is what applying it means.
    let prior_meetings_auto = !best.best_from_gallery
        && best.person_id.is_some()
        && crate::diarization::auto_label::prior_meeting_auto(best.best_cosine, best.meeting_count);
    let auto_label = email_auto || trusted_voice_auto || prior_meetings_auto;

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
mod tests;
