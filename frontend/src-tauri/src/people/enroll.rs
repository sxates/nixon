//! Enroll-on-confirm: opportunistically grow the voiceprint gallery when a diarized
//! cluster is confirmed to a Person (specs/0016 1c, ADR-0007 §3/§4).
//!
//! There is no explicit "record 10s" enrollment product (spec non-goal). Instead, every
//! time the user confirms an identity — assigning a detected speaker to a Person
//! (`api_assign_speaker_to_person`), to a calendar attendee
//! (`api_assign_speaker_to_attendee`), or marking a cluster "This is me"
//! (`api_mark_speaker_as_me`) — we insert ONE voiceprint sample from that speaker row's
//! already-stored embedding, **behind one consent gate** (specs/0078 owner decision 1):
//!
//! - **Everyone, the owner included:** gated on `store_voiceprints` (off by default).
//!   The owner ("You") is enrolled under a singleton reserved `people` row
//!   ([`OWNER_PERSON_ID`]), created lazily on first enroll. The owner path is taken for
//!   the local/mic row, for any speaker assigned to [`OWNER_PERSON_ID`], and for an
//!   attendee whose address is one of the owner's.
//! - **Everyone else** additionally honours the person's `voiceprint_opt_out`.
//!
//! Before specs/0078 the owner had a separate, on-by-default `self_enroll_voiceprint`
//! toggle; it is retired, and the owner's voice is treated like anyone else's.
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

/// The outcome of the pure consent gate (ADR-0007 §2/§3 as amended by specs/0078),
/// separated from I/O so it is unit-testable without disk settings or a DB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollDecision {
    /// Enroll under the singleton owner ("You") person.
    EnrollOwner,
    /// Enroll under the given non-owner person.
    EnrollPerson,
    /// The gate says no — associate identity but store NO voiceprint.
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
    /// `api_assign_speaker_to_attendee` / `api_mark_speaker_as_me`. Highest trust; enrolls
    /// subject to consent.
    UserConfirmed,
    /// Structural owner evidence, not an inference from the gallery (specs/0078 owner
    /// decision 2): the bleed-guarded mic turns of a call, or the single cluster of a room
    /// recording. Enrolls the OWNER only, subject to consent, at most once per meeting
    /// (see [`enroll_owner_sample_from_embedding`]). For anyone other than the owner it
    /// always skips.
    OwnerBootstrap,
    /// An automatic gallery auto-label (the offline pass, `pipeline.rs:~823`), including
    /// an automatic room-mode "You" found by owner voiceprint. It must **NEVER** enroll —
    /// that's the flywheel-in-reverse WS3 exists to prevent. The offline pass avoids
    /// enrollment today only by calling `assign_speaker_to_person` directly (never this
    /// enroll path); routing it here with this variant keeps that a hard, tested invariant
    /// even if a future refactor changes the call site.
    AutoLabel,
}

/// Pure consent + confidence gate (ADR-0007 §2/§3 as amended by specs/0078, specs/0039 WS3).
/// `is_owner` marks the device owner (the local/mic row, a speaker assigned to
/// [`OWNER_PERSON_ID`], or an owner-email attendee); `store_voiceprints` is the one global
/// consent; `person_opt_out` is the matched non-owner person's per-person flag (ignored for
/// the owner).
///
/// - Auto-label: always skip.
/// - Consent off: always skip, owner included.
/// - Owner: enroll.
/// - Others: enroll iff user-confirmed and not opted out.
pub fn decide_enrollment(
    confidence: EnrollConfidence,
    is_owner: bool,
    store_voiceprints: bool,
    person_opt_out: bool,
) -> EnrollDecision {
    // WS3 invariant: an automatic auto-label never enrolls, whatever the consent says.
    if confidence == EnrollConfidence::AutoLabel || !store_voiceprints {
        return EnrollDecision::Skip;
    }
    if is_owner {
        EnrollDecision::EnrollOwner
    } else if confidence == EnrollConfidence::OwnerBootstrap || person_opt_out {
        EnrollDecision::Skip
    } else {
        EnrollDecision::EnrollPerson
    }
}

