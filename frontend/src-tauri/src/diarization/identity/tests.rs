use super::*;
use crate::diarization::embedding::{embedding_to_bytes, l2_normalize};

const MODEL: &str = "3dspeaker_campplus_sv_en_voxceleb_16k";

fn bytes(v: &[f32]) -> Vec<u8> {
    embedding_to_bytes(&l2_normalize(v))
}

/// Each 1a helper call stands for ONE prior meeting, so a test that builds N candidates is
/// describing N distinct meetings. `same_meeting_as` exists for the case that matters:
/// several speaker rows from a SINGLE meeting, which must count once (specs/0064 W2 review).
fn next_meeting_id() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    format!("m-prior-{}", N.fetch_add(1, Ordering::Relaxed))
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
        meeting_id: Some(next_meeting_id()),
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
        meeting_id: Some(next_meeting_id()),
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
        meeting_id: None,
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
        meeting_id: Some(next_meeting_id()),
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

#[test]
fn a_voice_recognized_across_enough_prior_meetings_auto_labels() {
    // specs/0064 W2: the prior-speaker path (1a) could never auto-label, so a person the
    // matcher recognized in meeting after meeting stayed behind a confirm chip forever.
    // Repetition across distinct meetings is the corroboration the gallery routes get
    // from an email or a trained centroid.
    use crate::diarization::auto_label::AUTO_PRIOR_MEETINGS_MIN;
    let priya = vec![1.0, 0.0, 0.0, 0.0];
    let candidates: Vec<CandidateSample> = (0..AUTO_PRIOR_MEETINGS_MIN)
        .map(|_| cand_person("Priya", Some("priya@example.com"), "person-1", &priya))
        .collect();
    let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

    let s = &match_speakers_with_gallery(&current, &candidates, &[])[0];
    assert!(
        s.auto_label,
        "a repeatedly-recognized voice must be named, not offered: {s:?}"
    );
    assert_eq!(s.suggested_person_id.as_deref(), Some("person-1"));
}

#[test]
fn a_voice_from_too_few_prior_meetings_still_suggests() {
    use crate::diarization::auto_label::AUTO_PRIOR_MEETINGS_MIN;
    let priya = vec![1.0, 0.0, 0.0, 0.0];
    let candidates: Vec<CandidateSample> = (0..AUTO_PRIOR_MEETINGS_MIN - 1)
        .map(|_| cand_person("Priya", Some("priya@example.com"), "person-1", &priya))
        .collect();
    let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];
    let s = &match_speakers_with_gallery(&current, &candidates, &[])[0];
    assert!(
        !s.auto_label,
        "one or two meetings is not yet corroboration"
    );
}

#[test]
fn a_prior_match_with_no_durable_person_never_auto_labels() {
    // Nothing to link a name to: applying it would be a no-op, so it stays a chip
    // (which is also how the user creates the person in the first place).
    use crate::diarization::auto_label::AUTO_PRIOR_MEETINGS_MIN;
    let priya = vec![1.0, 0.0, 0.0, 0.0];
    let candidates: Vec<CandidateSample> = (0..AUTO_PRIOR_MEETINGS_MIN + 2)
        .map(|_| cand("Priya", Some("priya@example.com"), &priya))
        .collect();
    let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];
    let s = &match_speakers_with_gallery(&current, &candidates, &[])[0];
    assert!(!s.auto_label);
}

#[test]
fn several_speaker_rows_from_ONE_meeting_are_not_several_meetings() {
    // specs/0064 W2 review — a meeting that over-clusters a person into three speaker rows
    // the user then labels identically must NOT satisfy the repetition tier. Repetition
    // across separate meetings is the corroboration; three rows from one meeting is one
    // meeting's worth of evidence, however confident the cosine.
    use crate::diarization::auto_label::AUTO_PRIOR_MEETINGS_MIN;
    let priya = vec![1.0, 0.0, 0.0, 0.0];
    let one_meeting = next_meeting_id();
    let candidates: Vec<CandidateSample> = (0..AUTO_PRIOR_MEETINGS_MIN + 2)
        .map(|_| {
            let mut c = cand_person("Priya", Some("priya@example.com"), "person-1", &priya);
            c.meeting_id = Some(one_meeting.clone());
            c
        })
        .collect();
    let current = vec![("spk_0".to_string(), bytes(&priya), Some(MODEL.to_string()))];

    let s = &match_speakers_with_gallery(&current, &candidates, &[])[0];
    assert!(
        !s.auto_label,
        "one over-clustered meeting is not {AUTO_PRIOR_MEETINGS_MIN} meetings of evidence: {s:?}"
    );
    assert!(
        s.basis.contains("1 prior meeting"),
        "the basis must say one meeting, got {}",
        s.basis
    );
}
