// Meetings Tauri commands (moved from api/api.rs in specs/0042 WS3).

use log::{error as log_error, info as log_info, warn as log_warn};
use tauri::{AppHandle, Runtime};

use crate::{
    database::{
        models::MeetingModel,
        repositories::{
            meeting::MeetingsRepository, meeting_participant::MeetingParticipantsRepository,
        },
    },
    state::AppState,
};

use super::view::{
    gist_from_summary, gist_from_text, AttendeePreview, Meeting, MeetingDetails, MeetingMetadata,
    MeetingTranscript, PaginatedTranscriptsResponse,
};

#[tauri::command]
pub async fn api_get_meetings<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Meeting>, String> {
    log_info!("api_get_meetings called (native)");
    let pool = state.db_manager.pool();
    let meetings = MeetingsRepository::get_meetings_enriched(pool).await;

    match meetings {
        Ok(rows) => {
            log_info!("Successfully got {} meetings", rows.len());

            // Bounded attendee preview per meeting in ONE query (no N+1 across the set).
            // A failure here must never break the list — degrade to no previews.
            let mut previews: std::collections::HashMap<String, (Vec<AttendeePreview>, i64)> =
                match MeetingParticipantsRepository::attendee_previews(pool).await {
                    Ok(rows) => {
                        let mut map: std::collections::HashMap<
                            String,
                            (Vec<AttendeePreview>, i64),
                        > = std::collections::HashMap::new();
                        for r in rows {
                            let entry = map
                                .entry(r.meeting_id)
                                .or_insert_with(|| (Vec::new(), r.total));
                            entry.0.push(AttendeePreview {
                                name: r.display_name,
                                email: r.email,
                                is_current_user: r.is_current_user != 0,
                            });
                        }
                        map
                    }
                    Err(e) => {
                        log_warn!("Failed to load attendee previews: {}", e);
                        std::collections::HashMap::new()
                    }
                };

            let result: Vec<Meeting> = rows
                .into_iter()
                .map(|m| {
                    let gist = gist_from_summary(m.summary_result.as_deref())
                        .or_else(|| gist_from_text(m.first_transcript.as_deref()));
                    let (attendees, attendee_count) = previews.remove(&m.id).unwrap_or_default();
                    Meeting {
                        id: m.id,
                        title: m.title,
                        created_at: m.created_at.0.to_rfc3339(),
                        updated_at: m.updated_at.0.to_rfc3339(),
                        duration_seconds: m.duration_seconds,
                        gist,
                        origin: m.origin,
                        calendar_event_id: m.calendar_event_id,
                        attendees,
                        attendee_count,
                        reel_number: m.reel_number,
                    }
                })
                .collect();
            Ok(result)
        }
        Err(e) => {
            log_error!("Error getting meetings: {}", e);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub async fn api_delete_meeting<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<serde_json::Value, String> {
    log_info!(
        "api_delete_meeting called for meeting_id(native): {}",
        meeting_id
    );

    let pool = state.db_manager.pool();

    // specs/0073: hold the meeting's folder lease across the folder_path read, the DB
    // delete and the folder removal, so a delete never pulls a folder out from under a
    // retranscription, diarization or move of the same meeting. Waits for that job.
    let _folder_lease = crate::audio::folder_lease::acquire(
        &meeting_id,
        crate::audio::folder_lease::LeaseHolder::Delete,
    )
    .await;

    // Capture the recording folder BEFORE the delete (and after taking the lease, so it is
    // the folder's current location) so we can clean it up from disk after the DB
    // transaction commits. If the lookup fails we still proceed with the DB delete.
    let folder_path: Option<String> =
        sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
            .bind(&meeting_id)
            .fetch_optional(pool)
            .await
            .unwrap_or_else(|e| {
                log_warn!(
                    "Could not read folder_path for meeting {} before delete: {}",
                    meeting_id,
                    e
                );
                None
            })
            .flatten();

    match MeetingsRepository::delete_meeting(pool, &meeting_id).await {
        Ok(true) => {
            log_info!("Successfully deleted meeting {}", meeting_id);
            // DB rows are gone. Now best-effort remove the on-disk recording
            // folder. A filesystem error here must NOT fail the command or roll
            // back the committed DB delete — log and continue.
            delete_recording_folder_best_effort(&meeting_id, folder_path.as_deref()).await;
            Ok(serde_json::json!({
                "status": "success",
                "message": "Meeting deleted successfully"
            }))
        }
        Ok(false) => {
            log_warn!("Meeting not found or already deleted: {}", meeting_id);
            Err(format!(
                "Meeting not found or could not be deleted: {}",
                meeting_id
            ))
        }
        Err(e) => {
            log_error!("Error deleting meeting {}: {}", meeting_id, e);
            Err(format!("Failed to delete meeting: {}", e))
        }
    }
}

/// Best-effort removal of a meeting's on-disk recording folder after its DB rows
/// have been deleted.
///
/// Safety (specs/0073): the folder is removed only if it passes the ownership check
/// ([`is_meeting_folder`](crate::audio::meeting_folder::is_meeting_folder)): its
/// `metadata.json` names this meeting, or (a pre-0037 folder with no id) it lies inside
/// a recordings folder Nixon knows about, and it is never a root, `$HOME`, `/` or a volume
/// root. So a meeting left in an EARLIER recordings folder is still deleted, while we
/// never `remove_dir_all` an arbitrary stored path. A filesystem error (including the
/// folder already being gone) is logged and swallowed; it never fails the delete command.
async fn delete_recording_folder_best_effort(meeting_id: &str, folder_path: Option<&str>) {
    use crate::audio::meeting_folder::{remove_meeting_folder_in, RemoveOutcome};

    let folder_path = match folder_path {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => {
            log_info!("No recording folder to delete (meeting has no folder_path); skipping");
            return;
        }
    };
    let id = meeting_id.to_string();
    let target = folder_path.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let roots = crate::audio::recording_preferences::known_recording_roots();
        remove_meeting_folder_in(std::path::Path::new(&target), &id, &roots)
    })
    .await
    .unwrap_or_else(|e| RemoveOutcome::Failed(format!("delete task failed: {e}")));

    match outcome {
        RemoveOutcome::Removed => log_info!("Deleted recording folder: {}", folder_path),
        RemoveOutcome::AlreadyGone => log_info!(
            "Recording folder '{}' already gone; nothing to delete",
            folder_path
        ),
        RemoveOutcome::NotOwned => log_warn!(
            "Recording folder '{}' is not this meeting's recording folder; \
             skipping file deletion for safety",
            folder_path
        ),
        RemoveOutcome::Failed(e) => log_warn!(
            "Failed to delete recording folder '{}' (DB delete already committed): {}",
            folder_path,
            e
        ),
    }
}

#[tauri::command]
pub async fn api_get_meeting<R: Runtime>(
    _app: AppHandle<R>,
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<MeetingDetails, String> {
    log_info!(
        "api_get_meeting called(native) for meeting_id: {}",
        meeting_id
    );

    let pool = state.db_manager.pool();

    match MeetingsRepository::get_meeting(pool, &meeting_id).await {
        Ok(Some(meeting)) => {
            log_info!("Successfully retrieved meeting {}", meeting_id);
            Ok(meeting)
        }
        Ok(None) => {
            log_warn!("Meeting not found: {}", meeting_id);
            Err(format!("Meeting not found: {}", meeting_id))
        }
        Err(e) => {
            log_error!("Error retrieving meeting {}: {}", meeting_id, e);
            Err(format!("Failed to retrieve meeting: {}", e))
        }
    }
}

/// Get meeting metadata without transcripts (for pagination)
#[tauri::command]
pub async fn api_get_meeting_metadata<R: Runtime>(
    _app: AppHandle<R>,
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<MeetingMetadata, String> {
    log_info!(
        "api_get_meeting_metadata called for meeting_id: {}",
        meeting_id
    );

    let pool = state.db_manager.pool();

    match MeetingsRepository::get_meeting_metadata(pool, &meeting_id).await {
        Ok(Some(meeting)) => {
            log_info!("Successfully retrieved meeting metadata {}", meeting_id);
            // specs/0057: the reel ordinal for the detail page's identity line + reel
            // label. Non-fatal — a failure just hides the reel handle, never the meeting.
            let reel_number = MeetingsRepository::get_reel_number(pool, &meeting_id)
                .await
                .unwrap_or_else(|e| {
                    log_warn!("Could not compute reel number for {}: {}", meeting_id, e);
                    None
                });
            let is_manual_entry = meeting
                .calendar_event_id
                .as_deref()
                .map(crate::database::repositories::meeting::is_manual_event_id)
                .unwrap_or(false);

            Ok(MeetingMetadata {
                id: meeting.id,
                title: meeting.title,
                created_at: meeting.created_at.0.to_rfc3339(),
                updated_at: meeting.updated_at.0.to_rfc3339(),
                folder_path: meeting.folder_path,
                origin: meeting.origin,
                calendar_event_id: meeting.calendar_event_id,
                calendar_series_key: meeting.calendar_series_key,
                reel_number,
                scheduled_end_at: meeting.scheduled_end_at.map(|dt| dt.0.to_rfc3339()),
                join_url: meeting.join_url,
                is_manual_entry,
            })
        }
        Ok(None) => {
            log_warn!("Meeting not found: {}", meeting_id);
            Err(format!("Meeting not found: {}", meeting_id))
        }
        Err(e) => {
            log_error!("Error retrieving meeting metadata {}: {}", meeting_id, e);
            Err(format!("Failed to retrieve meeting metadata: {}", e))
        }
    }
}

/// Per-meeting processing-mode override (low-power-mode spec §3).
#[tauri::command]
pub async fn api_get_meeting_processing_mode<R: Runtime>(
    _app: AppHandle<R>,
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let pool = state.db_manager.pool();
    MeetingsRepository::get_processing_mode(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load processing mode: {e}"))
}

/// Persist the override. Pass `mode: None` to clear (follow global).
#[tauri::command]
pub async fn api_set_meeting_processing_mode<R: Runtime>(
    _app: AppHandle<R>,
    meeting_id: String,
    mode: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let pool = state.db_manager.pool();
    let updated = MeetingsRepository::set_processing_mode(pool, &meeting_id, mode.as_deref())
        .await
        .map_err(|e| format!("Failed to save processing mode: {e}"))?;
    if !updated {
        return Err("Meeting not found".to_string());
    }
    if mode.is_none() {
        // specs/0072: the backlog clears `defer` last. A sparse transcript is only taken as
        // final when a transcription pass for this meeting completed (server-side evidence).
        crate::audio::lifecycle::reevaluate_after_backlog(&_app, &meeting_id).await;
    }
    Ok(())
}

/// Get paginated transcripts for a meeting
#[tauri::command]
pub async fn api_get_meeting_transcripts<R: Runtime>(
    _app: AppHandle<R>,
    meeting_id: String,
    limit: i64,
    offset: i64,
    state: tauri::State<'_, AppState>,
) -> Result<PaginatedTranscriptsResponse, String> {
    log_info!(
        "api_get_meeting_transcripts called for meeting_id: {}, limit: {}, offset: {}",
        meeting_id,
        limit,
        offset
    );

    let pool = state.db_manager.pool();

    match MeetingsRepository::get_meeting_transcripts_paginated(pool, &meeting_id, limit, offset)
        .await
    {
        Ok((transcripts, total_count)) => {
            log_info!(
                "Successfully retrieved {} transcripts for meeting {} (total: {})",
                transcripts.len(),
                meeting_id,
                total_count
            );

            // Convert to MeetingTranscript (carries speaker + resolved display name)
            let meeting_transcripts = transcripts
                .into_iter()
                .map(MeetingTranscript::from)
                .collect::<Vec<_>>();

            let has_more = (offset + meeting_transcripts.len() as i64) < total_count;

            Ok(PaginatedTranscriptsResponse {
                transcripts: meeting_transcripts,
                total_count,
                has_more,
            })
        }
        Err(e) => {
            log_error!(
                "Error retrieving transcripts for meeting {}: {}",
                meeting_id,
                e
            );
            Err(format!("Failed to retrieve transcripts: {}", e))
        }
    }
}

#[tauri::command]
pub async fn api_save_meeting_title<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    title: String,
) -> Result<serde_json::Value, String> {
    log_info!(
        "api_save_meeting_title called for meeting_id: {}",
        meeting_id
    );
    let pool = state.db_manager.pool();
    match MeetingsRepository::update_meeting_title(pool, &meeting_id, &title).await {
        Ok(true) => {
            log_info!("Successfully saved meeting title");
            Ok(serde_json::json!({"message": "Meeting title saved successfully"}))
        }
        Ok(false) => {
            log_error!("No meeting found with id {}", meeting_id);
            Err(format!("No meeting found with id {}", meeting_id))
        }
        Err(e) => {
            log_error!("Failed to update meeting {}", e);
            Err(format!("Failed to update meeting: {}", e))
        }
    }
}

/// Creates a `meetings` row at recording START (specs/0007) so notes can autosave
/// against a real meeting id while the meeting is still in progress. Transcript
/// segments are attached later on STOP via `api_save_transcript` (passing the
/// returned `meeting_id`). Returns `{ "meeting_id": "<id>" }`, matching the shape
/// `api_save_transcript` already returns.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // cohesive param set; refactor deferred
pub async fn api_create_meeting<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_title: Option<String>,
    folder_path: Option<String>,
    origin: Option<String>,
    calendar_event_id: Option<String>,
    started_at: Option<String>,
    // specs/0036: the calendar event's recurring-series key (external_id / iCalUID), passed
    // by Join & Record so the recording is grouped into its series. Optional/backward-compatible
    // — callers that don't pass it leave the row's series key NULL (title fallback still works).
    calendar_series_key: Option<String>,
) -> Result<serde_json::Value, String> {
    log_info!(
        "api_create_meeting called for title: {:?}, folder_path: {:?}, origin: {:?}, calendar_event_id: {:?}, started_at: {:?}, series_key: {:?}",
        meeting_title,
        folder_path,
        origin,
        calendar_event_id,
        started_at,
        calendar_series_key
    );

    let pool = state.db_manager.pool();

    // For a calendar-linked meeting (Join & Record) the frontend passes the event's
    // scheduled start (ISO-8601) so the meeting is dated to the event, not the
    // recording start. Parse best-effort: an unparseable/absent value falls back to
    // now (the normal ad-hoc recording case) — a bad timestamp must never block create.
    let started_at =
        started_at
            .as_deref()
            .and_then(|s| match chrono::DateTime::parse_from_rfc3339(s) {
                Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
                Err(e) => {
                    log_error!(
                        "api_create_meeting: ignoring unparseable started_at {:?}: {}",
                        s,
                        e
                    );
                    None
                }
            });

    if let Some(event_id) = calendar_event_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
    {
        // specs/0036: adopt a pre-created `scheduled` prep row for THIS occurrence so its prep
        // notes + cached brief carry into the recording as one continuous meeting. Matched by
        // (calendar_event_id, same day as the occurrence start) — the day disambiguates
        // EventKit's shared-across-series event id. Preferred over the empty-row dedupe below.
        let occurrence = started_at.unwrap_or_else(chrono::Utc::now);
        match MeetingsRepository::find_scheduled_for_occurrence(pool, event_id, occurrence).await {
            Ok(Some(prep_id)) => {
                // specs/0064 W1 — a per-occurrence (Google) id now matches regardless of day,
                // so the adopted row may still be dated to the slot the meeting was moved
                // FROM. `promote_scheduled_to_recorded` deliberately preserves `created_at`,
                // which would file the recording under the old date. Re-date it to the
                // occurrence actually being recorded first; best-effort, since a failure here
                // must not block starting a recording.
                if let Err(e) = MeetingsRepository::redate_scheduled_meeting(
                    pool, &prep_id, occurrence, None, None,
                )
                .await
                {
                    log_error!(
                        "api_create_meeting: could not re-date adopted prep row {} (continuing): {}",
                        prep_id,
                        e
                    );
                }
                match MeetingsRepository::promote_scheduled_to_recorded(pool, &prep_id).await {
                    Ok(true) => {
                        log_info!(
                        "api_create_meeting: adopting scheduled prep meeting {} for event {} (specs/0036)",
                        prep_id, event_id
                    );
                        if let Some(k) = calendar_series_key
                            .as_deref()
                            .filter(|k| !k.trim().is_empty())
                        {
                            if let Err(e) =
                                MeetingsRepository::set_calendar_series_key(pool, &prep_id, Some(k))
                                    .await
                            {
                                log_error!("api_create_meeting: failed to set series key on adopted prep (non-fatal): {}", e);
                            }
                        }
                        return Ok(serde_json::json!({ "meeting_id": prep_id }));
                    }
                    // Already promoted (raced) or not scheduled anymore: fall through to the
                    // empty-row dedupe / normal create rather than duplicating.
                    Ok(false) => {}
                    Err(e) => log_error!(
                        "api_create_meeting: scheduled-prep promotion failed (continuing): {}",
                        e
                    ),
                }
            }
            Ok(None) => {}
            Err(e) => log_error!(
                "api_create_meeting: scheduled-prep lookup failed (continuing): {}",
                e
            ),
        }

        // WS2.1 (specs/0024): a calendar-started recording must be ONE object. If an empty,
        // recently-created row already exists for this event (the Join & Record pre-create that
        // the recorder failed to adopt), reuse it instead of minting a duplicate. Narrow by
        // design (empty + recent only) so it can never fold a genuine prior recording.
        match MeetingsRepository::find_adoptable_calendar_meeting(pool, event_id).await {
            Ok(Some(existing_id)) => {
                log_info!(
                    "api_create_meeting: reusing existing empty calendar-linked meeting {} for event {} (WS2.1 dedupe)",
                    existing_id,
                    event_id
                );
                if let Some(k) = calendar_series_key
                    .as_deref()
                    .filter(|k| !k.trim().is_empty())
                {
                    let _ =
                        MeetingsRepository::set_calendar_series_key(pool, &existing_id, Some(k))
                            .await;
                }
                return Ok(serde_json::json!({ "meeting_id": existing_id }));
            }
            Ok(None) => {}
            Err(e) => {
                // Non-fatal: a dedupe lookup failure must never block creating the meeting.
                log_error!("api_create_meeting: adoptable-meeting lookup failed (continuing to create): {}", e);
            }
        }
    }

    match MeetingsRepository::create_meeting(
        pool,
        meeting_title,
        folder_path,
        origin,
        calendar_event_id,
        started_at,
    )
    .await
    {
        Ok(meeting_id) => {
            log_info!("Successfully created meeting with id: {}", meeting_id);
            // specs/0036: stamp the series key so this recording groups with its recurring
            // series going forward. Non-fatal — a failure here must not fail the create.
            if let Some(k) = calendar_series_key
                .as_deref()
                .filter(|k| !k.trim().is_empty())
            {
                if let Err(e) =
                    MeetingsRepository::set_calendar_series_key(pool, &meeting_id, Some(k)).await
                {
                    log_error!(
                        "api_create_meeting: failed to set series key (non-fatal): {}",
                        e
                    );
                }
            }
            // Note (specs/0017): the participant roster is NOT seeded here. Seeding needs a
            // blocking EventKit calendar read keyed off `calendar_event_id`, which doesn't
            // fit this create command cleanly; instead `api_get_meeting_participants` seeds
            // lazily on first view (idempotent), which is the required safety net.
            Ok(serde_json::json!({ "meeting_id": meeting_id }))
        }
        Err(e) => {
            log_error!("Error creating meeting: {}", e);
            Err(format!("Failed to create meeting: {}", e))
        }
    }
}

/// Opens the meeting's recording folder in the system file explorer
#[tauri::command]
pub async fn open_meeting_folder<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<(), String> {
    log_info!("open_meeting_folder called for meeting_id: {}", meeting_id);

    let pool = state.db_manager.pool();

    // Get meeting with folder_path
    let meeting: Option<MeetingModel> = sqlx::query_as(
        "SELECT id, title, created_at, updated_at, folder_path, origin, calendar_event_id, title_manually_set FROM meetings WHERE id = ?",
    )
    .bind(&meeting_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    match meeting {
        Some(m) => {
            if let Some(folder_path) = m.folder_path {
                log_info!("Opening meeting folder: {}", folder_path);

                // Verify folder exists
                let path = std::path::Path::new(&folder_path);
                if !path.exists() {
                    log_warn!("Folder path does not exist: {}", folder_path);
                    return Err(format!("Recording folder not found: {}", folder_path));
                }

                // Open folder based on OS
                #[cfg(target_os = "macos")]
                {
                    std::process::Command::new("open")
                        .arg(&folder_path)
                        .spawn()
                        .map_err(|e| format!("Failed to open folder: {}", e))?;
                }

                #[cfg(target_os = "windows")]
                {
                    std::process::Command::new("explorer")
                        .arg(&folder_path)
                        .spawn()
                        .map_err(|e| format!("Failed to open folder: {}", e))?;
                }

                #[cfg(target_os = "linux")]
                {
                    std::process::Command::new("xdg-open")
                        .arg(&folder_path)
                        .spawn()
                        .map_err(|e| format!("Failed to open folder: {}", e))?;
                }

                log_info!("Successfully opened folder: {}", folder_path);
                Ok(())
            } else {
                log_warn!("Meeting {} has no folder_path set", meeting_id);
                Err("Recording folder path not available for this meeting".to_string())
            }
        }
        None => {
            log_warn!("Meeting not found: {}", meeting_id);
            Err("Meeting not found".to_string())
        }
    }
}
