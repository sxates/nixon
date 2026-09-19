//! Applying auto-labels — the tier of speaker match confident enough to name without asking
//! (specs/0016 1c, specs/0044 WS4, extended by specs/0064 W2).
//!
//! The matcher in [`crate::diarization::identity`] decides *whether* a match may be applied
//! without asking; this module is *where* that decision is carried out. It used to live
//! inline in the offline diarization pass, which meant an `auto_label` suggestion produced by
//! any other path — notably the on-demand refetch behind `api_get_speaker_suggestions` — had
//! nobody to act on it and was rendered as a confirm-first chip instead.
//!
//! That gap was not theoretical. The voiceprint gallery is grown by enroll-on-confirm, so a
//! person crosses [`crate::diarization::identity::TRUSTED_GALLERY_MIN_SAMPLES`] *after* the
//! meetings that taught it. Those earlier meetings were diarized when no auto tier could
//! fire, and the only path that later recognizes the now-well-trained voiceprint was the one
//! that could not apply it — so the user was asked to confirm the same person forever.

use sqlx::SqlitePool;

use crate::database::repositories::people::PeopleRepository;
use crate::database::repositories::speaker::SpeakersRepository;
use crate::diarization::identity::{SpeakerSuggestion, TAU_VOICE_AUTO};

/// specs/0064 W2 — how many DISTINCT prior meetings the same person must have been
/// recognized in before a gallery-free (prior-speaker, specs/0016 1a) match may be applied
/// without asking.
///
/// The gallery routes buy their confidence with corroboration: a calendar attendee email, or
/// a centroid trained by several confirmations. The prior-speaker path has neither, so
/// repetition across separate meetings is what stands in for it — one lucky cosine on a
/// single past meeting is exactly the "confident-wrong label" ADR-0007 §5 warns about, while
/// the same person recognized across three meetings has been confirmed by the user at least
/// that many times.
pub const AUTO_PRIOR_MEETINGS_MIN: usize = 3;

/// Whether a NON-gallery (prior-speaker) match may be auto-applied: it must clear the same
/// acoustic bar as the voice-only gallery route AND have been seen in enough distinct prior
/// meetings. Pure, so the tier boundary is unit-testable without a DB.
pub fn prior_meeting_auto(confidence: f32, meeting_count: usize) -> bool {
    confidence >= TAU_VOICE_AUTO && meeting_count >= AUTO_PRIOR_MEETINGS_MIN
}

