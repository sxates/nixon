//! Tauri commands for room recordings (specs/0078).
//!
//! - [`api_get_meeting_audio_setup`] — the stored "Who was on the mic?" override plus the
//!   setup the last diarization pass used.
//! - [`api_set_meeting_audio_setup`] — store the override and re-run diarization.
//! - [`api_mark_speaker_as_me`] — "This is me": re-key a cluster to the owner's `local`
//!   key and enroll it under the one voiceprint consent.
//! - [`api_unmark_speaker_as_me`] — "This isn't me": the inverse, for a wrong "You". It
//!   sticks: the owner stops auto-labeling clusters in this meeting until "This is me".
//!
//! Split out of `diarization/commands.rs`, which is at the file-size cap. The mark/unmark
//! commands work in any meeting (call or room): they only move rows, and a later call-mode
//! pass rebuilds `local` from the mic anyway.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::database::repositories::meeting_audio_setup::MeetingAudioSetupRepository;
use crate::database::repositories::speaker::SpeakersRepository;
use crate::database::repositories::speaker_rekey::{RekeyFromLocal, RekeyToLocal};
use crate::diarization::commands::DiarizeStartDto;
use crate::diarization::launch;
use crate::diarization::room_types::AudioSetupOverride;
use crate::diarization::{LOCAL_SPEAKER_KEY, UNKNOWN_SPEAKER_KEY};
use crate::people::enroll::{self, EnrollConfidence, OWNER_PERSON_ID};
use crate::state::AppState;

/// A meeting's audio setup on the wire: `{ override: "auto"|"room"|"call",
/// resolved: "call"|"room"|"hybrid"|null }`.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MeetingAudioSetupDto {
    #[serde(rename = "override")]
    pub override_setup: &'static str,
    pub resolved: Option<&'static str>,
}

/// The stored override and the setup the last pass used.
#[tauri::command]
pub async fn api_get_meeting_audio_setup<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<MeetingAudioSetupDto, String> {
    let state = app.state::<AppState>();
    get_audio_setup(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Store the "Who was on the mic?" override (`"auto" | "room" | "call"`), then re-run
/// diarization the same way "Identify speakers" does. Returns that command's DTO. When a
/// pass is already running (it read the old setting), `started` is false, the caller
/// attaches to that pass, and one more pass is queued to start when it finishes
/// (`launch::diarize_meeting_or_queue`); its `diarization-progress` events flip the UI
/// back to running.
#[tauri::command]
pub async fn api_set_meeting_audio_setup<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    setup: String,
) -> Result<DiarizeStartDto, String> {
    {
        let state = app.state::<AppState>();
        set_audio_setup(state.db_manager.pool(), &meeting_id, &setup)
            .await
            .map_err(|e| format!("{e:#}"))?;
    }
    let started = launch::diarize_meeting_or_queue(app, meeting_id);
    Ok(DiarizeStartDto {
        started,
        already_running: !started,
    })
}

/// "This is me": the speaker becomes "You" in this meeting, and its voice is enrolled as
/// the owner's when "Store voiceprints" is on. The summary refreshes its names.
#[tauri::command]
pub async fn api_mark_speaker_as_me<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    speaker_key: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    let consent = enroll::voiceprint_consent().await;
    let outcome = mark_speaker_as_me(pool, &meeting_id, &speaker_key, consent)
        .await
        .map_err(|e| format!("{e:#}"))?;
    log::info!(
        "marked {speaker_key} as the owner in meeting {meeting_id}: {} lines, merged={}, \
         displaced={:?}, {} other samples quarantined, enrolled={}",
        outcome.rekey.moved_lines,
        outcome.rekey.merged_into_existing,
        outcome.rekey.displaced_to,
        outcome.rekey.quarantined_other_samples,
        outcome.enrolled
    );
    crate::summary::refresh::schedule_name_refresh(&app, pool.clone(), &meeting_id);
    Ok(())
}

/// "This isn't me": this meeting's "You" becomes the next "Speaker N", and any owner
/// voiceprint samples taken from it are quarantined. The summary refreshes its names.
#[tauri::command]
pub async fn api_unmark_speaker_as_me<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    let outcome = unmark_speaker_as_me(pool, &meeting_id)
        .await
        .map_err(|e| format!("{e:#}"))?;
    log::info!(
        "unmarked the owner in meeting {meeting_id}: now {} ({} lines, {} pinned lines kept), \
         {} owner samples quarantined",
        outcome.new_key,
        outcome.moved_lines,
        outcome.kept_lines,
        outcome.quarantined_owner_samples
    );
    crate::summary::refresh::schedule_name_refresh(&app, pool.clone(), &meeting_id);
    Ok(())
}

// ----- pool-level cores (the commands are thin; these are what the tests drive) -----

