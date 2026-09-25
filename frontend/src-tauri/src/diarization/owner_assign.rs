//! Assigning a speaker to yourself (specs/0078 follow-up).
//!
//! "You" is keyed on `local` everywhere downstream: the "You" colour, talk time, "This
//! isn't me", the owner hint. In a call that key is the mic channel by construction, so
//! linking a remote cluster to the owner person is only an identity statement. In a room
//! (or hybrid) recording the owner is one of the clusters, and a cluster linked to the
//! owner person but still keyed `spk_N` is the owner treated as a stranger.
//!
//! So in a room/hybrid meeting, every route that says "this cluster is me" runs the same
//! core as "This is me" ([`room_commands::mark_speaker_as_me`]): re-key to `local`,
//! `owner_label = 'confirmed'`, one owner enrollment under the voiceprint consent. The
//! routes are assigning the cluster to the owner person (or to a person whose address is
//! one of the owner's) and assigning it to an attendee whose address is the owner's. In a
//! call, and before any pass has recorded a setup, those routes behave as before.
//!
//! [`convert_owner_links`] repairs meetings corrected before this existed: a room/hybrid
//! meeting's clusters linked to the owner person are re-keyed to `local` on the next read
//! (and after a re-run), without a new enrollment.
//!
//! The two assign commands' bodies live here as pool-level cores (the commands are thin),
//! so the routing is testable without an `AppHandle`.

use anyhow::{anyhow, Context, Result};
use sqlx::SqlitePool;

use crate::database::repositories::meeting_audio_setup::MeetingAudioSetupRepository;
use crate::database::repositories::meeting_participant::MeetingParticipantsRepository;
use crate::database::repositories::owner_emails::OwnerEmailsRepository;
use crate::database::repositories::people::PeopleRepository;
use crate::database::repositories::speaker::SpeakersRepository;
use crate::diarization::room_commands::{self, MarkOutcome};
use crate::diarization::{LOCAL_SPEAKER_KEY, UNKNOWN_SPEAKER_KEY};
use crate::people::enroll::{self, EnrollConfidence, OWNER_PERSON_ID};

/// Whether this meeting's last pass clustered the owner (room or hybrid). `false` for a
/// call, for a meeting no pass has run on, and on a read error.
pub(crate) async fn owner_is_clustered_here(pool: &SqlitePool, meeting_id: &str) -> bool {
    matches!(
        MeetingAudioSetupRepository::get(pool, meeting_id).await,
        Ok(Some(s)) if s.resolved.is_some_and(|r| r.owner_is_clustered())
    )
}

/// Whether `person_id` is the owner: the owner person itself, or a person whose email is
/// one of the owner's (declaring an address folds an existing person into the owner, but
/// a person created or edited afterwards can still carry one).
pub(crate) async fn is_owner_person(pool: &SqlitePool, person_id: &str) -> Result<bool> {
    if person_id == OWNER_PERSON_ID {
        return Ok(true);
    }
    let email = PeopleRepository::get(pool, person_id)
        .await
        .context("Couldn't read that person")?
        .and_then(|p| p.email);
    match email.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        Some(email) => OwnerEmailsRepository::contains(pool, email)
            .await
            .context("Couldn't read your email addresses"),
        None => Ok(false),
    }
}

/// When `speaker_key` is a cluster (not `local`, not `unknown`) of a room/hybrid meeting,
/// run "This is me" on it and return what it did. `Ok(None)` means the caller should take
/// its ordinary path.
async fn claim_if_room(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
    store_voiceprints: bool,
) -> Result<Option<MarkOutcome>> {
    let key = speaker_key.trim();
    if key == LOCAL_SPEAKER_KEY
        || key == UNKNOWN_SPEAKER_KEY
        || !owner_is_clustered_here(pool, meeting_id).await
    {
        return Ok(None);
    }
    room_commands::mark_speaker_as_me(pool, meeting_id, key, store_voiceprints)
        .await
        .map(Some)
}

