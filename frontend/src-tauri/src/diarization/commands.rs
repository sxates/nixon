//! Tauri commands for speaker diarization (specs/0010, ADR-0005, P1-B2).
//!
//! Frontend → Rust surface consumed by P1-C:
//! - [`api_diarize_meeting`]   — kick off an offline diarization pass (background).
//! - [`api_get_meeting_speakers`] — the per-meeting speaker keys + display names.
//! - [`api_download_diarization_models`] — explicit first-run model fetch.
//! - [`api_diarization_models_present`] — whether both models are cached.
//! - [`api_get_diarization_enabled`] / [`api_set_diarization_enabled`] — the
//!   opt-in on/off setting (default OFF).
//!
//! P2 (specs/0010 Tasks 6 & 7) adds speaker labeling + calendar association:
//! - [`api_rename_speaker`] — rename a speaker's display name.
//! - [`api_merge_speakers`] — merge two speakers (fix over-segmentation).
//! - [`api_get_meeting_attendees`] — the linked calendar event's attendees + an
//!   auto-suggested 1:1 speaker↔attendee mapping.
//! - [`api_assign_speaker_to_attendee`] — set display_name + email from an attendee.
//!
//! Diarization runs asynchronously and reports via the `diarization-{progress,
//! complete,error}` events (see [`crate::diarization::pipeline`]).

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::calendar::eventkit::{self, Attendee};
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::meeting_participant::{
    MeetingParticipant, MeetingParticipantsRepository,
};
use crate::database::repositories::people::PeopleRepository;
use crate::database::repositories::speaker::SpeakersRepository;
use crate::diarization::models;
use crate::diarization::pipeline::{self, EVENT_PROGRESS};
use crate::diarization::settings;
use crate::state::AppState;

/// A speaker as returned to the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerDto {
    pub speaker_key: String,
    pub display_name: String,
    pub is_local: bool,
    /// Calendar-attendee email once associated (specs/0010 P2 Task 7); None otherwise.
    pub email: Option<String>,
    /// Durable Person this speaker is linked to (specs/0016 1b); None until assigned.
    /// Lets the legend consolidate speakers mapped to the same person (specs/0019 WS2.4).
    pub person_id: Option<String>,
}

/// Result of a diarization start request (WS3.1, specs/0029). `started: false`
/// means a run is ALREADY live for the meeting — not a failure: the caller should
/// attach to the running pass (its progress arrives on the shared events and via
/// [`api_diarization_status`]) instead of toasting an error.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizeStartDto {
    pub started: bool,
    pub already_running: bool,
}

/// Start an offline diarization pass for a saved meeting. Returns once the
/// background task is spawned; progress/result arrive via events. At most one
/// run per meeting (WS3.1): a request while one is live returns
/// `started: false` / `already_running: true` instead of spawning a duplicate.
#[tauri::command]
pub async fn api_diarize_meeting<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<DiarizeStartDto, String> {
    if meeting_id.trim().is_empty() {
        return Err("meeting_id cannot be empty".to_string());
    }
    let started = pipeline::diarize_meeting(app, meeting_id);
    Ok(DiarizeStartDto {
        started,
        already_running: !started,
    })
}

/// The current (or last terminal) diarization run status for a meeting (WS3.1,
/// specs/0029): `{ running, stage, progressPct }`, or `null` when no run has
/// happened this session. The frontend rehydrates its "Identifying speakers…"
/// state from this on mount, so navigating away and back never loses (or
/// double-starts) a live pass.
#[tauri::command]
pub async fn api_diarization_status(
    meeting_id: String,
) -> Result<Option<pipeline::DiarizationRunStatus>, String> {
    Ok(pipeline::run_status(&meeting_id))
}

/// The diarized speakers for a meeting (empty until a pass has run).
#[tauri::command]
pub async fn api_get_meeting_speakers<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<Vec<SpeakerDto>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    SpeakersRepository::get_by_meeting(pool, &meeting_id)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|s| SpeakerDto {
                    speaker_key: s.speaker_key,
                    display_name: s.display_name,
                    is_local: s.is_local != 0,
                    email: s.email,
                    person_id: s.person_id,
                })
                .collect()
        })
        .map_err(|e| format!("Failed to load speakers: {e}"))
}