pub(crate) async fn get_audio_setup(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<MeetingAudioSetupDto> {
    let stored = MeetingAudioSetupRepository::get(pool, meeting_id)
        .await
        .context("Couldn't read this meeting's audio setup")?
        .ok_or_else(|| anyhow!("That meeting no longer exists"))?;
    Ok(MeetingAudioSetupDto {
        override_setup: stored.override_setup.as_str(),
        resolved: stored.resolved.map(|s| s.as_str()),
    })
}

pub(crate) async fn set_audio_setup(
    pool: &SqlitePool,
    meeting_id: &str,
    setup: &str,
) -> Result<()> {
    let parsed = AudioSetupOverride::parse(setup).ok_or_else(|| {
        anyhow!("Unknown audio setup {setup:?}; expected \"auto\", \"room\" or \"call\"")
    })?;
    let updated = MeetingAudioSetupRepository::set_override(pool, meeting_id, parsed)
        .await
        .context("Couldn't save this meeting's audio setup")?;
    if !updated {
        return Err(anyhow!("That meeting no longer exists"));
    }
    Ok(())
}

/// What "This is me" did.
#[derive(Debug)]
pub(crate) struct MarkOutcome {
    pub rekey: RekeyToLocal,
    /// Whether an owner voiceprint sample was written.
    pub enrolled: bool,
}

/// "This is me" with the voiceprint consent passed in. Also the core of assigning a room
/// cluster to yourself (`owner_assign`). The re-key is one transaction;
/// enrollment runs after it and is best-effort (a failure is logged, not returned), like
/// every enroll-on-confirm path.
///
/// The sample is the embedding of the cluster the user just confirmed, read before the
/// re-key: folded into an existing `local`, the cluster's row is gone and `local` may
/// still carry an automatic label's voice. Enrollment is skipped when this meeting
/// already has a LIVE owner sample back-linked to `local` (an earlier "This is me"
/// enrolled it).
pub(crate) async fn mark_speaker_as_me(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
    store_voiceprints: bool,
) -> Result<MarkOutcome> {
    let speaker_key = speaker_key.trim();
    if meeting_id.trim().is_empty() || speaker_key.is_empty() {
        return Err(anyhow!("A meeting and a speaker are required"));
    }
    if speaker_key == LOCAL_SPEAKER_KEY {
        return Err(anyhow!("That speaker is already you"));
    }
    if speaker_key == UNKNOWN_SPEAKER_KEY {
        return Err(anyhow!(
            "\"Unknown speaker\" mixes several voices, so it can't be marked as you. \
             Reassign its lines to \"You\" instead"
        ));
    }

    enroll::ensure_owner_person(pool)
        .await
        .context("Couldn't set up your own person record")?;
    let confirmed_voice = SpeakersRepository::get_speaker_embedding(pool, meeting_id, speaker_key)
        .await
        .context("Couldn't read that speaker's voice")?
        .map(|(_, bytes, model)| (bytes, model));
    // In a room/hybrid meeting an automatic "You" is a guess this corrects: it is moved
    // out rather than merged with (`claim_as_local`). In a call `local` is the mic.
    let displace =
        crate::diarization::owner_assign::owner_is_clustered_here(pool, meeting_id).await;
    let rekey = SpeakersRepository::claim_as_local(
        pool,
        meeting_id,
        speaker_key,
        OWNER_PERSON_ID,
        displace,
    )
    .await
    .context("Couldn't mark that speaker as you")?
    .ok_or_else(|| anyhow!("That speaker is no longer in this meeting"))?;

    let enrolled =
        match enroll_marked_owner(pool, meeting_id, confirmed_voice, store_voiceprints).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "owner enroll after \"This is me\" in {meeting_id} failed (continuing): {e:#}"
                );
                false
            }
        };
    Ok(MarkOutcome { rekey, enrolled })
}

/// Enroll `confirmed_voice` (the marked cluster's `(embedding, model)`) as an owner sample
/// back-linked to `(meeting, local)`, under the consent gate and the same WS3 contested
/// guard `enroll_speaker_gated` applies (specs/0039).
async fn enroll_marked_owner(
    pool: &SqlitePool,
    meeting_id: &str,
    confirmed_voice: Option<(Vec<u8>, Option<String>)>,
    store_voiceprints: bool,
) -> Result<bool> {
    use crate::database::repositories::transcript_speaker_overrides::{
        TranscriptSpeakerOverridesRepository, MATERIAL_CONTEST_FRACTION,
    };
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    use crate::diarization::embedding::{embedding_from_bytes, EMBEDDING_MODEL_ID};

    if enroll::decide_enrollment(
        EnrollConfidence::UserConfirmed,
        true,
        store_voiceprints,
        false,
    ) != enroll::EnrollDecision::EnrollOwner
    {
        return Ok(false);
    }
    let Some((bytes, model)) = confirmed_voice else {
        return Ok(false); // a cluster too short to embed
    };
    let live_sample: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM voiceprints
                        WHERE person_id = ? AND source_meeting_id = ? AND source_speaker_key = ?
                          AND quarantined_at IS NULL)",
    )
    .bind(OWNER_PERSON_ID)
    .bind(meeting_id)
    .bind(LOCAL_SPEAKER_KEY)
    .fetch_one(pool)
    .await
    .context("check for an existing owner sample")?;
    if live_sample {
        return Ok(false);
    }
    let contested = TranscriptSpeakerOverridesRepository::override_fraction_for_speaker(
        pool,
        meeting_id,
        LOCAL_SPEAKER_KEY,
    )
    .await
    .unwrap_or(0.0);
    if contested >= MATERIAL_CONTEST_FRACTION {
        return Ok(false);
    }
    let embedding = match embedding_from_bytes(&bytes) {
        Ok(v) if !v.is_empty() && v.iter().all(|x| x.is_finite()) => v,
        _ => return Ok(false),
    };
    VoiceprintsRepository::add_sample(
        pool,
        OWNER_PERSON_ID,
        &embedding,
        model.as_deref().unwrap_or(EMBEDDING_MODEL_ID),
        Some(meeting_id),
        Some(LOCAL_SPEAKER_KEY),
        None,
    )
    .await
    .context("insert owner voiceprint sample")?;
    Ok(true)
}

/// "This isn't me" at the pool level.
pub(crate) async fn unmark_speaker_as_me(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<RekeyFromLocal> {
    if meeting_id.trim().is_empty() {
        return Err(anyhow!("A meeting is required"));
    }
    SpeakersRepository::rekey_from_local(pool, meeting_id, OWNER_PERSON_ID)
        .await
        .context("Couldn't unmark you")?
        .ok_or_else(|| anyhow!("There is no \"You\" speaker in this meeting"))
}

#[cfg(test)]
mod tests;
