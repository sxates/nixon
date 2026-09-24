//! The cross-meeting matcher's inputs for one meeting: who the current speakers could be
//! (specs/0016 1a/1c). Moved out of `pipeline.rs` for specs/0078, which also matches the
//! owner in room recordings.
//!
//! Candidates are prior meetings' identified speakers (1a) plus the durable voiceprint
//! gallery centroids (1c). The owner is excluded in a call: "You" is the mic channel there,
//! and a similarity hit could only mislabel a remote cluster (echo bleed) as the owner
//! (specs/0043 W1.4). In a room recording the owner is one of the clusters, so the owner's
//! gallery centroid becomes an ordinary candidate, judged by the same thresholds and the
//! same `TRUSTED_GALLERY_MIN_SAMPLES` as anyone else (specs/0078 owner decision 3).

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::database::repositories::meeting_audio_setup::MeetingAudioSetupRepository;
use crate::database::repositories::people::PeopleRepository;
use crate::database::repositories::speaker::SpeakersRepository;
use crate::database::repositories::voiceprints::VoiceprintsRepository;
use crate::diarization::embedding::{embedding_to_bytes, EMBEDDING_MODEL_ID};
use crate::diarization::identity::{self, CandidateSample, SpeakerSuggestion};
use crate::diarization::LOCAL_SPEAKER_KEY;
use crate::people::enroll::OWNER_PERSON_ID;

/// Run the cross-meeting matcher for a saved meeting with NO corroborating emails, so the
/// email-corroborated auto-label route cannot fire; the voice-only routes still can. Both
/// real callers (the offline pass and `api_get_speaker_suggestions`) supply the meeting's
/// attendee emails via [`compute_suggestions_with_emails`]; this is the email-free case
/// used by tests.
pub async fn compute_suggestions(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<Vec<SpeakerSuggestion>> {
    compute_suggestions_with_emails(pool, meeting_id, &[]).await
}

/// Load this meeting's stored voiceprints and the candidate set, then rank by cosine via
/// the pure [`identity`] matcher. Shared by the offline pass and the on-demand command, so
/// both produce identical results. `corroborating_emails` (the meeting's calendar-attendee
/// emails) feeds the matcher's auto-label gate (specs/0064 W2: the refetch applies
/// auto-labels too, so it needs the same inputs the pass had).
///
/// In a room recording that has no "You" yet, the owner competes (see the module docs).
/// Only an owner match that clears the auto-label bar is kept: `auto_label::apply` turns
/// it into "You" by re-keying the cluster to `local`. An owner match below that bar is
/// dropped rather than shown as a chip, because accepting a chip links a person to the
/// cluster and would leave two "You"s; "This is me" is the manual route.
pub async fn compute_suggestions_with_emails(
    pool: &SqlitePool,
    meeting_id: &str,
    corroborating_emails: &[String],
) -> Result<Vec<SpeakerSuggestion>> {
    let current = SpeakersRepository::get_meeting_embeddings(pool, meeting_id)
        .await
        .with_context(|| format!("load embeddings for meeting {meeting_id}"))?;
    if current.is_empty() {
        return Ok(Vec::new());
    }
    let include_owner = owner_is_a_candidate(pool, meeting_id).await;
    let candidates = load_candidates(pool, meeting_id, include_owner).await?;
    Ok(keep_owner_only_as_auto_label(
        identity::match_speakers_with_gallery(&current, &candidates, corroborating_emails),
    ))
}

/// Whether the owner competes for this meeting's clusters: the last pass clustered the
/// owner (room/hybrid) and no speaker is "You" yet. Best-effort: any read error means no.
async fn owner_is_a_candidate(pool: &SqlitePool, meeting_id: &str) -> bool {
    let clustered = matches!(
        MeetingAudioSetupRepository::get(pool, meeting_id).await,
        Ok(Some(s)) if s.resolved.is_some_and(|r| r.owner_is_clustered())
    );
    if !clustered {
        return false;
    }
    let has_local: Result<bool, _> = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM speakers WHERE meeting_id = ? AND speaker_key = ?)",
    )
    .bind(meeting_id)
    .bind(LOCAL_SPEAKER_KEY)
    .fetch_one(pool)
    .await;
    matches!(has_local, Ok(false))
}

