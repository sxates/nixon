//! Tauri commands for the audio lifecycle (specs/0072 §"Tauri IPC"). Registered in
//! `registry.rs`.

use serde::Serialize;
use tauri::{AppHandle, Runtime};

use super::hooks::{self, pool};
use super::policy::{AudioRetention, AudioState};
use super::state::{self, scan_folder};
use super::sweep::{self, RetentionPreview, RetentionReport, SweepOptions};

/// What audio a meeting still has, per capability: "Identify speakers" needs `channels`,
/// "Enhance" (retranscribe) needs `mix`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingAudioStatus {
    pub mix: bool,
    /// The system channel speaker identification reads, as WAV or Opus.
    pub channels: bool,
    /// The channels are stored compressed (Opus).
    pub compressed: bool,
    pub state: AudioState,
}

/// Replaces the stop path's inline auto-diarize: diarize if that applies and hasn't run
/// (its completion hook reevaluates), otherwise reevaluate now. Idempotent.
#[tauri::command]
pub async fn api_finish_audio_processing<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<(), String> {
    if meeting_id.trim().is_empty() {
        return Err("meeting_id cannot be empty".to_string());
    }
    hooks::finish_processing(&app, &meeting_id, false).await;
    Ok(())
}

/// Dry run of the policy over every meeting with `policy` as the candidate. Deletes nothing.
#[tauri::command]
pub async fn api_preview_audio_retention<R: Runtime>(
    app: AppHandle<R>,
    policy: AudioRetention,
) -> Result<RetentionPreview, String> {
    let Some(pool) = pool(&app) else {
        return Ok(RetentionPreview::default()); // no database, no meetings
    };
    sweep::preview(&pool, policy, chrono::Utc::now())
        .await
        .map_err(|e| format!("Couldn't check which audio would be deleted: {e:#}"))
}

/// Run the sweep now with the SAVED policy (after the user confirmed). Deletes only;
/// compression stays with the background sweep.
#[tauri::command]
pub async fn api_apply_audio_retention_now<R: Runtime>(
    app: AppHandle<R>,
) -> Result<RetentionReport, String> {
    if pool(&app).is_none() {
        return Ok(RetentionReport::default());
    }
    hooks::sweep_now(&app, &SweepOptions::default()).await
}

/// The meeting page's audio status (replaces the WAV-counts-as-audio check).
#[tauri::command]
pub async fn api_meeting_audio_status<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<MeetingAudioStatus, String> {
    let pool = pool(&app).ok_or("The database isn't ready yet.")?;
    let row = state::load_row(&pool, &meeting_id)
        .await
        .map_err(|e| format!("{e:#}"))?;
    let state = row.as_ref().map_or(AudioState::Pending, |r| r.state());
    let audio = row
        .and_then(|r| r.folder())
        .and_then(|folder| scan_folder(&folder))
        .unwrap_or_default();
    Ok(MeetingAudioStatus {
        mix: audio.mix,
        channels: audio.system_channel,
        compressed: audio.opus_channels,
        state,
    })
}

/// Does this meeting still have audio? Thin wrapper (`mix || channels`) kept until its
/// callers move to [`api_meeting_audio_status`].
#[tauri::command]
pub async fn api_meeting_audio_available<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<bool, String> {
    let status = api_meeting_audio_status(app, meeting_id).await?;
    Ok(status.mix || status.channels)
}