/// Download the two diarization ONNX models on demand, emitting
/// `diarization-progress` per stage. No-op if already cached.
#[tauri::command]
pub async fn api_download_diarization_models<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let progress = move |stage: models::DownloadStage| {
            let label = match stage {
                models::DownloadStage::Segmentation => "downloading segmentation model",
                models::DownloadStage::Embedding => "downloading embedding model",
                models::DownloadStage::Extracting => "extracting model",
            };
            let _ = app.emit(EVENT_PROGRESS, serde_json::json!({ "stage": label }));
        };
        models::ensure_models(Some(&progress)).map(|_| ())
    })
    .await
    .map_err(|e| format!("model download task panicked: {e}"))?
    .map_err(|e| format!("Failed to download diarization models: {e:#}"))
}

/// Whether both diarization models are present (and a plausible size) on disk.
#[tauri::command]
pub async fn api_diarization_models_present() -> Result<bool, String> {
    Ok(models::models_present())
}

/// Whether diarization is enabled (opt-in; default OFF).
#[tauri::command]
pub async fn api_get_diarization_enabled() -> Result<bool, String> {
    Ok(settings::load_settings().await.diarization_enabled)
}

/// Enable or disable diarization. Persisted to disk. Preserves the live sub-toggle.
#[tauri::command]
pub async fn api_set_diarization_enabled(enabled: bool) -> Result<(), String> {
    // Load-mutate-save so flipping one toggle never clobbers the other field.
    let mut current = settings::load_settings().await;
    current.diarization_enabled = enabled;
    settings::save_settings(&current)
        .await
        .map_err(|e| format!("Failed to save diarization setting: {e}"))
}

/// Whether *live* (real-time, while-recording) diarization is enabled (specs/0011
/// P3-B). A sub-toggle under `diarization_enabled`; default OFF. Live labels only run
/// when both this and `diarization_enabled` are on and the models are present.
#[tauri::command]
pub async fn api_get_live_diarization_enabled() -> Result<bool, String> {
    Ok(settings::load_settings().await.live_diarization_enabled)
}

/// Enable or disable live diarization (specs/0011 P3-B). Persisted to disk; preserves
/// the offline `diarization_enabled` toggle. Takes effect on the next recording start.
#[tauri::command]
pub async fn api_set_live_diarization_enabled(enabled: bool) -> Result<(), String> {
    let mut current = settings::load_settings().await;
    current.live_diarization_enabled = enabled;
    settings::save_settings(&current)
        .await
        .map_err(|e| format!("Failed to save live diarization setting: {e}"))
}

/// The user's expected-speaker-count override, or `None` for Auto (specs/0011
/// accuracy gate). When set, diarization forces exactly this many speakers.
#[tauri::command]
pub async fn api_get_expected_speaker_count() -> Result<Option<u32>, String> {
    Ok(settings::load_settings().await.expected_speaker_count)
}

/// Set (or clear, with `None`/`Some(0)`) the expected-speaker-count override.
/// Persisted to disk; preserves the other toggles. Takes effect on the next
/// diarization pass (offline) and next recording start (live). `Some(0)` is
/// normalized to `None` (Auto).
#[tauri::command]
pub async fn api_set_expected_speaker_count(count: Option<u32>) -> Result<(), String> {
    let mut current = settings::load_settings().await;
    // Normalize 0 → None (Auto): a fixed count of zero is meaningless.
    current.expected_speaker_count = match count {
        Some(0) | None => None,
        Some(n) => Some(n),
    };
    settings::save_settings(&current)
        .await
        .map_err(|e| format!("Failed to save expected speaker count: {e}"))
}

/// The two voiceprint-consent toggles surfaced to the frontend (specs/0016 1c,
/// ADR-0007 §2/§3). `storeOthersVoiceprints` is the global opt-in to persist *other
/// people's* voiceprints (default false); `selfEnrollVoiceprint` is the owner ("You")
/// self-enroll toggle (default true).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceprintSettingsDto {
    pub store_others_voiceprints: bool,
    pub self_enroll_voiceprint: bool,
}

/// Read both voiceprint-consent toggles (specs/0016 1c).
#[tauri::command]
pub async fn api_get_voiceprint_settings() -> Result<VoiceprintSettingsDto, String> {
    let s = settings::load_settings().await;
    Ok(VoiceprintSettingsDto {
        store_others_voiceprints: s.store_others_voiceprints,
        self_enroll_voiceprint: s.self_enroll_voiceprint,
    })
}

/// Set the **global** opt-in to store other people's voiceprints (ADR-0007 §2). Persisted;
/// preserves the other diarization toggles. Turning it OFF does not delete already-stored
/// voiceprints (use "clear all" or per-person opt-out for that) — it only blocks future
/// enrollment of non-owner people.
#[tauri::command]
pub async fn api_set_store_others_voiceprints(enabled: bool) -> Result<(), String> {
    let mut current = settings::load_settings().await;
    current.store_others_voiceprints = enabled;
    settings::save_settings(&current)
        .await
        .map_err(|e| format!("Failed to save voiceprint setting: {e}"))
}