/// The candidate set for matching `meeting_id`'s speakers.
///
/// - 1a: prior per-meeting speaker embeddings, excluding THIS meeting's own rows (a
///   speaker is never suggested against itself) and any row linked to the owner.
/// - 1c: gallery centroids, each enriched with the person's name/email. The owner's
///   centroid is included only when `include_owner` (room recordings). Best-effort: a
///   gallery read failure leaves the 1a candidates.
pub(crate) async fn load_candidates(
    pool: &SqlitePool,
    meeting_id: &str,
    include_owner: bool,
) -> Result<Vec<CandidateSample>> {
    let identified = SpeakersRepository::get_identified_with_embeddings(pool)
        .await
        .context("load identified candidate speakers")?;

    let mut candidates: Vec<CandidateSample> = identified
        .into_iter()
        .filter(|c| c.meeting_id != meeting_id && c.person_id.as_deref() != Some(OWNER_PERSON_ID))
        .map(|c| CandidateSample {
            display_name: c.display_name,
            email: c.email,
            person_id: c.person_id,
            embedding: c.embedding,
            embedding_model: c.embedding_model,
            from_gallery: false,
            gallery_sample_count: 0,
            // The meeting this prior speaker row belongs to: the matcher counts DISTINCT
            // meetings for the repetition tier (specs/0064 W2 review).
            meeting_id: Some(c.meeting_id),
        })
        .collect();

    match VoiceprintsRepository::all_centroids(pool, EMBEDDING_MODEL_ID).await {
        Ok(centroids) => {
            for (person_id, centroid, sample_count) in centroids {
                // specs/0043 W1.4: in a call the owner never competes (see module docs).
                if person_id == OWNER_PERSON_ID && !include_owner {
                    continue;
                }
                let (name, email) = match PeopleRepository::get(pool, &person_id).await {
                    Ok(Some(p)) => (p.display_name, p.email),
                    // Centroid with no person row (shouldn't happen: explicit cascades);
                    // skip rather than label with a placeholder.
                    _ => continue,
                };
                candidates.push(CandidateSample {
                    display_name: name,
                    email,
                    person_id: Some(person_id),
                    embedding: embedding_to_bytes(&centroid),
                    embedding_model: Some(EMBEDDING_MODEL_ID.to_string()),
                    from_gallery: true,
                    // specs/0044 WS4: enrollment count gates the trusted (voice-only)
                    // auto-label tier in the matcher.
                    gallery_sample_count: sample_count,
                    // A centroid belongs to no single meeting.
                    meeting_id: None,
                });
            }
        }
        Err(e) => log::warn!(
            "diarization: gallery centroid load failed for meeting {meeting_id} ({e}); \
             matching prior speakers only"
        ),
    }
    Ok(candidates)
}

/// Keep at most ONE owner suggestion, and only one the matcher marked `auto_label`: the
/// highest-confidence one. Every other owner suggestion is dropped (see
/// [`compute_suggestions_with_emails`]). Non-owner suggestions pass through untouched.
pub(crate) fn keep_owner_only_as_auto_label(
    suggestions: Vec<SpeakerSuggestion>,
) -> Vec<SpeakerSuggestion> {
    let is_owner =
        |s: &SpeakerSuggestion| s.suggested_person_id.as_deref() == Some(OWNER_PERSON_ID);
    let best_owner_key = suggestions
        .iter()
        .filter(|s| is_owner(s) && s.auto_label)
        .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
        .map(|s| s.speaker_key.clone());
    suggestions
        .into_iter()
        .filter(|s| !is_owner(s) || Some(&s.speaker_key) == best_owner_key.as_ref())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggestion(key: &str, person: &str, confidence: f32, auto: bool) -> SpeakerSuggestion {
        SpeakerSuggestion {
            speaker_key: key.to_string(),
            suggested_name: "Someone".to_string(),
            suggested_email: None,
            suggested_person_id: Some(person.to_string()),
            confidence,
            basis: String::new(),
            auto_label: auto,
        }
    }

    #[test]
    fn owner_suggestions_survive_only_as_the_single_best_auto_label() {
        let out = keep_owner_only_as_auto_label(vec![
            suggestion("spk_0", OWNER_PERSON_ID, 0.95, false), // a chip, even the closest: dropped
            suggestion("spk_1", OWNER_PERSON_ID, 0.90, true),  // the best auto: kept
            suggestion("spk_2", OWNER_PERSON_ID, 0.85, true),  // a second auto: dropped
            suggestion("spk_3", "p-other", 0.60, false),       // not the owner: kept
        ]);
        let keys: Vec<&str> = out.iter().map(|s| s.speaker_key.as_str()).collect();
        assert_eq!(keys, vec!["spk_1", "spk_3"]);
    }
}
