//! Enroll-on-confirm: opportunistically grow the voiceprint gallery when a diarized
//! cluster is confirmed to a Person (specs/0016 1c, ADR-0007 §3/§4).
//!
//! There is no explicit "record 10s" enrollment product (spec non-goal). Instead, every
//! time the user confirms an identity — assigning a detected speaker to a Person
//! (`api_assign_speaker_to_person`) or to a calendar attendee
//! (`api_assign_speaker_to_attendee`) — we insert ONE voiceprint sample from that
//! speaker row's already-stored embedding, **behind a two-layer consent gate**:
//!
//! - **Owner ("You") / local channel:** gated on `self_enroll_voiceprint` (ADR-0007 §3,
//!   on by default). The owner is represented by a singleton reserved `people` row
//!   ([`OWNER_PERSON_ID`]), created lazily on first self-enroll.
//! - **Everyone else:** gated on `store_others_voiceprints && !person.voiceprint_opt_out`
//!   (ADR-0007 §2). With the global toggle off, or that person opted out, identity is
//!   still associated but NO voiceprint is stored.
//!
//! Gating off is a silent, expected no-op — identity association always succeeds; only the
//! biometric storage is suppressed. Every function here is best-effort from the caller's
//! perspective: an enroll failure must never fail the (already-committed) identity
//! assignment, so callers log-and-continue.

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::database::repositories::people::PeopleRepository;
use crate::database::repositories::speaker::SpeakersRepository;
use crate::database::repositories::voiceprints::VoiceprintsRepository;
use crate::diarization::embedding::{embedding_from_bytes, EMBEDDING_MODEL_ID};

/// Reserved id of the singleton "You" person — the device owner (mic channel). Created
/// lazily on first self-enroll (ADR-0007 §3). A fixed, well-known id (rather than a
/// flag column) keeps the owner identifiable across the app without a migration: any
/// code that needs "is this the owner?" compares against this constant.
pub const OWNER_PERSON_ID: &str = "person-owner-self";

/// Display name for the owner person row.
const OWNER_DISPLAY_NAME: &str = "You";

/// The outcome of the pure two-layer consent gate (ADR-0007 §2/§3), separated from I/O so
/// it is unit-testable without disk settings or a DB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollDecision {
    /// Enroll under the singleton owner ("You") person — the local/mic channel, gated on
    /// `self_enroll_voiceprint`.
    EnrollOwner,
    /// Enroll under the given non-owner person — gated on
    /// `store_others_voiceprints && !voiceprint_opt_out`.
    EnrollPerson,
    /// Consent gate says no — associate identity but store NO voiceprint.
    Skip,
}

/// How much we trust the *attribution* behind an enrollment (specs/0039 WS3 pollution
/// guard). Orthogonal to the consent gate: consent asks "is the user willing to store this
/// person's voice?"; confidence asks "is this cluster actually that person?". A bad
/// attribution enrolled into the gallery poisons future auto-labeling, so a low-confidence
/// attribution must never enroll no matter the consent settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollConfidence {
    /// An explicit human confirmation — `api_assign_speaker_to_person` /
    /// `api_assign_speaker_to_attendee`. Highest trust; enrolls subject to consent.
    UserConfirmed,
    /// An automatic gallery auto-label (the offline pass, `pipeline.rs:~823`). It must
    /// **NEVER** enroll — that's the flywheel-in-reverse WS3 exists to prevent. The offline
    /// pass avoids enrollment today only by calling `assign_speaker_to_person` directly
    /// (never this enroll path); routing it here with this variant keeps that a hard,
    /// tested invariant even if a future refactor changes the call site.
    AutoLabel,
}

/// Pure consent + confidence gate (ADR-0007 §2/§3, specs/0039 WS3). `confidence` is the
/// attribution-trust layer: an [`EnrollConfidence::AutoLabel`] attribution always [`Skip`]s
/// regardless of consent (a low-confidence auto-label must never grow the gallery). `is_local`
/// marks the device-owner/mic channel; `store_others`/`self_enroll` are the two global toggles;
/// `person_opt_out` is the matched non-owner person's per-person flag (ignored for the owner
/// path).
///
/// - Auto-label: always skip.
/// - Owner (local): enroll iff `self_enroll`.
/// - Others: enroll iff `store_others && !person_opt_out`.
///
/// [`Skip`]: EnrollDecision::Skip
pub fn decide_enrollment(
    confidence: EnrollConfidence,
    is_local: bool,
    store_others: bool,
    self_enroll: bool,
    person_opt_out: bool,
) -> EnrollDecision {
    // WS3 invariant: an automatic auto-label never enrolls, whatever the consent toggles say.
    if confidence == EnrollConfidence::AutoLabel {
        return EnrollDecision::Skip;
    }
    if is_local {
        if self_enroll {
            EnrollDecision::EnrollOwner
        } else {
            EnrollDecision::Skip
        }
    } else if store_others && !person_opt_out {
        EnrollDecision::EnrollPerson
    } else {
        EnrollDecision::Skip
    }
}