/// `api_assign_speaker_to_person` at the pool level, with the voiceprint consent passed in.
///
/// Assigning a room/hybrid cluster to the owner is "This is me" (see the module docs);
/// otherwise: link the person, add them to the roster, enroll under the consent gate.
pub(crate) async fn assign_speaker_to_person(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
    person_id: &str,
    store_voiceprints: bool,
) -> Result<()> {
    if meeting_id.trim().is_empty() || speaker_key.trim().is_empty() {
        return Err(anyhow!("meeting and speaker are required"));
    }
    if person_id.trim().is_empty() {
        return Err(anyhow!("a person must be selected"));
    }

    if is_owner_person(pool, person_id).await.unwrap_or(false) {
        if let Some(out) = claim_if_room(pool, meeting_id, speaker_key, store_voiceprints).await? {
            log_claim(meeting_id, speaker_key, &out);
            return Ok(());
        }
    }

    let assigned =
        PeopleRepository::assign_speaker_to_person(pool, meeting_id, speaker_key, person_id)
            .await
            .map_err(|e| anyhow!("Failed to assign speaker to person: {e}"))?;
    if !assigned {
        return Err(anyhow!(
            "Couldn't link that speaker — the person or speaker no longer exists"
        ));
    }

    // specs/0038 WS6.c: identifying a speaker also puts that person on the meeting's
    // participant roster. The owner ("You") is implicit and never a roster row
    // (specs/0018); `add_identified` skips the owner person and dedupes on its PK.
    // Best-effort: the identity link is already committed.
    if let Err(e) = MeetingParticipantsRepository::add_identified(pool, meeting_id, person_id).await
    {
        log::warn!(
            "speaker-assign: roster add for person {person_id} in meeting {meeting_id} failed (continuing): {e}"
        );
    }

    // Enroll-on-confirm (specs/0016 1c, ADR-0007 §2/§3), best-effort. The owner path
    // covers the local/mic row AND a cluster assigned to the owner person (specs/0078
    // open question 6), under the one consent. An explicit human assignment is the
    // highest attribution trust (specs/0039 WS3); the cluster-quality guards still apply.
    match enroll::enroll_speaker_gated(
        pool,
        meeting_id,
        speaker_key,
        person_id,
        EnrollConfidence::UserConfirmed,
        false,
        store_voiceprints,
    )
    .await
    {
        Ok(true) => log::info!(
            "enrolled voiceprint for {speaker_key} in meeting {meeting_id} (person {person_id})"
        ),
        Ok(false) => {} // correctly gated off, or nothing to enroll — expected.
        Err(e) => log::warn!(
            "voiceprint enroll for {speaker_key} in meeting {meeting_id} failed (continuing): {e:#}"
        ),
    }
    Ok(())
}

/// `api_assign_speaker_to_attendee` at the pool level, with the voiceprint consent passed
/// in.
///
/// An attendee whose address is one of the owner's IS the owner (specs/0018): in a
/// room/hybrid meeting that is "This is me"; otherwise the speaker is linked to the owner
/// person and enrolled on the owner path. Any other attendee gets a durable person by
/// email, a roster row and an enrollment under the consent gate.
pub(crate) async fn assign_speaker_to_attendee(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
    display_name: &str,
    email: &str,
    store_voiceprints: bool,
) -> Result<()> {
    let display_name = display_name.trim();
    let email = email.trim();
    if display_name.is_empty() {
        return Err(anyhow!("display_name cannot be empty"));
    }
    if email.is_empty() {
        return Err(anyhow!("email cannot be empty"));
    }
    let is_owner_email = OwnerEmailsRepository::contains(pool, email)
        .await
        .unwrap_or(false);
    if is_owner_email {
        if let Some(out) = claim_if_room(pool, meeting_id, speaker_key, store_voiceprints).await? {
            log_claim(meeting_id, speaker_key, &out);
            return Ok(());
        }
    }

    let assigned =
        SpeakersRepository::assign_to_attendee(pool, meeting_id, speaker_key, display_name, email)
            .await
            .map_err(|e| anyhow!("Failed to assign speaker to attendee: {e}"))?;
    if !assigned {
        return Err(anyhow!(
            "No speaker '{speaker_key}' found for this meeting to assign"
        ));
    }

    // Enroll-on-confirm (specs/0016 1c): upsert a durable `people` row by email (the
    // cross-meeting anchor; the owner's singleton for an owner address), link the speaker,
    // roster it, then enroll behind the consent gate. All best-effort: a failure here must
    // not undo the (committed) attendee assignment.
    let person_result = if is_owner_email {
        match enroll::ensure_owner_person(pool).await {
            Ok(owner_id) => PeopleRepository::get(pool, &owner_id).await,
            Err(e) => Err(e),
        }
        .and_then(|opt| opt.ok_or_else(|| sqlx::Error::Protocol("owner person missing".into())))
    } else {
        PeopleRepository::create(pool, display_name, Some(email), None, None).await
    };
    let person = match person_result {
        Ok(p) => p,
        Err(e) => {
            log::warn!("attendee-assign: upsert person by email failed (continuing): {e}");
            return Ok(());
        }
    };
    if let Err(e) =
        PeopleRepository::assign_speaker_to_person(pool, meeting_id, speaker_key, &person.id).await
    {
        log::warn!("attendee-assign: link person failed (continuing): {e}");
    }
    // specs/0038 WS6.c: `add_identified` skips the owner and dedupes on its PK.
    if let Err(e) =
        MeetingParticipantsRepository::add_identified(pool, meeting_id, &person.id).await
    {
        log::warn!(
            "attendee-assign: roster add for {} in meeting {meeting_id} failed (continuing): {e}",
            person.id
        );
    }
    // An owner-email attendee forces the OWNER enroll path (a remote cluster the user
    // declared as their own voice); otherwise the usual structural `is_local` routing.
    match enroll::enroll_speaker_gated(
        pool,
        meeting_id,
        speaker_key,
        &person.id,
        EnrollConfidence::UserConfirmed,
        is_owner_email,
        store_voiceprints,
    )
    .await
    {
        Ok(true) => log::info!(
            "enrolled voiceprint for {speaker_key} in meeting {meeting_id} (attendee {email})"
        ),
        Ok(false) => {}
        Err(e) => log::warn!("attendee-assign voiceprint enroll failed (continuing): {e:#}"),
    }
    Ok(())
}

