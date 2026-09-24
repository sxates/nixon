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

use crate::database::repositories::meeting_participant::MeetingParticipantsRepository;
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

/// Whether this speaker row still carries the name diarization generated for it, or one the
/// user chose. Pure, so the rule is testable without a DB.
///
/// `display_name_for_key` is the generator: `local` → "You", `spk_N` → "Speaker N+1".
fn is_claimed_by_hand(row: &crate::database::models::SpeakerModel) -> bool {
    row.person_id.is_none()
        && row.display_name.trim()
            != crate::diarization::pipeline::display_name_for_key(&row.speaker_key)
}

/// Whether a NON-gallery (prior-speaker) match may be auto-applied: it must clear the same
/// acoustic bar as the voice-only gallery route AND have been seen in enough distinct prior
/// meetings. Pure, so the tier boundary is unit-testable without a DB.
pub fn prior_meeting_auto(confidence: f32, meeting_count: usize) -> bool {
    confidence >= TAU_VOICE_AUTO && meeting_count >= AUTO_PRIOR_MEETINGS_MIN
}

/// Persist every `auto_label` suggestion as a real identity assignment: the durable person
/// link plus the participant-roster row a user confirmation also creates. It deliberately
/// does NOT enroll a voiceprint — enrolment is consent-gated on a *user* confirmation
/// (ADR-0007 §2/§3), and an automatic label is not one.
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
    // The matcher re-derives the same auto-label on every refetch (a named speaker keeps its
    // embedding and keeps matching), so this runs against rows that may already be settled.
    // Two of them must be left alone — see `is_claimed_by_hand` and the already-linked check
    // below — and skipping makes a non-zero count mean "something actually changed".
    let rows = SpeakersRepository::get_by_meeting(pool, meeting_id)
        .await
        .unwrap_or_default();
    for s in suggestions.iter().filter(|s| s.auto_label) {
        let row = rows.iter().find(|row| row.speaker_key == s.speaker_key);
        if row
            .map(|r| r.person_id == s.suggested_person_id)
            .unwrap_or(false)
        {
            continue;
        }
        // NEVER overwrite a name the user typed. `SpeakersRepository::rename` sets only
        // `display_name`, leaving `person_id` NULL, so a hand-named speaker is indistinguishable
        // from an unnamed one by the person link alone — and auto-labeling it would replace the
        // user's word with the matcher's on the very next refresh, repeatedly. A row still
        // carrying its generated name (`Speaker 3`, `You`) is fair game; anything else is not.
        if let Some(row) = row {
            if is_claimed_by_hand(row) {
                log::debug!(
                    "diarization: leaving {} alone — it carries a name the user typed",
                    s.speaker_key
                );
                continue;
            }
        }
        let Some(person_id) = s.suggested_person_id.as_deref() else {
            log::warn!(
                "diarization: auto-label of {} has no person to link; leaving it as a suggestion",
                s.speaker_key
            );
            continue;
        };
        // specs/0078: the owner won a room-recording cluster. "You" is keyed on `local`
        // everywhere downstream, so the cluster is re-keyed rather than linked. Like every
        // automatic label, this never enrolls a voiceprint.
        if person_id == crate::people::enroll::OWNER_PERSON_ID {
            match SpeakersRepository::rekey_to_local(pool, meeting_id, &s.speaker_key, person_id)
                .await
            {
                Ok(Some(r)) => {
                    applied += 1;
                    log::info!(
                        "diarization: auto-labeled {} as the owner (\"You\", {} lines) in meeting {meeting_id}",
                        s.speaker_key,
                        r.moved_lines
                    );
                }
                Ok(None) => log::warn!(
                    "diarization: owner auto-label of {} found no speaker row",
                    s.speaker_key
                ),
                Err(e) => log::warn!(
                    "diarization: owner auto-label of {} failed (continuing): {e}",
                    s.speaker_key
                ),
            }
            continue;
        }
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
                // specs/0038 WS6.c — identifying a speaker also puts that person on the
                // meeting's participant roster, so a named speaker always shows up as a
                // participant. `api_assign_speaker_to_person` does this for a user
                // confirmation; an automatic label must not produce a half-identified
                // speaker that appears in the Speakers card but never on the roster.
                // `add_identified` skips the singleton owner and dedupes on its PK.
                if let Err(e) =
                    MeetingParticipantsRepository::add_identified(pool, meeting_id, person_id).await
                {
                    log::warn!(
                        "diarization: roster add for auto-labeled person {person_id} failed (continuing): {e}"
                    );
                }
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

    /// A freshly diarized speaker row: the display name is the one the pipeline generates,
    /// which is what makes it eligible for auto-labeling (see `is_claimed_by_hand`).
    async fn insert_speaker(pool: &SqlitePool, meeting_id: &str, key: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("speaker-{}", Uuid::new_v4()))
        .bind(meeting_id)
        .bind(key)
        .bind(crate::diarization::pipeline::display_name_for_key(key))
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

    /// specs/0078: when the owner wins a room cluster, the cluster BECOMES "You" (re-keyed
    /// to `local`), and no voiceprint sample is written for it.
    #[tokio::test]
    async fn an_owner_auto_label_rekeys_the_cluster_to_local_without_enrolling() {
        use crate::people::enroll::{ensure_owner_person, OWNER_PERSON_ID};
        let pool = test_pool().await;
        let meeting =
            MeetingsRepository::create_meeting(&pool, Some("M".into()), None, None, None, None)
                .await
                .unwrap();
        insert_speaker(&pool, &meeting, "spk_0").await;
        insert_speaker(&pool, &meeting, "spk_1").await;
        ensure_owner_person(&pool).await.unwrap();

        let applied = apply(
            &pool,
            &meeting,
            &[suggestion("spk_1", Some(OWNER_PERSON_ID), true)],
        )
        .await;
        assert_eq!(applied, 1);

        let rows = SpeakersRepository::get_by_meeting(&pool, &meeting)
            .await
            .unwrap();
        let keys: Vec<&str> = rows.iter().map(|r| r.speaker_key.as_str()).collect();
        assert!(
            keys.contains(&"local"),
            "the owner cluster is now local: {keys:?}"
        );
        assert!(!keys.contains(&"spk_1"), "{keys:?}");
        let samples: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voiceprints")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(samples, 0, "an automatic owner label never enrolls");
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

    /// specs/0064 W2 review — a name the user typed must survive every refresh. The rename
    /// path leaves `person_id` NULL, so without this guard the matcher would overwrite the
    /// user's word with its own on the very next open, over and over.
    #[tokio::test]
    async fn apply_never_overwrites_a_name_the_user_typed() {
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

        // The user renames the speaker by hand — display_name only, person_id still NULL.
        SpeakersRepository::rename(&pool, &meeting, "spk_1", "Mum")
            .await
            .unwrap();

        let applied = apply(
            &pool,
            &meeting,
            &[suggestion("spk_1", Some(&priya.id), true)],
        )
        .await;
        assert_eq!(applied, 0, "a hand-typed name is never replaced");

        let rows = SpeakersRepository::get_by_meeting(&pool, &meeting)
            .await
            .unwrap();
        let row = rows.iter().find(|r| r.speaker_key == "spk_1").unwrap();
        assert_eq!(row.display_name, "Mum");
        assert_eq!(row.person_id, None);
    }

    /// The generated name is not a claim, so an untouched speaker is still auto-namable.
    #[tokio::test]
    async fn apply_still_names_a_speaker_carrying_its_generated_name() {
        let pool = test_pool().await;
        let meeting =
            MeetingsRepository::create_meeting(&pool, Some("M".into()), None, None, None, None)
                .await
                .unwrap();
        insert_speaker(&pool, &meeting, "spk_1").await;
        SpeakersRepository::rename(&pool, &meeting, "spk_1", "Speaker 2")
            .await
            .unwrap();
        let priya = crate::database::repositories::people::PeopleRepository::create(
            &pool, "Priya", None, None, None,
        )
        .await
        .unwrap();

        assert_eq!(
            apply(
                &pool,
                &meeting,
                &[suggestion("spk_1", Some(&priya.id), true)]
            )
            .await,
            1
        );
    }

    /// specs/0038 WS6.c — an automatically named speaker joins the roster, exactly as one the
    /// user confirms does. Otherwise it shows in the Speakers card but nowhere else.
    #[tokio::test]
    async fn apply_puts_the_named_person_on_the_participant_roster() {
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

        apply(
            &pool,
            &meeting,
            &[suggestion("spk_1", Some(&priya.id), true)],
        )
        .await;

        let roster = crate::database::repositories::meeting_participant::MeetingParticipantsRepository::list(&pool, &meeting)
            .await
            .unwrap();
        assert!(
            roster.iter().any(|p| p.person_id == priya.id),
            "the auto-named person must appear on the roster"
        );
    }
}
