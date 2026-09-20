// audio/capture_commands.rs
//
// Top-level recording / device / language Tauri commands, moved out of lib.rs
// (specs/0042 WS1). These are the frontend-facing wrappers around
// `recording_commands` / `simple_level_monitor` — command names and serde
// shapes are unchanged from when they lived in lib.rs.

use log::{error as log_error, info as log_info};
use serde::{Deserialize, Serialize};
use std::sync::Mutex as StdMutex;
use tauri::{AppHandle, Runtime};

use crate::notifications;
use crate::tray;

use super::{list_audio_devices, trigger_audio_permission, AudioDevice};

// specs/0028: the former `RECORDING_FLAG: AtomicBool` was a write-only second source of
// truth for recording state — it was never read. Recording state now has a single source
// of truth: `audio::recording_commands::is_recording()` (backed by the audio module's
// `RecordingState`), which both `is_recording` and the tray already consult.

// Global language preference storage (default to "auto-translate" for automatic translation to English)
static LANGUAGE_PREFERENCE: std::sync::LazyLock<StdMutex<String>> =
    std::sync::LazyLock::new(|| StdMutex::new("auto-translate".to_string()));

#[derive(Debug, Deserialize)]
pub struct RecordingArgs {
    save_path: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct TranscriptionStatus {
    chunks_in_queue: usize,
    is_processing: bool,
    last_activity_ms: u64,
}

#[tauri::command]
pub async fn start_recording<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
    system_device_name: Option<String>,
    meeting_name: Option<String>,
) -> Result<(), String> {
    log_info!("🔥 CALLED start_recording with meeting: {:?}", meeting_name);
    log_info!(
        "📋 Backend received parameters - mic: {:?}, system: {:?}, meeting: {:?}",
        mic_device_name,
        system_device_name,
        meeting_name
    );

    if is_recording().await {
        return Err("Recording already in progress".to_string());
    }

    // Call the actual audio recording system with meeting name
    match super::recording_commands::start_recording_with_devices_and_meeting(
        app.clone(),
        mic_device_name,
        system_device_name,
        meeting_name.clone(),
        None, // meeting_id: this legacy wrapper doesn't resume (specs/0037)
        None, // resume_folder_path
    )
    .await
    {
        Ok(_) => {
            tray::update_tray_menu(&app);

            log_info!("Recording started successfully");

            // Banner: the recording may have been started from a notification button
            // while Nixon has no visible window, and this is the only sign of it
            // (specs/0068 — before that, this call could not produce one).
            notifications::os_commands::recording_banner(
                &app,
                "Recording started",
                meeting_name.as_deref().unwrap_or("Meeting"),
                "nixon-recording",
            )
            .await;

            Ok(())
        }
        Err(e) => {
            log_error!("Failed to start audio recording: {}", e);
            Err(format!("Failed to start recording: {}", e))
        }
    }
}

#[tauri::command]
pub async fn stop_recording<R: Runtime>(
    app: AppHandle<R>,
    args: RecordingArgs,
) -> Result<serde_json::Value, String> {
    log_info!("Attempting to stop recording...");

    // Check the actual audio recording system state instead of the flag
    if !super::recording_commands::is_recording().await {
        log_info!("Recording is already stopped");
        return Ok(serde_json::json!({ "message": "Recording is already stopped" }));
    }

    // Call the actual audio recording system to stop
    match super::recording_commands::stop_recording(
        app.clone(),
        super::recording_commands::RecordingArgs {
            save_path: args.save_path.clone(),
        },
    )
    .await
    {
        // specs/0037: pass the stop info (folder_path, meeting_name, resumed,
        // prior_audio_duration_seconds) through as the command's resolved value —
        // the frontend's race-free primary source for resume-save routing.
        Ok(stop_info) => {
            tray::update_tray_menu(&app);

            // Create the save directory if it doesn't exist
            if let Some(parent) = std::path::Path::new(&args.save_path).parent() {
                if !parent.exists() {
                    log_info!("Creating directory: {:?}", parent);
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        let err_msg = format!("Failed to create save directory: {}", e);
                        log_error!("{}", err_msg);
                        return Err(err_msg);
                    }
                }
            }

            // Same id as the start banner, so the stop notice replaces it rather than
            // leaving two entries for one recording in Notification Center.
            notifications::os_commands::recording_banner(
                &app,
                "Recording stopped",
                "Nixon is processing the meeting.",
                "nixon-recording",
            )
            .await;

            Ok(stop_info)
        }
        Err(e) => {
            log_error!("Failed to stop audio recording: {}", e);
            tray::update_tray_menu(&app);
            Err(format!("Failed to stop recording: {}", e))
        }
    }
}