/// Ensure the singleton owner ("You") `people` row exists, returning its id
/// ([`OWNER_PERSON_ID`]). Idempotent: inserts the reserved row only if absent, so it can
/// be called on every self-enroll. The owner has no email by default (the mic channel is
/// identified structurally, not by calendar).
pub async fn ensure_owner_person(pool: &SqlitePool) -> Result<String, sqlx::Error> {
    if PeopleRepository::get(pool, OWNER_PERSON_ID)
        .await?
        .is_some()
    {
        return Ok(OWNER_PERSON_ID.to_string());
    }
    let now = chrono::Utc::now().to_rfc3339();
    // Insert the reserved-id row directly (PeopleRepository::create mints a random uuid;
    // we need the stable reserved id). `INSERT OR IGNORE` makes a race a no-op.
    sqlx::query(
        "INSERT OR IGNORE INTO people
            (id, email, display_name, role, notes, voiceprint_opt_out, created_at, updated_at)
         VALUES (?, NULL, ?, NULL, NULL, 0, ?, ?)",
    )
    .bind(OWNER_PERSON_ID)
    .bind(OWNER_DISPLAY_NAME)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(OWNER_PERSON_ID.to_string())
}

/// Enroll a confirmed speaker's voiceprint into a Person's gallery, behind the two-layer
/// consent gate (ADR-0007 §2/§3).
///
/// Reads the speaker row's stored embedding; if it's the local/owner channel, gates on
/// `self_enroll_voiceprint` and enrolls under the singleton owner person (creating it
/// lazily, ignoring the passed `person_id`); otherwise gates on
/// `store_others_voiceprints && !person.voiceprint_opt_out` and enrolls under
/// `person_id`. Returns `Ok(true)` iff a voiceprint row was actually written, `Ok(false)`
/// when correctly gated off or there was nothing to enroll (no embedding). Never panics;
/// callers treat a returned `Err` as best-effort and log-and-continue.
pub async fn enroll_voiceprint_for_speaker(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
    person_id: &str,
    confidence: EnrollConfidence,
) -> Result<bool> {
    // WS3 cluster-quality guards (specs/0039), before any embedding read:
    // - The `unknown` overflow bucket is a MIX of voices and carries no centroid
    //   (sherpa.rs UNKNOWN_SPEAKER_KEY) — never enroll it. (get_speaker_embedding would also
    //   return None, but refuse explicitly so a future change can't start enrolling it.)
    if speaker_key == crate::diarization::UNKNOWN_SPEAKER_KEY {
        return Ok(false);
    }
    // - A cluster is refused only when MATERIALLY contested (specs/0039 WS3): a fraction of
    //   its current lines at or above `MATERIAL_CONTEST_FRACTION` carry a manual override, so
    //   its membership was substantially hand-edited and it is no longer a clean voiceprint
    //   source. A single stray correction on an otherwise-clean cluster (a tiny fraction)
    //   still enrolls — the earlier ANY-override gate over-blocked, refusing a whole 200-line
    //   cluster because one line was corrected. Best-effort: a read error yields 0.0 (enroll).
    use crate::database::repositories::transcript_speaker_overrides::{
        TranscriptSpeakerOverridesRepository, MATERIAL_CONTEST_FRACTION,
    };
    let contested = TranscriptSpeakerOverridesRepository::override_fraction_for_speaker(
        pool,
        meeting_id,
        speaker_key,
    )
    .await
    .unwrap_or(0.0);
    if contested >= MATERIAL_CONTEST_FRACTION {
        return Ok(false);
    }

    // The speaker's stored voiceprint + whether it's the local (owner) channel.
    let (is_local, bytes, model) =
        match SpeakersRepository::get_speaker_embedding(pool, meeting_id, speaker_key)
            .await
            .context("load speaker embedding for enrollment")?
        {
            Some(t) => t,
            // No row or no embedding (offline owner row, too-short cluster) → nothing to do.
            None => return Ok(false),
        };

    let settings = crate::diarization::settings::load_settings().await;
    let model = model.unwrap_or_else(|| EMBEDDING_MODEL_ID.to_string());

    // Decode + sanity-check the embedding before any gate work (a corrupt blob enrolls
    // nothing rather than poisoning the gallery).
    let embedding = match embedding_from_bytes(&bytes) {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(false),
    };

    // Resolve the target person + consent gate. For the non-owner path we must know the
    // person's opt-out flag first (a missing person → no enrollment).
    let person_opt_out = if is_local {
        false // owner path ignores this
    } else {
        match PeopleRepository::get(pool, person_id)
            .await
            .context("load person for enrollment gate")?
        {
            Some(p) => p.voiceprint_opt_out,
            None => return Ok(false), // person missing → nothing to enroll under
        }
    };

    let target_person_id = match decide_enrollment(
        confidence,
        is_local,
        settings.store_others_voiceprints,
        settings.self_enroll_voiceprint,
        person_opt_out,
    ) {
        EnrollDecision::Skip => return Ok(false), // gated off — identity already associated.
        EnrollDecision::EnrollOwner => ensure_owner_person(pool)
            .await
            .context("ensure owner person")?,
        EnrollDecision::EnrollPerson => person_id.to_string(),
    };

    VoiceprintsRepository::add_sample(
        pool,
        &target_person_id,
        &embedding,
        &model,
        Some(meeting_id),
        // specs/0039 WS3: back-link the sample to the cluster it came from so a later span
        // correction can retract exactly this sample.
        Some(speaker_key),
        // sample_quality: we don't have a per-cluster quality score plumbed yet; leave
        // NULL so best-N falls back to recency. (Quality tagging is a tuning follow-on.)
        None,
    )
    .await
    .context("insert voiceprint sample")?;

    Ok(true)
}