/// The one voiceprint consent, read from the persisted diarization settings
/// (`store_voiceprints`, specs/0078 owner decision 1). Every enroll entry point resolves
/// the gate through this.
pub async fn voiceprint_consent() -> bool {
    crate::diarization::settings::load_settings()
        .await
        .store_voiceprints
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

/// Enroll a confirmed speaker's voiceprint into a Person's gallery, behind the consent
/// gate (ADR-0007 §2/§3 as amended by specs/0078).
///
/// Reads the speaker row's stored embedding. The owner path (the local/mic row, or
/// `person_id == OWNER_PERSON_ID`) enrolls under the singleton owner person, creating it
/// lazily; any other person enrolls under `person_id` unless they opted out. Both need
/// `store_voiceprints`. Returns `Ok(true)` iff a voiceprint row was actually written,
/// `Ok(false)` when correctly gated off or there was nothing to enroll (no embedding).
/// Never panics; callers treat a returned `Err` as best-effort and log-and-continue.
pub async fn enroll_voiceprint_for_speaker(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
    person_id: &str,
    confidence: EnrollConfidence,
) -> Result<bool> {
    let consent = voiceprint_consent().await;
    enroll_speaker_gated(
        pool,
        meeting_id,
        speaker_key,
        person_id,
        confidence,
        false,
        consent,
    )
    .await
}

/// Enroll a confirmed speaker's voiceprint under the singleton OWNER ("You") regardless of
/// the row's channel (specs/0018). Unlike [`enroll_voiceprint_for_speaker`] — which
/// routes by the speaker row's `is_local` flag and the target person — this always takes
/// the owner path. It exists for the "this attendee is me" case
/// (`api_assign_speaker_to_attendee` owner branch): the speaker is a *remote* cluster
/// (`is_local = 0`) that the user has declared to be their own voice.
///
/// Gated on `store_voiceprints` like every other enrollment (specs/0078), and subject to
/// the same WS3 cluster-quality guards. Returns `Ok(true)` iff a voiceprint row was
/// written. Best-effort — callers log-and-continue.
pub async fn enroll_owner_voiceprint_for_speaker(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
) -> Result<bool> {
    let consent = voiceprint_consent().await;
    enroll_speaker_gated(
        pool,
        meeting_id,
        speaker_key,
        OWNER_PERSON_ID,
        EnrollConfidence::UserConfirmed,
        true,
        consent,
    )
    .await
}

/// The enroll-from-a-speaker-row core with the consent passed in, so DB tests can pin it
/// without touching the settings file on disk. `force_owner` takes the owner path whatever
/// the row and `person_id` say.
pub(crate) async fn enroll_speaker_gated(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
    person_id: &str,
    confidence: EnrollConfidence,
    force_owner: bool,
    store_voiceprints: bool,
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
    let model = model.unwrap_or_else(|| EMBEDDING_MODEL_ID.to_string());

    // Decode + sanity-check the embedding before any gate work (a corrupt blob enrolls
    // nothing rather than poisoning the gallery).
    let embedding = match embedding_from_bytes(&bytes) {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(false),
    };

    // specs/0078 open question 6: a cluster assigned to the owner person is the owner's
    // voice, whatever its channel, so it takes the owner path.
    let is_owner = force_owner || is_local || person_id == OWNER_PERSON_ID;

    // For the non-owner path we must know the person's opt-out flag first (a missing
    // person → no enrollment).
    let person_opt_out = if is_owner {
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

    let target_person_id =
        match decide_enrollment(confidence, is_owner, store_voiceprints, person_opt_out) {
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

/// Add one owner ("You") voiceprint sample from an embedding computed by the diarization
/// pass (specs/0078 owner decision 2: the owner bootstrap). For the call-mode mic turns
/// and the room-mode single cluster, both of which are structural owner evidence.
///
/// - Gated on `store_voiceprints`, like every enrollment.
/// - `confidence` should be [`EnrollConfidence::OwnerBootstrap`] (or `UserConfirmed`);
///   `AutoLabel` never enrolls.
/// - **At most one sample per meeting:** it is back-linked to the meeting's `local`
///   speaker key, and nothing is written when any owner sample back-linked to
///   `(meeting_id, "local")` already exists, live or quarantined. A re-run of the pass
///   therefore never piles up samples, and a sample the user retracted with "This isn't
///   me" (quarantined) is not silently re-added.
/// - The back-link is what lets "This isn't me" (`api_unmark_speaker_as_me`) and "Clear
///   all voiceprints" find the sample again.
///
/// Returns `Ok(true)` iff a sample was written. Best-effort for the caller.
pub async fn enroll_owner_sample_from_embedding(
    pool: &SqlitePool,
    meeting_id: &str,
    embedding: &[f32],
    embedding_model: &str,
    confidence: EnrollConfidence,
) -> Result<bool> {
    let consent = voiceprint_consent().await;
    enroll_owner_sample_gated(
        pool,
        meeting_id,
        embedding,
        embedding_model,
        confidence,
        consent,
    )
    .await
}

/// [`enroll_owner_sample_from_embedding`] with the consent passed in (for DB tests).
pub(crate) async fn enroll_owner_sample_gated(
    pool: &SqlitePool,
    meeting_id: &str,
    embedding: &[f32],
    embedding_model: &str,
    confidence: EnrollConfidence,
    store_voiceprints: bool,
) -> Result<bool> {
    if decide_enrollment(confidence, true, store_voiceprints, false) != EnrollDecision::EnrollOwner
    {
        return Ok(false);
    }
    if embedding.is_empty() || embedding.iter().any(|x| !x.is_finite()) {
        return Ok(false);
    }
    let local = crate::diarization::LOCAL_SPEAKER_KEY;
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM voiceprints
                        WHERE person_id = ? AND source_meeting_id = ? AND source_speaker_key = ?)",
    )
    .bind(OWNER_PERSON_ID)
    .bind(meeting_id)
    .bind(local)
    .fetch_one(pool)
    .await
    .context("check for an existing owner sample from this meeting")?;
    if already {
        return Ok(false);
    }
    let owner_id = ensure_owner_person(pool)
        .await
        .context("ensure owner person")?;
    VoiceprintsRepository::add_sample(
        pool,
        &owner_id,
        embedding,
        embedding_model,
        Some(meeting_id),
        Some(local),
        None,
    )
    .await
    .context("insert owner voiceprint sample")?;
    Ok(true)
}

#[cfg(test)]
mod tests;