/// Set the owner ("You") self-enroll toggle (ADR-0007 §3). Persisted; preserves the other
/// diarization toggles. On by default; turning it off stops self-enrollment from then on
/// (existing owner samples are untouched — use "clear all" to remove them).
#[tauri::command]
pub async fn api_set_self_enroll_voiceprint(enabled: bool) -> Result<(), String> {
    let mut current = settings::load_settings().await;
    current.self_enroll_voiceprint = enabled;
    settings::save_settings(&current)
        .await
        .map_err(|e| format!("Failed to save self-enroll setting: {e}"))
}

/// Wipe the entire voiceprint gallery (ADR-0007 §6 "clear all voiceprints"): every stored
/// voiceprint for every person. Leaves People/identity rows intact. Returns the number of
/// voiceprints deleted.
#[tauri::command]
pub async fn api_clear_all_voiceprints<R: Runtime>(app: AppHandle<R>) -> Result<u64, String> {
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    VoiceprintsRepository::clear_all(pool)
        .await
        .map_err(|e| format!("Failed to clear voiceprints: {e}"))
}

/// Number of stored voiceprint samples for a person (specs/0016 1c) — for the People
/// detail "N voice samples" affordance.
#[tauri::command]
pub async fn api_get_person_voiceprint_count<R: Runtime>(
    app: AppHandle<R>,
    person_id: String,
) -> Result<i64, String> {
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    VoiceprintsRepository::count_for_person(pool, &person_id)
        .await
        .map_err(|e| format!("Failed to count voiceprints: {e}"))
}

/// One voiceprint sample as returned to the People-detail per-sample list (specs/0039 WS3).
/// Display-safe metadata ONLY — provenance + status, **never** the embedding bytes (ADR-0007
/// §1 / specs/0016 no-egress: biometric vectors never cross the IPC boundary).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceprintSampleDto {
    pub id: String,
    pub source_meeting_id: Option<String>,
    pub source_speaker_key: Option<String>,
    pub created_at: String,
    pub sample_quality: Option<f32>,
    pub quarantined: bool,
}

/// List a person's voiceprint samples, newest first (specs/0039 WS3) — the People-detail
/// per-sample gallery where the user can quarantine/restore/delete one bad sample. Returns
/// metadata + status only (no embeddings).
#[tauri::command]
pub async fn api_list_person_voiceprints<R: Runtime>(
    app: AppHandle<R>,
    person_id: String,
) -> Result<Vec<VoiceprintSampleDto>, String> {
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    VoiceprintsRepository::list_for_person(pool, &person_id)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|s| VoiceprintSampleDto {
                    id: s.id,
                    source_meeting_id: s.source_meeting_id,
                    source_speaker_key: s.source_speaker_key,
                    created_at: s.created_at,
                    sample_quality: s.sample_quality,
                    quarantined: s.quarantined,
                })
                .collect()
        })
        .map_err(|e| format!("Failed to load voiceprint samples: {e}"))
}

/// Shared trim/validate + run + not-found check for the per-sample voiceprint commands
/// (specs/0039 WS3). Quarantine/restore/delete differ only in the repo mutation they run, the
/// verb in the error message, and whether a no-match is a user-facing error (`require_match`
/// = quarantine/restore) or a silent no-op (delete is idempotent). `op` receives an owned
/// pool + id (so it needs no borrow across the await) and reports whether a row matched.
async fn mutate_voiceprint_sample<F, Fut>(
    pool: sqlx::SqlitePool,
    sample_id: &str,
    verb: &str,
    require_match: bool,
    op: F,
) -> Result<(), String>
where
    F: FnOnce(sqlx::SqlitePool, String) -> Fut,
    Fut: std::future::Future<Output = Result<bool, sqlx::Error>>,
{
    if sample_id.trim().is_empty() {
        return Err("A voiceprint sample id is required".to_string());
    }
    let matched = op(pool, sample_id.to_string())
        .await
        .map_err(|e| format!("Failed to {verb} voiceprint sample: {e}"))?;
    if require_match && !matched {
        return Err("That voiceprint sample no longer exists".to_string());
    }
    Ok(())
}