/// Enroll a confirmed speaker's voiceprint under the singleton OWNER ("You"), gated on
/// `self_enroll_voiceprint` (specs/0018). Unlike [`enroll_voiceprint_for_speaker`] — which
/// routes by the speaker row's structural `is_local` flag — this forces the OWNER
/// self-enroll path regardless of the channel. It exists for the "this attendee is me"
/// case (`api_assign_speaker_to_attendee` owner branch): the speaker is a *remote* cluster
/// (`is_local = 0`) that the user has declared to be their own voice (e.g. joined under an
/// alias on another device), so it must be gated like the owner, not like "others".
///
/// Returns `Ok(true)` iff a voiceprint row was written; `Ok(false)` when gated off
/// (`self_enroll` disabled) or there is nothing to enroll. Best-effort — callers
/// log-and-continue.
pub async fn enroll_owner_voiceprint_for_speaker(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
) -> Result<bool> {
    let settings = crate::diarization::settings::load_settings().await;
    if !settings.self_enroll_voiceprint {
        return Ok(false); // owner self-enroll gated off — identity already linked.
    }

    let (_, bytes, model) =
        match SpeakersRepository::get_speaker_embedding(pool, meeting_id, speaker_key)
            .await
            .context("load speaker embedding for owner enrollment")?
        {
            Some(t) => t,
            None => return Ok(false), // no embedding → nothing to enroll.
        };
    let model = model.unwrap_or_else(|| EMBEDDING_MODEL_ID.to_string());

    let embedding = match embedding_from_bytes(&bytes) {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(false),
    };

    let owner_id = ensure_owner_person(pool)
        .await
        .context("ensure owner person")?;
    VoiceprintsRepository::add_sample(
        pool,
        &owner_id,
        &embedding,
        &model,
        Some(meeting_id),
        Some(speaker_key),
        None,
    )
    .await
    .context("insert owner voiceprint sample")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::people::PeopleRepository;
    use sqlx::SqlitePool;

    // ----- pure consent-gate matrix (ADR-0007 §2/§3) -----

    #[test]
    fn owner_enrolls_only_when_self_enroll_on() {
        use EnrollConfidence::UserConfirmed;
        // self_enroll on → owner enrolls regardless of the others-toggle / opt-out.
        assert_eq!(
            decide_enrollment(UserConfirmed, true, false, true, true),
            EnrollDecision::EnrollOwner
        );
        // self_enroll off → owner is skipped.
        assert_eq!(
            decide_enrollment(UserConfirmed, true, true, false, false),
            EnrollDecision::Skip
        );
    }

    #[test]
    fn others_enroll_only_when_global_on_and_not_opted_out() {
        use EnrollConfidence::UserConfirmed;
        // Global on + not opted out → enroll.
        assert_eq!(
            decide_enrollment(UserConfirmed, false, true, true, false),
            EnrollDecision::EnrollPerson
        );
        // Global OFF → skip (the off-by-default consent posture).
        assert_eq!(
            decide_enrollment(UserConfirmed, false, false, true, false),
            EnrollDecision::Skip
        );
        // Global on but this person OPTED OUT → skip (per-person override wins).
        assert_eq!(
            decide_enrollment(UserConfirmed, false, true, true, true),
            EnrollDecision::Skip
        );
    }

    /// specs/0039 WS3: an auto-label attribution NEVER enrolls, no matter how permissive the
    /// consent toggles are — the guard against the pollution flywheel. Mirror of the two
    /// consent-matrix rows above but with `AutoLabel`, all of which must collapse to `Skip`.
    #[test]
    fn auto_label_never_enrolls_regardless_of_consent() {
        use EnrollConfidence::AutoLabel;
        // Owner path, self_enroll on — would enroll if user-confirmed; auto-label → skip.
        assert_eq!(
            decide_enrollment(AutoLabel, true, false, true, true),
            EnrollDecision::Skip
        );
        // Others path, global on + not opted out — would enroll if user-confirmed; skip.
        assert_eq!(
            decide_enrollment(AutoLabel, false, true, true, false),
            EnrollDecision::Skip
        );
    }

    // ----- integration: opt-out deletes existing samples (ADR-0007 §6) -----

    /// In-memory pool through the app's REAL migration set (the previous hand-rolled DDL
    /// had to be kept in sync with four migrations by hand and silently drifted). One
    /// connection max — each in-memory connection is a separate database. `foreign_keys`
    /// is ON by sqlx default, matching the app pool (manager.rs); the explicit cascades
    /// exist because most cross-table links are documentation-only, not declared FKs.
    async fn pool_with_schema() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn opt_out_deletes_existing_voiceprints_but_keeps_person() {
        let pool = pool_with_schema().await;
        let person =
            PeopleRepository::create(&pool, "Priya", Some("priya@example.com"), None, None)
                .await
                .unwrap();
        VoiceprintsRepository::add_sample(
            &pool,
            &person.id,
            &[1.0, 0.0],
            "3dspeaker_campplus_sv_en_voxceleb_16k",
            None,
            None,
            Some(1.0),
        )
        .await
        .unwrap();
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, &person.id)
                .await
                .unwrap(),
            1
        );

        // Turn on per-person opt-out → samples deleted, person survives.
        PeopleRepository::set_voiceprint_opt_out(&pool, &person.id, true)
            .await
            .unwrap();
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, &person.id)
                .await
                .unwrap(),
            0,
            "opt-out must delete existing voiceprints"
        );
        assert!(
            PeopleRepository::get(&pool, &person.id)
                .await
                .unwrap()
                .is_some(),
            "the person row must survive opt-out"
        );

        // Flipping opt-out back OFF does NOT recreate samples.
        PeopleRepository::set_voiceprint_opt_out(&pool, &person.id, false)
            .await
            .unwrap();
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, &person.id)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn forget_person_cascades_voiceprints() {
        let pool = pool_with_schema().await;
        let person = PeopleRepository::create(&pool, "Sam", None, None, None)
            .await
            .unwrap();
        VoiceprintsRepository::add_sample(
            &pool,
            &person.id,
            &[0.0, 1.0],
            "3dspeaker_campplus_sv_en_voxceleb_16k",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(PeopleRepository::delete(&pool, &person.id).await.unwrap());
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, &person.id)
                .await
                .unwrap(),
            0,
            "forget-person must cascade-delete voiceprints"
        );
        assert!(PeopleRepository::get(&pool, &person.id)
            .await
            .unwrap()
            .is_none());
    }

    // ----- specs/0039 WS3: enroll cluster-quality guards -----

    /// The `unknown` overflow bucket is a MIX of voices and carries no centroid; the enroll
    /// gate must refuse it outright (returns before any embedding/settings work), writing NO
    /// voiceprint row. Guards the mixed bucket from ever polluting a gallery.
    #[tokio::test]
    async fn unknown_bucket_never_enrolls() {
        let pool = pool_with_schema().await;
        let enrolled = enroll_voiceprint_for_speaker(
            &pool,
            "m1",
            crate::diarization::UNKNOWN_SPEAKER_KEY,
            "p-anything",
            EnrollConfidence::UserConfirmed,
        )
        .await
        .unwrap();
        assert!(!enrolled, "the unknown bucket must never enroll");
    }

    // --- shared setup for the "materially contested" enroll-gate tests (specs/0039 WS3) ---

    async fn insert_meeting(pool: &SqlitePool, id: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind("t")
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
    }

    /// `n` transcript lines `t0..tn` for a meeting under `speaker`.
    async fn insert_lines(pool: &SqlitePool, meeting: &str, n: usize, speaker: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        for i in 0..n {
            sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(format!("t{i}"))
            .bind(meeting)
            .bind("hello")
            .bind(&now)
            .bind(speaker)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    /// A local/mic speaker row carrying a real embedding — the owner-path enroll source.
    async fn insert_local_speaker_with_embedding(pool: &SqlitePool, meeting: &str, key: &str) {
        use crate::database::repositories::speaker::SpeakersRepository;
        use crate::diarization::embedding::{embedding_to_bytes, l2_normalize};
        let emb = embedding_to_bytes(&l2_normalize(&[1.0, 0.0, 0.0]));
        SpeakersRepository::upsert(
            pool,
            meeting,
            key,
            "You",
            true, // is_local → owner self-enroll path
            Some(&emb),
            Some(3),
            Some(EMBEDDING_MODEL_ID),
        )
        .await
        .unwrap();
    }

    /// A single stray override on an otherwise-clean cluster (1/4 = 0.25 < MATERIAL) must NOT
    /// block enrollment — the regression the ANY-override gate broke. Routed through the
    /// owner/local path (self-enroll on by default) so the enroll actually fires.
    #[tokio::test]
    async fn single_stray_override_still_enrolls() {
        let pool = pool_with_schema().await;
        insert_meeting(&pool, "m1").await;
        insert_local_speaker_with_embedding(&pool, "m1", crate::diarization::LOCAL_SPEAKER_KEY)
            .await;
        insert_lines(&pool, "m1", 4, crate::diarization::LOCAL_SPEAKER_KEY).await;
        // One stray correction on a single line of the 4-line cluster → 0.25 contested.
        crate::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository
            ::set(&pool, "m1", "t0", crate::diarization::LOCAL_SPEAKER_KEY)
            .await
            .unwrap();

        let enrolled = enroll_voiceprint_for_speaker(
            &pool,
            "m1",
            crate::diarization::LOCAL_SPEAKER_KEY,
            "ignored-for-owner-path",
            EnrollConfidence::UserConfirmed,
        )
        .await
        .unwrap();
        assert!(
            enrolled,
            "a single stray override must not block a clean cluster's enrollment"
        );
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, OWNER_PERSON_ID)
                .await
                .unwrap(),
            1,
            "the owner gained exactly one sample"
        );
    }

    /// A MATERIALLY contested cluster (half its lines overridden, 2/4 = 0.5 >= MATERIAL) is
    /// refused — its membership was substantially hand-edited, so it is not a clean source.
    /// This gate returns before any settings/embedding work, so it is deterministic.
    #[tokio::test]
    async fn materially_contested_cluster_refuses_enrollment() {
        let pool = pool_with_schema().await;
        insert_meeting(&pool, "m1").await;
        insert_local_speaker_with_embedding(&pool, "m1", crate::diarization::LOCAL_SPEAKER_KEY)
            .await;
        insert_lines(&pool, "m1", 4, crate::diarization::LOCAL_SPEAKER_KEY).await;
        // Override HALF the cluster's lines → materially contested.
        for id in ["t0", "t1"] {
            crate::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository
                ::set(&pool, "m1", id, crate::diarization::LOCAL_SPEAKER_KEY)
                .await
                .unwrap();
        }

        let enrolled = enroll_voiceprint_for_speaker(
            &pool,
            "m1",
            crate::diarization::LOCAL_SPEAKER_KEY,
            "ignored",
            EnrollConfidence::UserConfirmed,
        )
        .await
        .unwrap();
        assert!(!enrolled, "a materially-contested cluster must not enroll");
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, OWNER_PERSON_ID)
                .await
                .unwrap(),
            0,
            "no sample written when refused"
        );
    }
}