/// Persist every `auto_label` suggestion as a real identity assignment, exactly as a user
/// confirmation would (so enroll-on-confirm and the durable person link behave identically).
///
/// Best-effort per row: a failure is logged and the rest still apply, because the caller has
/// already committed the work these names decorate. Returns how many were applied.
///
/// A suggestion with no `suggested_person_id` cannot be applied — there is no durable person
/// to link to — and is left as a chip.
pub async fn apply(
    pool: &SqlitePool,
    meeting_id: &str,
    suggestions: &[SpeakerSuggestion],
) -> usize {
    let mut applied = 0usize;
    // Already-linked rows are skipped rather than re-written: the matcher re-derives the same
    // auto-label on every refetch (a named speaker keeps its embedding and keeps matching), so
    // without this the count would say "applied" forever and the log would repeat each time a
    // meeting is opened. Skipping makes a non-zero count mean "something actually changed".
    let linked = SpeakersRepository::get_by_meeting(pool, meeting_id)
        .await
        .unwrap_or_default();
    for s in suggestions.iter().filter(|s| s.auto_label) {
        if linked
            .iter()
            .any(|row| row.speaker_key == s.speaker_key && row.person_id == s.suggested_person_id)
        {
            continue;
        }
        let Some(person_id) = s.suggested_person_id.as_deref() else {
            log::warn!(
                "diarization: auto-label of {} has no person to link; leaving it as a suggestion",
                s.speaker_key
            );
            continue;
        };
        match PeopleRepository::assign_speaker_to_person(
            pool,
            meeting_id,
            &s.speaker_key,
            person_id,
        )
        .await
        {
            Ok(true) => {
                applied += 1;
                log::info!(
                    "diarization: auto-labeled {} as {} (person {person_id}) in meeting {meeting_id}",
                    s.speaker_key,
                    s.suggested_name
                );
            }
            Ok(false) => log::warn!(
                "diarization: auto-label of {} found no speaker/person row to link",
                s.speaker_key
            ),
            Err(e) => log::warn!(
                "diarization: auto-label of {} failed (continuing): {e}",
                s.speaker_key
            ),
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::MeetingsRepository;
    use sqlx::sqlite::SqlitePoolOptions;
    use uuid::Uuid;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn insert_speaker(pool: &SqlitePool, meeting_id: &str, key: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("speaker-{}", Uuid::new_v4()))
        .bind(meeting_id)
        .bind(key)
        .bind("Speaker")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("insert speaker");
    }

    fn suggestion(key: &str, person_id: Option<&str>, auto: bool) -> SpeakerSuggestion {
        SpeakerSuggestion {
            speaker_key: key.to_string(),
            suggested_name: "Priya".to_string(),
            suggested_email: None,
            suggested_person_id: person_id.map(str::to_string),
            confidence: 0.91,
            basis: "recognized Priya's well-trained voiceprint".to_string(),
            auto_label: auto,
        }
    }

    /// The whole point of the module: an `auto_label` match is persisted like a
    /// confirmation, and everything else is left for the user to confirm.
    #[tokio::test]
    async fn apply_persists_auto_labels_and_leaves_the_rest_alone() {
        let pool = test_pool().await;
        let meeting =
            MeetingsRepository::create_meeting(&pool, Some("M".into()), None, None, None, None)
                .await
                .unwrap();
        insert_speaker(&pool, &meeting, "spk_1").await;
        insert_speaker(&pool, &meeting, "spk_2").await;
        let priya = crate::database::repositories::people::PeopleRepository::create(
            &pool, "Priya", None, None, None,
        )
        .await
        .unwrap();

        let applied = apply(
            &pool,
            &meeting,
            &[
                suggestion("spk_1", Some(&priya.id), true),
                suggestion("spk_2", Some(&priya.id), false),
            ],
        )
        .await;
        assert_eq!(applied, 1);

        let speakers = SpeakersRepository::get_by_meeting(&pool, &meeting)
            .await
            .unwrap();
        let spk_1 = speakers.iter().find(|s| s.speaker_key == "spk_1").unwrap();
        assert_eq!(spk_1.person_id.as_deref(), Some(priya.id.as_str()));
        let spk_2 = speakers.iter().find(|s| s.speaker_key == "spk_2").unwrap();
        assert_eq!(
            spk_2.person_id, None,
            "a confirm-first suggestion must never be applied"
        );
    }

    /// No durable person to link to → nothing to apply, and the rest of the batch survives.
    #[tokio::test]
    async fn apply_skips_a_personless_auto_label_without_failing_the_batch() {
        let pool = test_pool().await;
        let meeting =
            MeetingsRepository::create_meeting(&pool, Some("M".into()), None, None, None, None)
                .await
                .unwrap();
        insert_speaker(&pool, &meeting, "spk_1").await;
        insert_speaker(&pool, &meeting, "spk_2").await;
        let priya = crate::database::repositories::people::PeopleRepository::create(
            &pool, "Priya", None, None, None,
        )
        .await
        .unwrap();

        let applied = apply(
            &pool,
            &meeting,
            &[
                suggestion("spk_1", None, true),
                suggestion("spk_2", Some(&priya.id), true),
            ],
        )
        .await;
        assert_eq!(
            applied, 1,
            "the personless one is skipped, the other applies"
        );
    }

    #[test]
    fn prior_meeting_tier_needs_both_confidence_and_repetition() {
        assert!(
            !prior_meeting_auto(TAU_VOICE_AUTO - 0.01, 5),
            "below the acoustic bar stays a suggestion however often it is seen"
        );
        assert!(
            !prior_meeting_auto(0.99, AUTO_PRIOR_MEETINGS_MIN - 1),
            "a near-perfect one-off match still stays a suggestion"
        );
        assert!(prior_meeting_auto(TAU_VOICE_AUTO, AUTO_PRIOR_MEETINGS_MIN));
        assert!(prior_meeting_auto(0.93, AUTO_PRIOR_MEETINGS_MIN + 2));
    }

    #[test]
    fn the_prior_tier_is_never_looser_than_the_gallery_voice_tier() {
        // specs/0064 W2 — dropping the gallery requirement must not lower the acoustic bar;
        // the corroboration moves from "trained centroid" to "seen in N meetings".
        assert!(!prior_meeting_auto(
            TAU_VOICE_AUTO - f32::EPSILON,
            usize::MAX
        ));
    }

    /// The matcher re-derives the same auto-label on every open, so a second pass over an
    /// already-linked speaker must report nothing new — that count is the frontend's signal
    /// to reload, and a permanently non-zero one would make it reload forever.
    #[tokio::test]
    async fn apply_is_idempotent_across_repeat_fetches() {
        let pool = test_pool().await;
        let meeting =
            MeetingsRepository::create_meeting(&pool, Some("M".into()), None, None, None, None)
                .await
                .unwrap();
        insert_speaker(&pool, &meeting, "spk_1").await;
        let priya = crate::database::repositories::people::PeopleRepository::create(
            &pool, "Priya", None, None, None,
        )
        .await
        .unwrap();
        let batch = [suggestion("spk_1", Some(&priya.id), true)];

        assert_eq!(apply(&pool, &meeting, &batch).await, 1);
        assert_eq!(
            apply(&pool, &meeting, &batch).await,
            0,
            "the same auto-label applied twice is not a second change"
        );
    }
}