/// Quarantine (soft-delete) one voiceprint sample (specs/0039 WS3): it stops contributing to
/// the person's match centroid but its row + provenance survive, so it can be restored. The
/// recoverable alternative to [`api_delete_voiceprint_sample`]. Idempotent.
#[tauri::command]
pub async fn api_quarantine_voiceprint_sample<R: Runtime>(
    app: AppHandle<R>,
    sample_id: String,
) -> Result<(), String> {
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    let pool = app.state::<AppState>().db_manager.pool().clone();
    mutate_voiceprint_sample(pool, &sample_id, "quarantine", true, |p, id| async move {
        VoiceprintsRepository::quarantine_sample(&p, &id).await
    })
    .await
}

/// Restore a quarantined voiceprint sample (specs/0039 WS3): clears its quarantine so it
/// re-enters matching — the undo for the retraction toast and for a manual quarantine.
/// Idempotent on an already-live sample.
#[tauri::command]
pub async fn api_restore_voiceprint_sample<R: Runtime>(
    app: AppHandle<R>,
    sample_id: String,
) -> Result<(), String> {
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    let pool = app.state::<AppState>().db_manager.pool().clone();
    mutate_voiceprint_sample(pool, &sample_id, "restore", true, |p, id| async move {
        VoiceprintsRepository::restore_sample(&p, &id).await
    })
    .await
}

/// Permanently delete one voiceprint sample (specs/0039 WS3) — the explicit, unrecoverable
/// per-sample purge (quarantine is the recoverable default). Idempotent (deleting an absent
/// sample is a no-op → `require_match: false`).
#[tauri::command]
pub async fn api_delete_voiceprint_sample<R: Runtime>(
    app: AppHandle<R>,
    sample_id: String,
) -> Result<(), String> {
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    let pool = app.state::<AppState>().db_manager.pool().clone();
    mutate_voiceprint_sample(pool, &sample_id, "delete", false, |p, id| async move {
        VoiceprintsRepository::delete_sample(&p, &id).await
    })
    .await
}

/// Rename a diarized speaker's display name (specs/0010 P2 Task 6). The new name is
/// resolved through the transcript→speakers join, so every rendered segment updates
/// from this single row change. Errors if the speaker doesn't exist for the meeting.
#[tauri::command]
pub async fn api_rename_speaker<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    speaker_key: String,
    display_name: String,
) -> Result<(), String> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err("display_name cannot be empty".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let renamed = SpeakersRepository::rename(pool, &meeting_id, &speaker_key, display_name)
        .await
        .map_err(|e| format!("Failed to rename speaker: {e}"))?;
    if !renamed {
        return Err(format!(
            "No speaker '{speaker_key}' found for this meeting to rename"
        ));
    }
    // specs/0044 WS3: naming changed what a summary would say — debounced refresh
    // of a pristine summary whose name set is now stale. Fire-and-forget.
    crate::summary::refresh::schedule_name_refresh(&app, pool.clone(), &meeting_id);
    Ok(())
}