fn log_claim(meeting_id: &str, speaker_key: &str, out: &MarkOutcome) {
    log::info!(
        "assigned {speaker_key} to you in room meeting {meeting_id}: now \"You\" ({} lines, \
         merged={}, displaced={:?}, enrolled={})",
        out.rekey.moved_lines,
        out.rekey.merged_into_existing,
        out.rekey.displaced_to,
        out.enrolled
    );
}

/// Re-key every cluster of a room/hybrid meeting that is linked to the owner person onto
/// `local`, as "This is me" would have: `owner_label = 'confirmed'`, an automatic "You"
/// displaced, several owner-linked clusters merged into one "You". It writes NO voiceprint
/// sample: the owner assignment that linked each cluster already enrolled it (and those
/// samples' back-links follow the cluster to `local`).
///
/// A call meeting, or one no pass has run on, is left alone. Returns how many clusters
/// were re-keyed; `0` is the common, write-free case. Callers treat an error as
/// best-effort.
pub(crate) async fn convert_owner_links(pool: &SqlitePool, meeting_id: &str) -> Result<usize> {
    if !owner_is_clustered_here(pool, meeting_id).await {
        return Ok(0);
    }
    // The cluster with the most lines first: it becomes `local` (keeping its voice), and
    // the rest fold into it.
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT s.speaker_key FROM speakers s
         WHERE s.meeting_id = ?1 AND s.person_id = ?2 AND s.speaker_key NOT IN (?3, ?4)
         ORDER BY (SELECT COUNT(*) FROM transcripts t
                    WHERE t.meeting_id = ?1 AND t.speaker = s.speaker_key) DESC,
                  s.speaker_key ASC",
    )
    .bind(meeting_id)
    .bind(OWNER_PERSON_ID)
    .bind(LOCAL_SPEAKER_KEY)
    .bind(UNKNOWN_SPEAKER_KEY)
    .fetch_all(pool)
    .await
    .context("Couldn't read this meeting's speakers")?;
    let mut converted = 0;
    for key in &keys {
        // `Ok(None)`: a concurrent read already converted this one.
        if SpeakersRepository::claim_as_local(pool, meeting_id, key, OWNER_PERSON_ID, true)
            .await
            .with_context(|| format!("Couldn't make {key} \"You\""))?
            .is_some()
        {
            converted += 1;
        }
    }
    if converted > 0 {
        log::info!(
            "speakers: {converted} cluster(s) assigned to you are now \"You\" in room meeting \
             {meeting_id}"
        );
    }
    Ok(converted)
}

#[cfg(test)]
mod tests;
