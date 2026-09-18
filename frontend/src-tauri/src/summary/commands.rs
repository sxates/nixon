use crate::database::repositories::{
    meeting::MeetingsRepository, meeting_note::MeetingNotesRepository, setting::SettingsRepository,
    summary::SummaryProcessesRepository, transcript::TranscriptsRepository,
    transcript_chunk::TranscriptChunksRepository,
};
use crate::llm_activity::Origin;
use crate::state::AppState;
use crate::summary::language_detection::{detect_summary_language, SummaryLanguageDetection};
use crate::summary::metadata::{
    read_detected_summary_language_from_metadata, read_summary_language_from_metadata,
    write_detected_summary_language_to_metadata, write_summary_language_to_metadata,
};
use crate::summary::service::SummaryService;
use log::{error as log_error, info as log_info, warn as log_warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};

#[derive(Debug, Serialize, Deserialize)]
pub struct SummaryResponse {
    pub status: String,
    #[serde(rename = "meetingName")]
    pub meeting_name: Option<String>,
    pub meeting_id: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessTranscriptResponse {
    pub message: String,
    pub process_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryLanguageStorage {
    Metadata,
    LocalFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummaryLanguagePreference {
    pub language: Option<String>,
    pub storage: SummaryLanguageStorage,
}

impl MeetingSummaryLanguagePreference {
    fn metadata(language: Option<String>) -> Self {
        Self {
            language,
            storage: SummaryLanguageStorage::Metadata,
        }
    }

    fn local_fallback() -> Self {
        Self {
            language: None,
            storage: SummaryLanguageStorage::LocalFallback,
        }
    }
}

enum MeetingFolderResolution {
    Folder(PathBuf),
    NoFolder,
}

/// Saves a meeting summary (Native SQLx implementation)
///
/// Expected format: { "markdown": "...", "summary_json": [...BlockNote blocks...] }
#[tauri::command]
pub async fn api_save_meeting_summary<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    summary: serde_json::Value,
    _auth_token: Option<String>,
) -> Result<serde_json::Value, String> {
    log_info!(
        "api_save_meeting_summary (native) called for meeting_id: {}",
        meeting_id
    );
    let pool = state.db_manager.pool();

    match SummaryProcessesRepository::update_meeting_summary(pool, &meeting_id, &summary).await {
        Ok(true) => {
            log_info!("Summary saved successfully for meeting_id: {}", meeting_id);
            Ok(serde_json::json!({
                "message": "Meeting summary saved successfully"
            }))
        }
        Ok(false) => {
            log_warn!(
                "Meeting not found or invalid JSON for meeting_id: {}",
                meeting_id
            );
            Err("Meeting not found or can't convert the json".into())
        }
        Err(e) => {
            log_error!("Failed to save meeting summary for {}: {}", meeting_id, e);
            Err(e.to_string())
        }
    }
}

/// Gets the per-meeting summary language override from metadata.json.
#[tauri::command]
pub async fn api_get_meeting_summary_language<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingSummaryLanguagePreference, String> {
    log_info!(
        "api_get_meeting_summary_language called for meeting_id: {}",
        meeting_id
    );

    match resolve_meeting_folder(state.db_manager.pool(), &meeting_id).await? {
        MeetingFolderResolution::Folder(folder) => read_summary_language_from_metadata(&folder)
            .map(MeetingSummaryLanguagePreference::metadata)
            .map_err(|e| e.to_string()),
        MeetingFolderResolution::NoFolder => Ok(MeetingSummaryLanguagePreference::local_fallback()),
    }
}

/// Saves or clears the per-meeting summary language override in metadata.json.
#[tauri::command]
pub async fn api_save_meeting_summary_language<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    summary_language: Option<String>,
) -> Result<MeetingSummaryLanguagePreference, String> {
    log_info!(
        "api_save_meeting_summary_language called for meeting_id: {}, language: {:?}",
        meeting_id,
        summary_language
    );

    match resolve_meeting_folder(state.db_manager.pool(), &meeting_id).await? {
        MeetingFolderResolution::Folder(folder) => {
            write_summary_language_to_metadata(&folder, summary_language.as_deref())
                .map_err(|e| e.to_string())?;
            read_summary_language_from_metadata(&folder)
                .map(MeetingSummaryLanguagePreference::metadata)
                .map_err(|e| e.to_string())
        }
        MeetingFolderResolution::NoFolder => Ok(MeetingSummaryLanguagePreference::local_fallback()),
    }
}

/// Gets the cached Auto-detected summary language from metadata.json.
#[tauri::command]
pub async fn api_get_meeting_detected_summary_language<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingSummaryLanguagePreference, String> {
    log_info!(
        "api_get_meeting_detected_summary_language called for meeting_id: {}",
        meeting_id
    );

    match resolve_meeting_folder(state.db_manager.pool(), &meeting_id).await? {
        MeetingFolderResolution::Folder(folder) => {
            read_detected_summary_language_from_metadata(&folder)
                .map(MeetingSummaryLanguagePreference::metadata)
                .map_err(|e| e.to_string())
        }
        MeetingFolderResolution::NoFolder => Ok(MeetingSummaryLanguagePreference::local_fallback()),
    }
}

/// Saves or clears the cached Auto-detected summary language in metadata.json.
#[tauri::command]
pub async fn api_save_meeting_detected_summary_language<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    detected_summary_language: Option<String>,
) -> Result<MeetingSummaryLanguagePreference, String> {
    log_info!(
        "api_save_meeting_detected_summary_language called for meeting_id: {}, language: {:?}",
        meeting_id,
        detected_summary_language
    );

    match resolve_meeting_folder(state.db_manager.pool(), &meeting_id).await? {
        MeetingFolderResolution::Folder(folder) => {
            write_detected_summary_language_to_metadata(
                &folder,
                detected_summary_language.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            read_detected_summary_language_from_metadata(&folder)
                .map(MeetingSummaryLanguagePreference::metadata)
                .map_err(|e| e.to_string())
        }
        MeetingFolderResolution::NoFolder => Ok(MeetingSummaryLanguagePreference::local_fallback()),
    }
}

/// Detects the dominant supported summary language from transcript segments.
#[tauri::command]
pub async fn api_detect_transcript_summary_language(
    transcript_texts: Vec<String>,
) -> Result<SummaryLanguageDetection, String> {
    Ok(detect_summary_language(&transcript_texts))
}

async fn resolve_meeting_folder(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<MeetingFolderResolution, String> {
    let meeting = MeetingsRepository::get_meeting_metadata(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load meeting metadata: {}", e))?
        .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    let Some(folder_path) = meeting.folder_path.filter(|p| !p.trim().is_empty()) else {
        return Ok(MeetingFolderResolution::NoFolder);
    };

    Ok(MeetingFolderResolution::Folder(PathBuf::from(folder_path)))
}

/// Gets summary status and data (Native SQLx implementation)
///
/// Returns summary status (pending/processing/completed/failed) and parsed result data
#[tauri::command]
pub async fn api_get_summary<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    _auth_token: Option<String>,
) -> Result<SummaryResponse, String> {
    log_info!(
        "api_get_summary (native) called for meeting_id: {}",
        meeting_id
    );
    let pool = state.db_manager.pool();

    match SummaryProcessesRepository::get_summary_data_for_meeting(pool, &meeting_id).await {
        Ok(Some(process)) => {
            let status = process.status.to_lowercase();
            let error = process.error;

            // Parse result data if it exists (regardless of status)
            // This allows displaying restored summaries after cancellation or failure
            let data = if let Some(result_str) = process.result {
                match serde_json::from_str::<serde_json::Value>(&result_str) {
                    Ok(parsed) => Some(parsed),
                    Err(e) => {
                        log_error!("Failed to parse summary result JSON: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            // Fetch meeting title from database
            let meeting_name = match MeetingsRepository::get_meeting(pool, &meeting_id).await {
                Ok(Some(meeting_details)) => {
                    log_info!("Fetched meeting title: {}", &meeting_details.title);
                    Some(meeting_details.title)
                }
                Ok(None) => {
                    log_warn!("Meeting not found for meeting_id: {}", meeting_id);
                    None
                }
                Err(e) => {
                    log_error!("Failed to fetch meeting title: {}", e);
                    None
                }
            };

            let response = SummaryResponse {
                status: status.clone(),
                meeting_name,
                meeting_id: meeting_id.clone(),
                start: process.start_time.map(|t| t.to_rfc3339()),
                end: process.end_time.map(|t| t.to_rfc3339()),
                data,
                error,
            };

            log_info!(
                "Summary status for {}: {}, has_data: {}, meeting_name: {:?}",
                meeting_id,
                status,
                response.data.is_some(),
                response.meeting_name
            );
            Ok(response)
        }
        Ok(None) => {
            log_info!("No summary process found for meeting_id: {}", meeting_id);

            // Still fetch meeting title for idle state
            let meeting_name = match MeetingsRepository::get_meeting(pool, &meeting_id).await {
                Ok(Some(meeting_details)) => Some(meeting_details.title),
                _ => None,
            };

            Ok(SummaryResponse {
                status: "idle".to_string(),
                meeting_name,
                meeting_id,
                start: None,
                end: None,
                data: None,
                error: None,
            })
        }
        Err(e) => {
            log_error!("Error retrieving summary for {}: {}", meeting_id, e);
            Err(format!("Failed to retrieve summary: {}", e))
        }
    }
}

/// Processes transcript and generates summary (Native SQLx implementation)
///
/// Spawns a background task and returns immediately with process_id
#[tauri::command]
#[allow(clippy::too_many_arguments)] // cohesive param set; refactor deferred
pub async fn api_process_transcript<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    text: String,
    model: String,
    model_name: String,
    meeting_id: Option<String>,
    _chunk_size: Option<i32>,
    _overlap: Option<i32>,
    custom_prompt: Option<String>,
    template_id: Option<String>,
    summary_language: Option<String>,
    _auth_token: Option<String>,
) -> Result<ProcessTranscriptResponse, String> {
    use uuid::Uuid;

    let m_id = meeting_id.unwrap_or_else(|| format!("meeting-{}", Uuid::new_v4()));
    log_info!(
        "api_process_transcript (native) called for meeting_id: {}, model: {}",
        &m_id,
        &model
    );

    let pool = state.db_manager.pool().clone();
    let final_prompt = custom_prompt.unwrap_or_default();

    // Template resolution (specs/0020 task 4): an explicit caller choice wins;
    // otherwise honor the meeting's persisted `meetings.template_id`; otherwise
    // the shared default. For a brand-new meeting id (no `meeting_id` supplied,
    // so `m_id` has no row yet) the DB read finds nothing and we land on the
    // default. A DB error degrades to the default rather than blocking the
    // summary — the choice is metadata, not a hard dependency.
    let final_template_id = match template_id
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
    {
        Some(explicit) => explicit,
        None => MeetingsRepository::get_meeting_template(&pool, &m_id)
            .await
            .unwrap_or_else(|e| {
                log_warn!(
                    "Failed to read persisted template for {}; using default: {}",
                    &m_id,
                    e
                );
                None
            })
            .unwrap_or_else(|| crate::summary::templates::DEFAULT_SUMMARY_TEMPLATE_ID.to_string()),
    };

    // Normalise empty / whitespace-only to None so "" and null behave identically
    let summary_language = summary_language.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    });

    // Create or reset the process entry in the database
    SummaryProcessesRepository::create_or_reset_process(&pool, &m_id)
        .await
        .map_err(|e| format!("Failed to initialize process: {}", e))?;

    log_info!("✓ Summary process initialized for meeting_id: {}", &m_id);

    // Save transcript chunks data (matching Python backend behavior)
    let chunk_size = _chunk_size.unwrap_or(40000);
    let overlap = _overlap.unwrap_or(1000);

    TranscriptChunksRepository::save_transcript_data(
        &pool,
        &m_id,
        &text,
        &model,
        &model_name,
        chunk_size,
        overlap,
    )
    .await
    .map_err(|e| format!("Failed to save transcript data: {}", e))?;

    log_info!("✓ Transcript chunks saved for meeting_id: {}", &m_id);

    // Spawn background task for actual processing
    let meeting_id_clone = m_id.clone();
    tauri::async_runtime::spawn(async move {
        SummaryService::process_transcript_background(
            app,
            pool,
            meeting_id_clone.clone(),
            text,
            model,
            model_name,
            final_prompt,
            final_template_id,
            summary_language,
            // specs/0063 W3 fix round 1 (I1): this command is invoked from the
            // Generate/Regenerate button the user is watching live
            // (`useSummaryGeneration.ts`), which already renders its own
            // `ChunkProgressDisplay` — Foreground keeps it out of the footer queue's
            // running list so it is never shown twice.
            Origin::Foreground,
        )
        .await;
    });

    log_info!("🚀 Background task spawned for meeting_id: {}", &m_id);

    Ok(ProcessTranscriptResponse {
        message: "Summary generation started".to_string(),
        process_id: m_id,
    })
}

/// Kicks off summary (re)generation for an already-saved meeting WITHOUT the
/// frontend supplying the transcript text or model — used by the Day Agenda's
/// one-click "Summarize" action (specs/0012), which never opens the meeting.
///
/// Unlike [`api_process_transcript`] (which the meeting-detail view drives with the
/// in-memory transcript + the currently-selected model), this command resolves
/// everything from the DB: the saved transcript via
/// [`TranscriptsRepository::get_full_transcript`] and the provider/model from the
/// saved model config. It then reuses the exact background summary path
/// ([`SummaryService::process_transcript_background`]), so diarization-aware
/// attribution, language detection, and result persistence all apply unchanged.
///
/// Returns immediately with the process id; progress/result arrive via the same
/// `api_get_summary` polling the detail view uses.
#[tauri::command]
pub async fn api_generate_summary_for_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<ProcessTranscriptResponse, String> {
    if meeting_id.trim().is_empty() {
        return Err("meeting_id cannot be empty".to_string());
    }
    log_info!(
        "api_generate_summary_for_meeting called for meeting_id: {}",
        meeting_id
    );

    let pool = state.db_manager.pool().clone();
    start_summary_generation_for_meeting(app, pool, meeting_id).await
}

/// Backend-callable body of [`api_generate_summary_for_meeting`]: resolves the saved
/// transcript, the configured provider/model, and the meeting's persisted template from
/// the DB, then spawns the shared background summary path. Extracted (specs/0041 WS2) so
/// the post-diarization summary-refresh trigger (`summary::refresh`) re-runs the summary
/// through EXACTLY the code path auto-summary/one-click summarize use — same template
/// resolution, same provider config, same persistence and action-item re-extraction.
pub async fn start_summary_generation_for_meeting<R: Runtime>(
    app: AppHandle<R>,
    pool: sqlx::SqlitePool,
    meeting_id: String,
) -> Result<ProcessTranscriptResponse, String> {
    // Resolve the saved transcript text for this meeting.
    let text = TranscriptsRepository::get_full_transcript(&pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load transcript: {}", e))?;
    // Notes-only degrade (spec 0015 Phase B, Task 6): a `notes_only` meeting has
    // no transcript and no recording folder, but it DOES have user notes. In that
    // case we still summarize — grounding on the notes alone — instead of
    // aborting. The empty-transcript + notes path runs the same
    // `process_transcript_background` → notes-grounding final synthesis pass
    // (`processor::generate_meeting_summary`), which already routes empty `text`
    // to the single-pass final report with `<user_notes>` set. We only block when
    // there is BOTH no transcript AND no notes — there is genuinely nothing to
    // summarize. Behavior for a meeting WITH a transcript is unchanged.
    if text.trim().is_empty() {
        let has_notes = MeetingNotesRepository::get_notes(&pool, &meeting_id)
            .await
            .map_err(|e| format!("Failed to load notes: {}", e))?
            .and_then(|note| note.notes_markdown)
            .map(|m| !m.trim().is_empty())
            .unwrap_or(false);
        if !has_notes {
            return Err(
                "This meeting has no transcript or notes yet, so it can't be summarized."
                    .to_string(),
            );
        }
        log_info!(
            "Notes-only summary: meeting_id {} has no transcript; grounding on user notes",
            meeting_id
        );
    }

    // Resolve the configured summary provider/model. The background task fetches
    // the API key / endpoint itself, so we only need provider + model here.
    let config = SettingsRepository::get_model_config(&pool)
        .await
        .map_err(|e| format!("Failed to load model configuration: {}", e))?
        .ok_or_else(|| {
            "No summary model is configured. Choose a provider and model in Settings first."
                .to_string()
        })?;
    let model = config.provider;
    let model_name = config.model;

    // Mirror api_process_transcript's setup so the detail view sees a consistent
    // process row + transcript chunks.
    SummaryProcessesRepository::create_or_reset_process(&pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to initialize summary process: {}", e))?;

    TranscriptChunksRepository::save_transcript_data(
        &pool,
        &meeting_id,
        &text,
        &model,
        &model_name,
        40000,
        1000,
    )
    .await
    .map_err(|e| format!("Failed to save transcript data: {}", e))?;

    // Honor the meeting's persisted template choice (specs/0020 task 4); fall
    // back to the shared default when none is set. A DB error degrades to the
    // default rather than blocking the one-click/auto-summarize path.
    let template_id = MeetingsRepository::get_meeting_template(&pool, &meeting_id)
        .await
        .unwrap_or_else(|e| {
            log_warn!(
                "Failed to read persisted template for {}; using default: {}",
                meeting_id,
                e
            );
            None
        })
        .unwrap_or_else(|| crate::summary::templates::DEFAULT_SUMMARY_TEMPLATE_ID.to_string());

    let meeting_id_clone = meeting_id.clone();
    tauri::async_runtime::spawn(async move {
        SummaryService::process_transcript_background(
            app,
            pool,
            meeting_id_clone,
            text,
            model,
            model_name,
            String::new(), // no custom prompt
            template_id,   // persisted per-meeting choice, else DEFAULT_SUMMARY_TEMPLATE_ID
            None,          // auto-detect summary language
            // specs/0063 W3 fix round 1 (I1): this path has no live viewer — it is
            // the Day Agenda's one-click "Summarize" (never opens the meeting), so
            // Background is correct here.
            Origin::Background,
        )
        .await;
    });

    log_info!(
        "🚀 Background summary task spawned for meeting_id: {}",
        meeting_id
    );
    Ok(ProcessTranscriptResponse {
        message: "Summary generation started".to_string(),
        process_id: meeting_id,
    })
}

/// Cancels an ongoing summary generation process
///
/// This command triggers the cancellation token for the specified meeting,
/// stopping the summary generation gracefully.
#[tauri::command]
pub async fn api_cancel_summary<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<serde_json::Value, String> {
    log_info!("api_cancel_summary called for meeting_id: {}", meeting_id);

    // Trigger cancellation via the service
    let cancelled = SummaryService::cancel_summary(&meeting_id);

    if cancelled {
        // Update database status to cancelled
        let pool = state.db_manager.pool();
        if let Err(e) =
            SummaryProcessesRepository::update_process_cancelled(pool, &meeting_id).await
        {
            log_error!(
                "Failed to update DB status to cancelled for {}: {}",
                meeting_id,
                e
            );
            return Err(format!("Failed to update cancellation status: {}", e));
        }

        log_info!(
            "Successfully cancelled summary generation for meeting_id: {}",
            meeting_id
        );
        Ok(serde_json::json!({
            "message": "Summary generation cancelled successfully",
            "meeting_id": meeting_id,
        }))
    } else {
        log_warn!(
            "No active summary generation found for meeting_id: {}",
            meeting_id
        );
        Ok(serde_json::json!({
            "message": "No active summary generation to cancel",
            "meeting_id": meeting_id,
        }))
    }
}
