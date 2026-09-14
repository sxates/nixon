// Transcript-save command + the shared `TranscriptSegment` IPC/persistence DTO
// (moved from api/api.rs in specs/0042 WS3).

use log::{debug as log_debug, error as log_error, info as log_info, warn as log_warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

use crate::{database::repositories::transcript::TranscriptsRepository, state::AppState};

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveTranscriptRequest {
    pub meeting_title: String,
    pub transcripts: Vec<TranscriptSegment>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub id: String,
    pub text: String,
    pub timestamp: String,
    // NEW: Recording-relative timestamps for playback synchronization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_start_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_end_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Per-meeting speaker key (specs/0010); NULL until diarized. Defaulted so the
    /// existing save payloads (which omit it) still deserialize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    /// Capture-channel tag (specs/0029 WS3.4): "microphone" | "system" | "mixed",
    /// from per-window RMS dominance — computed at capture time for live sessions,
    /// or re-derived from the per-channel WAVs by batch retranscription (1.10
    /// feedback). NULL for legacy rows and sources with no per-channel capture
    /// (imports). Defaulted so older payloads still deserialize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Per-word timestamps (specs/0046 WS2), JSON `[{"w":..,"s":..,"e":..}, ..]`.
    /// Populated on the Parakeet batch path (`transcribe_audio_timestamped`);
    /// NULL for Whisper-transcribed and legacy rows, which carry no per-word
    /// timing. Consumed by the offline diarization split (Task 7) to snap
    /// speaker-turn boundaries to word edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_timestamps: Option<String>,
}

#[tauri::command]
// Tauri command whose args are the IPC payload (specs/0037 added resume routing).
#[allow(clippy::too_many_arguments)]
pub async fn api_save_transcript<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_title: String,
    transcripts: Vec<serde_json::Value>,
    folder_path: Option<String>,
    meeting_id: Option<String>,
    // specs/0037: a resumed session APPENDS its segments to `meeting_id` (bypassing the
    // WS6.7 duplicate-row guard) with `audio_offset_seconds` shifting their recording-
    // relative times onto the meeting's continuous timeline. Absent/false = save as today.
    resumed: Option<bool>,
    audio_offset_seconds: Option<f64>,
) -> Result<serde_json::Value, String> {
    log_info!(
        "api_save_transcript called for meeting: {}, transcripts: {}, folder_path: {:?}, meeting_id: {:?}",
        meeting_title,
        transcripts.len(),
        folder_path,
        meeting_id
    );

    // Log first transcript for debugging
    if let Some(first) = transcripts.first() {
        log_debug!(
            "First transcript data: {}",
            serde_json::to_string_pretty(first).unwrap_or_default()
        );
    }

    // Convert serde_json::Value to TranscriptSegment
    let transcripts_to_save: Vec<TranscriptSegment> = transcripts
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            log_error!("Failed to parse transcript segments: {}", e);
            format!(
                "Invalid transcript data format: {}. Please check the data structure.",
                e
            )
        })?;

    // Log parsed segments count and first segment details
    if let Some(first_seg) = transcripts_to_save.first() {
        log_debug!("First parsed segment: text='{}', audio_start_time={:?}, audio_end_time={:?}, duration={:?}",
                   first_seg.text.chars().take(50).collect::<String>(),
                   first_seg.audio_start_time,
                   first_seg.audio_end_time,
                   first_seg.duration);
    }

    let pool = state.db_manager.pool();

    // Two paths:
    // - meeting_id = Some(existing): attach segments to the meeting created at
    //   recording START (specs/0007), refresh title/updated_at, do NOT create a row.
    // - meeting_id = None (existing callers): behave exactly as before -> create a
    //   new meeting with the segments.
    if let Some(existing_id) = meeting_id.as_deref().filter(|id| !id.trim().is_empty()) {
        // specs/0037: a RESUMED session appends to the SAME meeting (the WS6.7 guard is
        // for the duplicate-row race, not an intentional continue). On success we return;
        // if the meeting is missing we fall through to create-new so nothing is lost.
        if resumed.unwrap_or(false) {
            let offset = audio_offset_seconds.unwrap_or(0.0);
            match TranscriptsRepository::append_transcripts_for_meeting(
                pool,
                existing_id,
                &transcripts_to_save,
                folder_path.clone(),
                offset,
            )
            .await
            {
                Ok(true) => {
                    log_info!(
                        "Appended resumed transcript to meeting {} (offset {:.3}s)",
                        existing_id,
                        offset
                    );
                    return Ok(serde_json::json!({
                        "status": "success",
                        "message": "Resumed transcript appended successfully",
                        "meeting_id": existing_id
                    }));
                }
                Ok(false) => {
                    log_warn!(
                        "Resume append: meeting {} not found; creating a new meeting instead",
                        existing_id
                    );
                }
                Err(e) => {
                    log_error!(
                        "Error appending resumed transcript to {}: {}",
                        existing_id,
                        e
                    );
                    return Err(format!("Failed to append resumed transcript: {}", e));
                }
            }
        }
        match TranscriptsRepository::save_transcripts_for_meeting(
            pool,
            existing_id,
            &meeting_title,
            &transcripts_to_save,
            folder_path.clone(),
        )
        .await
        {
            Ok(true) => {
                log_info!(
                    "Successfully attached transcript to existing meeting {}",
                    existing_id
                );
                return Ok(serde_json::json!({
                    "status": "success",
                    "message": "Transcript saved successfully",
                    "meeting_id": existing_id
                }));
            }
            Ok(false) => {
                // The save did NOT attach to the existing row — either the meeting
                // no longer exists, OR it already holds another session's transcripts
                // and we refused to merge (specs/0019 WS6.7). Either way, fall through
                // to creating a fresh meeting so this session lands in its own row and
                // is never lost or merged.
                log_warn!(
                    "Did not attach to meeting_id {} (missing, or already populated by \
                     another session); creating a new meeting for this transcript",
                    existing_id
                );
            }
            Err(e) => {
                log_error!(
                    "Error attaching transcript to meeting {}: {}",
                    existing_id,
                    e
                );
                return Err(format!("Failed to save transcript: {}", e));
            }
        }
    }

    // Default path: create a new meeting with the segments.
    match TranscriptsRepository::save_transcript(
        pool,
        &meeting_title,
        &transcripts_to_save,
        folder_path,
    )
    .await
    {
        Ok(meeting_id) => {
            log_info!(
                "Successfully saved transcript and created meeting with id: {}",
                meeting_id
            );
            Ok(serde_json::json!({
                "status": "success",
                "message": "Transcript saved successfully",
                "meeting_id": meeting_id
            }))
        }
        Err(e) => {
            log_error!(
                "Error saving transcript for meeting '{}': {}",
                meeting_title,
                e
            );
            Err(format!("Failed to save transcript: {}", e))
        }
    }
}