/// Merge speaker `from_key` into `into_key` (specs/0010 P2 Task 6) to fix
/// over-segmentation: every transcript segment labeled `from_key` is reassigned to
/// `into_key`, and the now-orphaned `from_key` speaker row is deleted. Idempotent.
#[tauri::command]
pub async fn api_merge_speakers<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    from_key: String,
    into_key: String,
) -> Result<(), String> {
    if from_key.trim().is_empty() || into_key.trim().is_empty() {
        return Err("from_key and into_key cannot be empty".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    SpeakersRepository::merge(pool, &meeting_id, &from_key, &into_key)
        .await
        .map_err(|e| format!("Failed to merge speakers: {e}"))?;
    // specs/0044 WS3: a merge collapses the resolved name set — debounced refresh.
    crate::summary::refresh::schedule_name_refresh(&app, pool.clone(), &meeting_id);
    Ok(())
}

/// Associate a diarized speaker with a real calendar attendee (specs/0010 P2 Task 7):
/// set both the display name and the stable `email` identity key on the speaker row.
#[tauri::command]
pub async fn api_assign_speaker_to_attendee<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    speaker_key: String,
    display_name: String,
    email: String,
) -> Result<(), String> {
    let display_name = display_name.trim();
    let email = email.trim();
    if display_name.is_empty() {
        return Err("display_name cannot be empty".to_string());
    }
    if email.is_empty() {
        return Err("email cannot be empty".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let assigned = SpeakersRepository::assign_to_attendee(
        pool,
        &meeting_id,
        &speaker_key,
        display_name,
        email,
    )
    .await
    .map_err(|e| format!("Failed to assign speaker to attendee: {e}"))?;
    if !assigned {
        return Err(format!(
            "No speaker '{speaker_key}' found for this meeting to assign"
        ));
    }

    // specs/0018: if the assigned address is one of the owner's emails, this attendee IS
    // the owner — link the speaker to the singleton "You" person rather than minting a
    // separate person, and route enrollment through the owner self-enroll gate. When the
    // address is NOT an owner email, behavior is byte-identical to before.
    use crate::database::repositories::owner_emails::OwnerEmailsRepository;
    let is_owner_email = OwnerEmailsRepository::contains(pool, email)
        .await
        .unwrap_or(false);

    // Enroll-on-confirm (specs/0016 1c): an attendee assignment is a confirmed identity,
    // so it should grow the gallery too — but it writes `email`, not `person_id`. Upsert
    // a durable `people` row by email first (the cross-meeting anchor), link the speaker
    // to it, then enroll behind the consent gate. All best-effort: a failure here must
    // not undo the (committed) attendee assignment.
    {
        use crate::database::repositories::people::PeopleRepository;

        // Owner branch: ensure the "You" person, link the speaker to it (no separate
        // person), and enroll under the owner. Non-owner branch: the original upsert path.
        let person_result = if is_owner_email {
            match crate::people::enroll::ensure_owner_person(pool).await {
                Ok(owner_id) => PeopleRepository::get(pool, &owner_id).await,
                Err(e) => Err(e),
            }
            .and_then(|opt| {
                opt.ok_or_else(|| sqlx::Error::Protocol("owner person missing".to_string()))
            })
        } else {
            PeopleRepository::create(pool, display_name, Some(email), None, None).await
        };

        match person_result {
            Ok(person) => {
                // Link the speaker to the durable person (also keeps name/email consistent).
                if let Err(e) = PeopleRepository::assign_speaker_to_person(
                    pool,
                    &meeting_id,
                    &speaker_key,
                    &person.id,
                )
                .await
                {
                    log::warn!("attendee-assign: link person failed (continuing): {e}");
                }
                // specs/0038 WS6.c: naming a speaker from an attendee also adds that person
                // to the meeting roster (so an identified speaker always shows as a
                // participant). `add_identified` skips the owner ("You" is implicit, never a
                // roster row — specs/0018; the owner-email branch resolves to the singleton
                // owner person) and dedupes on the `(meeting_id, person_id)` PK. Best-effort:
                // the assignment is already committed; a roster failure must not undo it.
                if let Err(e) =
                    MeetingParticipantsRepository::add_identified(pool, &meeting_id, &person.id)
                        .await
                {
                    log::warn!(
                        "attendee-assign: roster add for {} in meeting {meeting_id} failed (continuing): {e}",
                        person.id
                    );
                }
                // Owner-email attendee → force the OWNER self-enroll gate (the speaker is a
                // remote cluster the user declared as their own voice). Otherwise the usual
                // structural is_local routing.
                let enroll = if is_owner_email {
                    crate::people::enroll::enroll_owner_voiceprint_for_speaker(
                        pool,
                        &meeting_id,
                        &speaker_key,
                    )
                    .await
                } else {
                    crate::people::enroll::enroll_voiceprint_for_speaker(
                        pool,
                        &meeting_id,
                        &speaker_key,
                        &person.id,
                        // Explicit attendee assignment = user-confirmed (specs/0039 WS3).
                        crate::people::enroll::EnrollConfidence::UserConfirmed,
                    )
                    .await
                };
                match enroll {
                    Ok(true) => log::info!(
                        "enrolled voiceprint for {speaker_key} in meeting {meeting_id} (attendee {email})"
                    ),
                    Ok(false) => {}
                    Err(e) => log::warn!(
                        "attendee-assign voiceprint enroll failed (continuing): {e:#}"
                    ),
                }
            }
            Err(e) => {
                log::warn!("attendee-assign: upsert person by email failed (continuing): {e}")
            }
        }
    }

    // specs/0044 WS3: the speaker now carries a real name — debounced refresh of a
    // pristine summary whose name set is stale. Fire-and-forget.
    crate::summary::refresh::schedule_name_refresh(&app, pool.clone(), &meeting_id);
    Ok(())
}

/// Attendees of the calendar event linked to a meeting, plus an optional auto-suggest
/// (specs/0010 P2 Task 7). Reuses the EventKit reader (`specs/0008`). When the meeting
/// persisted a `calendar_event_id` (specs/0015, Join & Record), attendees are resolved
/// by that exact event id; otherwise we fall back to re-locating the event by title +
/// recording-start instant (legacy recorded meetings with no stored id).
///
/// `suggestion`, when present, is the obvious 1:1 mapping: exactly one remote (non-
/// local) speaker and exactly one remote attendee → pre-fill that pairing in the UI.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingAttendeesDto {
    pub attendees: Vec<Attendee>,
    pub suggestion: Option<AttendeeSuggestion>,
}

/// A pre-filled speaker↔attendee pairing the UI can offer as a one-click default.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttendeeSuggestion {
    pub speaker_key: String,
    pub display_name: String,
    pub email: String,
}

/// Cross-meeting name suggestions for a saved meeting (specs/0016 1a). Re-runs the
/// pure cosine matcher over this meeting's persisted remote voiceprints vs. previously
/// identified speakers, returning a suggestion per current speaker that clears the
/// confidence threshold + runner-up margin. Empty when nothing matches (or the meeting
/// has no stored embeddings, e.g. it predates 1a). Suggestions carry **no** embedding
/// bytes — only display-safe fields.
///
/// This is the same matcher the offline pass runs and attaches to
/// `diarization-complete`; the command lets the detail view re-fetch on demand.
#[tauri::command]
pub async fn api_get_speaker_suggestions<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<Vec<crate::diarization::identity::SpeakerSuggestion>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    pipeline::compute_suggestions(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to compute speaker suggestions: {e:#}"))
}

#[tauri::command]
pub async fn api_get_meeting_attendees<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<MeetingAttendeesDto, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let meeting = MeetingsRepository::get_meeting_metadata(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load meeting: {e}"))?
        .ok_or_else(|| "Meeting not found".to_string())?;

    let title = meeting.title.clone();
    let started_at = meeting.created_at.0.to_rfc3339();
    let calendar_event_id = meeting.calendar_event_id.clone();

    // Prefer an exact lookup by the stored calendar event id (specs/0015) — the id names
    // its source, so `gcal:`-prefixed ids route to the local Google cache and EventKit
    // ids to EventKit regardless of which source is currently active (a meeting recorded
    // under either source keeps resolving after a switch; a purged Google cache degrades
    // to empty, never an error). Legacy meetings with no persisted id use the title+time
    // match, routed by the single active source (specs/0032): Google connected →
    // cache-only; otherwise EventKit only. EventKit reads touch the Objective-C runtime,
    // so those run off the async executor thread.
    let attendees = match calendar_event_id {
        Some(event_id) if event_id.starts_with("gcal:") => {
            crate::calendar::google::sync::cached_attendees_for_event_id(pool, &event_id).await
        }
        Some(event_id) if !event_id.trim().is_empty() => {
            tokio::task::spawn_blocking(move || eventkit::event_attendees_by_id(&event_id))
                .await
                .map_err(|e| format!("Calendar read task failed: {e}"))?
        }
        _ => {
            if crate::calendar::google_is_active_source(&app).await {
                crate::calendar::google::sync::cached_attendees_by_title_time(
                    pool,
                    &title,
                    &started_at,
                )
                .await
            } else {
                let (t, s) = (title.clone(), started_at.clone());
                tokio::task::spawn_blocking(move || eventkit::event_attendees(&t, &s))
                    .await
                    .map_err(|e| format!("Calendar read task failed: {e}"))?
            }
        }
    };

    // Auto-suggest the obvious 1:1 mapping: exactly one remote attendee AND exactly
    // one remote (non-local), still-unnamed speaker → pair them.
    let speakers = SpeakersRepository::get_by_meeting(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load speakers: {e}"))?;

    let suggestion = pick_1to1_attendee_suggestion(&attendees, &speakers);

    Ok(MeetingAttendeesDto {
        attendees,
        suggestion,
    })
}

/// The obvious 1:1 auto-suggest: exactly one bindable remote attendee AND
/// exactly one still-unnamed remote speaker → pair them. A bindable attendee is
/// a real person (not the current user, not a distribution list — finding #2)
/// carrying an email; a DL is never a speaker identity. Any other cardinality
/// (0, or 2+) yields no suggestion.
fn pick_1to1_attendee_suggestion(
    attendees: &[Attendee],
    speakers: &[crate::database::models::SpeakerModel],
) -> Option<AttendeeSuggestion> {
    let remote_attendees: Vec<&Attendee> = attendees
        .iter()
        .filter(|a| !a.is_current_user && !a.is_distribution_list && a.email.is_some())
        .collect();
    let remote_speakers: Vec<_> = speakers
        .iter()
        .filter(|s| s.is_local == 0 && s.email.is_none())
        .collect();

    match (remote_attendees.as_slice(), remote_speakers.as_slice()) {
        ([attendee], [speaker]) => Some(AttendeeSuggestion {
            speaker_key: speaker.speaker_key.clone(),
            display_name: attendee.name.clone(),
            email: attendee
                .email
                .clone()
                .expect("filtered to Some(email) above"),
        }),
        _ => None,
    }
}

/// Resolve a calendar-linked meeting's attendees off the async executor (EventKit FFI is
/// blocking), preferring the stored event id (specs/0015) — routed by the id's own source —
/// then falling back to a title+time match routed by the single active calendar source
/// (specs/0032). Returns an empty list (never an error) when the meeting has no calendar
/// link or the read fails — seeding is best-effort and must never block returning the roster.
async fn resolve_attendees_for_meeting<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
) -> Vec<Attendee> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let meeting = match MeetingsRepository::get_meeting_metadata(pool, meeting_id).await {
        Ok(Some(m)) => m,
        _ => return Vec::new(),
    };

    let calendar_event_id = match meeting.calendar_event_id.clone() {
        Some(id) if !id.trim().is_empty() => id,
        // No calendar link → nothing to seed from (ad-hoc recordings start empty).
        _ => return Vec::new(),
    };

    // Google-sourced meetings (specs/0032): attendees ride in the local cache
    // row — no EventKit read, no network. After a disconnect the purged cache
    // degrades this to an empty roster (never an error).
    if calendar_event_id.starts_with("gcal:") {
        return crate::calendar::google::sync::cached_attendees_for_event_id(
            pool,
            &calendar_event_id,
        )
        .await;
    }

    let title = meeting.title.clone();
    let started_at = meeting.created_at.0.to_rfc3339();

    // The stored id names its source: a non-`gcal:` id is an EventKit id, so
    // the exact lookup stays an EventKit read regardless of the active source.
    let event_id = calendar_event_id;
    let by_id = tokio::task::spawn_blocking(move || eventkit::event_attendees_by_id(&event_id))
        .await
        .unwrap_or_default();
    if !by_id.is_empty() {
        return by_id;
    }

    // Legacy/fuzzy title+time fallback when the id no longer resolves, routed
    // by the single active source (specs/0032): Google connected → local-cache
    // lookup only; otherwise EventKit only.
    if crate::calendar::google_is_active_source(app).await {
        return crate::calendar::google::sync::cached_attendees_by_title_time(
            pool,
            &title,
            &started_at,
        )
        .await;
    }
    tokio::task::spawn_blocking(move || eventkit::event_attendees(&title, &started_at))
        .await
        .unwrap_or_default()
}

/// The persistent participant roster for a meeting (specs/0017). Lazy seed-on-view: if the
/// meeting is linked to a calendar event, resolve its attendees and `seed_from_attendees`
/// (idempotent) before returning, so legacy/linked meetings self-heal on first view. An
/// EventKit failure is non-fatal — we just return whatever rows already exist.
#[tauri::command]
pub async fn api_get_meeting_participants<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<Vec<MeetingParticipant>, String> {
    let pool = {
        let state = app.state::<AppState>();
        state.db_manager.pool().clone()
    };

    // Best-effort seed from the calendar (idempotent). Skips cleanly when there's no
    // calendar link or the read fails — the roster below is the source of truth.
    let attendees = resolve_attendees_for_meeting(&app, &meeting_id).await;
    if !attendees.is_empty() {
        if let Err(e) =
            MeetingParticipantsRepository::seed_from_attendees(&pool, &meeting_id, &attendees).await
        {
            log::warn!("participant seed-on-view for {meeting_id} failed (continuing): {e}");
        }
    }

    MeetingParticipantsRepository::list(&pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load participants: {e}"))
}

/// Add a participant to a meeting's roster (specs/0017). Either links an existing Person by
/// `personId`, or creates/upserts a Person from `displayName`/`email` (upsert-by-email when
/// an email is given) and links it. The added row's `source` is `'manual'`. Works during and
/// after recording (a plain DB write). Returns the added [`MeetingParticipant`].
#[tauri::command]
pub async fn api_add_meeting_participant<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    person_id: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
) -> Result<MeetingParticipant, String> {
    if meeting_id.trim().is_empty() {
        return Err("A meeting is required".to_string());
    }
    let pool = {
        let state = app.state::<AppState>();
        state.db_manager.pool().clone()
    };

    // Resolve the person: link an existing one, or create/upsert from name/email.
    let person_id = match person_id {
        Some(id) if !id.trim().is_empty() => {
            let id = id.trim().to_string();
            if PeopleRepository::get(&pool, &id)
                .await
                .map_err(|e| format!("Failed to load person: {e}"))?
                .is_none()
            {
                return Err("That person no longer exists".to_string());
            }
            id
        }
        _ => {
            let name = display_name.as_deref().map(str::trim).unwrap_or("");
            let email = email.as_deref().map(str::trim).filter(|e| !e.is_empty());
            if name.is_empty() && email.is_none() {
                return Err("A name or email is required to add a participant".to_string());
            }
            // Fall back to the email local-part as the display name when no name is given.
            let name = if name.is_empty() {
                email
                    .and_then(|e| e.split('@').next())
                    .unwrap_or("Unknown")
                    .to_string()
            } else {
                name.to_string()
            };
            let person = PeopleRepository::create(&pool, &name, email, None, None)
                .await
                .map_err(|e| format!("Failed to create person: {e}"))?;
            person.id
        }
    };

    MeetingParticipantsRepository::restore_or_add(&pool, &meeting_id, &person_id, "manual")
        .await
        .map_err(|e| format!("Failed to add participant: {e}"))?;

    // Return the freshly-joined roster row (so the caller gets name/email/role/source).
    let roster = MeetingParticipantsRepository::list(&pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load participant: {e}"))?;
    roster
        .into_iter()
        .find(|p| p.person_id == person_id)
        .ok_or_else(|| "Participant was added but could not be read back".to_string())
}

/// Remove a participant from a meeting's roster (specs/0017). Deletes the JOIN ROW ONLY —
/// the Person and any `speakers.person_id` link survive. Idempotent (removing an absent
/// participant is a no-op).
#[tauri::command]
pub async fn api_remove_meeting_participant<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    person_id: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    MeetingParticipantsRepository::remove(pool, &meeting_id, &person_id)
        .await
        .map_err(|e| format!("Failed to remove participant: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // --- 1:1 attendee auto-suggest (finding #2) -----------------------------

    fn attendee(name: &str, email: Option<&str>, is_dl: bool, is_me: bool) -> Attendee {
        Attendee {
            name: name.to_string(),
            email: email.map(str::to_string),
            is_current_user: is_me,
            is_distribution_list: is_dl,
            photo_data_uri: None,
        }
    }

    fn remote_speaker(key: &str) -> crate::database::models::SpeakerModel {
        use crate::database::models::DateTimeUtc;
        let now = DateTimeUtc(chrono::Utc::now());
        crate::database::models::SpeakerModel {
            id: format!("id-{key}"),
            meeting_id: "m1".to_string(),
            speaker_key: key.to_string(),
            display_name: key.to_string(),
            is_local: 0,
            email: None,
            person_id: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn one_to_one_suggestion_pairs_a_lone_person_and_speaker() {
        let attendees = vec![
            attendee("Me", Some("me@x.com"), false, true), // current user — excluded
            attendee("Priya", Some("priya@x.com"), false, false),
        ];
        let speakers = vec![remote_speaker("spk_1")];
        let suggestion = pick_1to1_attendee_suggestion(&attendees, &speakers).expect("pairs 1:1");
        assert_eq!(suggestion.speaker_key, "spk_1");
        assert_eq!(suggestion.email, "priya@x.com");
        assert_eq!(suggestion.display_name, "Priya");
    }

    #[test]
    fn distribution_list_is_never_auto_suggested_as_a_speaker() {
        // One real remote speaker, and the only emailed non-owner attendee is a
        // distribution list — it must NOT be bound to the speaker (finding #2).
        let attendees = vec![
            attendee("Me", Some("me@x.com"), false, true),
            attendee("eng-team@x.com", Some("eng-team@x.com"), true, false),
        ];
        let speakers = vec![remote_speaker("spk_1")];
        assert!(
            pick_1to1_attendee_suggestion(&attendees, &speakers).is_none(),
            "a DL is not a person and must never be suggested as a speaker identity"
        );

        // With a real person alongside the DL, the DL is ignored and the person
        // is the unique bindable attendee → it pairs.
        let attendees = vec![
            attendee("eng-team@x.com", Some("eng-team@x.com"), true, false),
            attendee("Priya", Some("priya@x.com"), false, false),
        ];
        let suggestion =
            pick_1to1_attendee_suggestion(&attendees, &speakers).expect("the lone person pairs");
        assert_eq!(suggestion.email, "priya@x.com");
    }
}