#[tauri::command]
pub async fn is_recording() -> bool {
    super::recording_commands::is_recording().await
}

#[tauri::command]
pub fn get_transcription_status() -> TranscriptionStatus {
    TranscriptionStatus {
        chunks_in_queue: 0,
        is_processing: false,
        last_activity_ms: 0,
    }
}

// Audio level monitoring commands
#[tauri::command]
pub async fn start_audio_level_monitoring<R: Runtime>(
    app: AppHandle<R>,
    device_names: Vec<String>,
) -> Result<(), String> {
    log_info!(
        "Starting audio level monitoring for devices: {:?}",
        device_names
    );

    super::simple_level_monitor::start_monitoring(app, device_names)
        .await
        .map_err(|e| format!("Failed to start audio level monitoring: {}", e))
}

#[tauri::command]
pub async fn stop_audio_level_monitoring() -> Result<(), String> {
    log_info!("Stopping audio level monitoring");

    super::simple_level_monitor::stop_monitoring()
        .await
        .map_err(|e| format!("Failed to stop audio level monitoring: {}", e))
}

#[tauri::command]
pub async fn is_audio_level_monitoring() -> bool {
    super::simple_level_monitor::is_monitoring()
}

// Whisper commands are now handled by whisper_engine::commands module

#[tauri::command]
pub async fn get_audio_devices() -> Result<Vec<AudioDevice>, String> {
    list_audio_devices()
        .await
        .map_err(|e| format!("Failed to list audio devices: {}", e))
}

#[tauri::command]
pub async fn trigger_microphone_permission() -> Result<bool, String> {
    trigger_audio_permission()
        .map_err(|e| format!("Failed to trigger microphone permission: {}", e))
}

#[tauri::command]
pub async fn start_recording_with_devices_and_meeting<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
    system_device_name: Option<String>,
    meeting_name: Option<String>,
    // specs/0037: the DB row to attach this recording to (written into metadata.json
    // for crash recovery); when `resume_folder_path` is also present, RESUME it.
    meeting_id: Option<String>,
    resume_folder_path: Option<String>,
) -> Result<(), String> {
    log_info!("🚀 CALLED start_recording_with_devices_and_meeting - Mic: {:?}, System: {:?}, Meeting: {:?}, meeting_id: {:?}, resume: {:?}",
             mic_device_name, system_device_name, meeting_name, meeting_id, resume_folder_path);

    // Clone meeting_name for notification use later
    let meeting_name_for_notification = meeting_name.clone();

    // Call the recording module functions that support meeting names
    let recording_result = match (mic_device_name.clone(), system_device_name.clone()) {
        (None, None) => {
            log_info!(
                "No devices specified, starting with defaults and meeting: {:?}",
                meeting_name
            );
            super::recording_commands::start_recording_with_meeting_name(
                app.clone(),
                meeting_name,
                meeting_id,
                resume_folder_path,
            )
            .await
        }
        _ => {
            log_info!(
                "Starting with specified devices: mic={:?}, system={:?}, meeting={:?}",
                mic_device_name,
                system_device_name,
                meeting_name
            );
            super::recording_commands::start_recording_with_devices_and_meeting(
                app.clone(),
                mic_device_name,
                system_device_name,
                meeting_name,
                meeting_id,
                resume_folder_path,
            )
            .await
        }
    };

    match recording_result {
        Ok(_) => {
            log_info!("Recording started successfully via tauri command");

            // Same banner as the direct path above.
            notifications::os_commands::recording_banner(
                &app,
                "Recording started",
                meeting_name_for_notification.as_deref().unwrap_or("Meeting"),
                "nixon-recording",
            )
            .await;

            Ok(())
        }
        Err(e) => {
            log_error!("Failed to start recording via tauri command: {}", e);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn set_language_preference(language: String) -> Result<(), String> {
    let mut lang_pref = LANGUAGE_PREFERENCE
        .lock()
        .map_err(|e| format!("Failed to set language preference: {}", e))?;
    log_info!("Setting language preference to: {}", language);
    *lang_pref = language;
    Ok(())
}

// Internal helper function to get language preference (for use within Rust code)
pub fn get_language_preference_internal() -> Option<String> {
    LANGUAGE_PREFERENCE.lock().ok().map(|lang| lang.clone())
}
