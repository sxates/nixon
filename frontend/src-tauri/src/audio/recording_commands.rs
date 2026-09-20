// audio/recording_commands.rs
//
// Slim Tauri command layer for recording functionality.
// Delegates to transcription and recording modules for actual implementation.

use anyhow::Result;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::{
    device_resolution, parse_audio_device, DeviceEvent, DeviceMonitorType, RecordingManager,
};

// Import transcription modules
use super::transcription::{self, reset_speech_detected_flag};

// Re-export TranscriptUpdate for backward compatibility
pub use super::transcription::TranscriptUpdate;

// ============================================================================
// GLOBAL STATE
// ============================================================================

// Simple recording state tracking
static IS_RECORDING: AtomicBool = AtomicBool::new(false);
/// specs/0037 — set true at start when the session RESUMES an existing meeting
/// (a `resume_folder_path` was supplied), read at stop to tell the frontend to
/// APPEND (not save-new) and with which audio offset. Reset each start.
static RESUMED_SESSION: AtomicBool = AtomicBool::new(false);

// Global recording manager (kept alive during recording). The transcription-task handle
// lives in `live_toggle` (low-power-mode spec §3) so mid-meeting go-live can store it too.
static RECORDING_MANAGER: Mutex<Option<RecordingManager>> = Mutex::new(None);

// Listener ID for proper cleanup - prevents microphone from staying active after recording stops
static TRANSCRIPT_LISTENER_ID: Mutex<Option<tauri::EventId>> = Mutex::new(None);

// specs/0011 P3-B: live diarization session state. All `None`/empty (and zero cost)
// unless live diarization is enabled (the default-off sub-toggle) AND the models are
// present. The pipeline holds a clone of the same `Arc<LiveDiarizer>`; this handle lets
// the transcript-update listener feed it segments and lets `stop_recording` stop it.
static LIVE_DIARIZER: Mutex<Option<Arc<crate::diarization::live::LiveDiarizer>>> = Mutex::new(None);
// Accumulated transcript segment windows for the live diarizer to align against. Grows
// as `transcript-update` events arrive; pushed wholesale via `set_segments` each time.
static LIVE_SEGMENTS: Mutex<Vec<crate::diarization::live::LiveSegment>> = Mutex::new(Vec::new());

/// Resolve the live-diarization speaker count by the same precedence as the offline
/// pass (specs/0011 calendar-seed): manual override > calendar-remote estimate > Auto.
///
/// The live path has no persisted meeting row (saved on stop), so we locate the linked
/// calendar event by the in-progress meeting *name* + the current instant as the
/// recording start. Best-effort and non-fatal: a missing/denied calendar or no match
/// yields no attendees → Auto. The title+time lookup is routed by the single active
/// calendar source (specs/0032): Google connected → local-cache lookup only; otherwise
/// EventKit only (its FFI runs off the async executor).
async fn resolve_live_speaker_count<R: Runtime>(
    app: &AppHandle<R>,
    meeting_name: &str,
) -> (
    crate::diarization::SpeakerCount,
    crate::diarization::settings::SpeakerCountSource,
) {
    use crate::diarization::settings::resolve_speaker_count;

    let title = meeting_name.to_string();
    // The recording is starting "now"; the calendar reader searches a ±4h window
    // around this instant and matches on title. Routed by the single active
    // source (specs/0032): Google connected → cache-only; otherwise EventKit only.
    let started_at = chrono::Utc::now().to_rfc3339();
    let attendees = if crate::calendar::google_is_active_source(app).await {
        match app.try_state::<crate::state::AppState>() {
            Some(state) => {
                crate::calendar::google::sync::cached_attendees_by_title_time(
                    state.db_manager.pool(),
                    &title,
                    &started_at,
                )
                .await
            }
            None => Vec::new(),
        }
    } else {
        match tokio::task::spawn_blocking(move || {
            crate::calendar::eventkit::event_attendees(&title, &started_at)
        })
        .await
        {
            Ok(a) => a,
            Err(e) => {
                warn!("Live diarization: calendar attendee lookup failed ({e}); using Auto");
                Vec::new()
            }
        }
    };

    resolve_speaker_count(&attendees)
}

/// Build a `LiveDiarizer` for this recording, gated on
/// `diarization_enabled && live_diarization_enabled && models_present`. Returns `None`
/// (the default) when live diarization is off or the engine can't be constructed — the
/// pipeline then receives `None` and the whole live path is inert (no behaviour change
/// vs v0.4.0). The returned diarizer's `on_labels` callback emits `live-diarization-update`.
///
/// `meeting_id` here is the in-progress recording's *meeting name* (no DB row exists yet —
/// the frontend persists the meeting after stop), carried so the live panel can scope the
/// event; the live panel correlates individual rows by `segment_id` (the `sequence_id`).
async fn build_live_diarizer<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Option<Arc<crate::diarization::live::LiveDiarizer>> {
    use crate::diarization::{live::LiveDiarizer, models, settings, SherpaDiarizer};

    let s = settings::load_settings().await;
    if !(s.diarization_enabled && s.live_diarization_enabled) {
        return None;
    }
    if !models::models_present() {
        warn!(
            "Live diarization enabled but models are not present; \
             skipping live labels (offline pass at stop still applies)"
        );
        return None;
    }

    // Construct the same CPU/offline sherpa engine the offline pass uses (the live pass
    // re-runs it over the growing buffer — ADR-0006). Cheap to build; no download here
    // (models_present() was true).
    let paths = models::model_paths();
    // Resolve the speaker count by the same precedence as the offline pass
    // (specs/0011 calendar-seed): manual override > calendar-remote estimate > Auto.
    // The live path has no persisted meeting row yet (the frontend saves on stop), so
    // we locate the linked calendar event by the in-progress meeting *name* + "now" as
    // the start instant. Best-effort and non-fatal — any failure falls through to Auto.
    let (speaker_count, count_source) = resolve_live_speaker_count(&app, &meeting_id).await;
    match count_source {
        crate::diarization::settings::SpeakerCountSource::Calendar => {
            if let crate::diarization::SpeakerCount::Fixed(n) = speaker_count {
                info!("🗣️ Live diarization: seeding expected speakers = {n} from calendar attendees (excluding self)");
            }
        }
        crate::diarization::settings::SpeakerCountSource::Auto => {} // specs/0050: audio seed caps in the pass
    }
    let engine = match SherpaDiarizer::with_speaker_count(
        &paths.segmentation,
        &paths.embedding,
        speaker_count,
    )
    // specs/0039 WS1: consolidation is an offline-only pass. Live labels are
    // provisional and reconciled by the offline pass, so disable it here (Non-goal to
    // consolidate the live windowed pass).
    .map(|e| e.with_consolidation(false))
    {
        Ok(e) => Arc::new(e) as Arc<dyn crate::diarization::Diarizer>,
        Err(e) => {
            warn!("Could not init live diarizer engine ({e:#}); live labels disabled (recording unaffected)");
            return None;
        }
    };

    // The pass callback (runs on the blocking pass thread): emit the newly-resolved
    // labels as `live-diarization-update`. Each `NewSegmentLabel.segment_id` is the
    // transcript `sequence_id` (see the listener that feeds `set_segments`).
    let app_for_labels = app.clone();
    let meeting_for_labels = meeting_id.clone();
    let on_labels: crate::diarization::live::LabelCallback = Box::new(move |labels| {
        let segments: Vec<serde_json::Value> = labels
            .into_iter()
            .map(|l| {
                serde_json::json!({
                    "segment_id": l.segment_id,
                    "speaker": l.display_name,
                })
            })
            .collect();
        if segments.is_empty() {
            return;
        }
        let _ = app_for_labels.emit(
            "live-diarization-update",
            serde_json::json!({
                "meeting_id": meeting_for_labels,
                "segments": segments,
            }),
        );
    });

    match LiveDiarizer::start(engine, on_labels) {
        Ok(d) => {
            info!("🗣️ Live diarization enabled for this recording");
            Some(Arc::new(d))
        }
        Err(e) => {
            warn!("Could not start live diarizer ({e:#}); live labels disabled (recording unaffected)");
            None
        }
    }
}

/// Accumulate one transcript segment window and push the full set into the live
/// diarizer (so its next pass can align turns to current segments). No-op (and only a
/// cheap `Option` check) when live diarization is disabled — the common path.
fn feed_live_diarizer_segment(update: &TranscriptUpdate) {
    // Cheap gate: nothing to do unless a live diarizer is active for this recording.
    let diarizer = {
        let guard = LIVE_DIARIZER.lock().unwrap();
        match guard.as_ref() {
            Some(d) => d.clone(),
            None => return,
        }
    };

    // The live `segment_id` is the transcript `sequence_id` as a string — the same key
    // the frontend live panel uses to correlate `live-diarization-update` rows.
    let seg = crate::diarization::live::LiveSegment {
        id: update.sequence_id.to_string(),
        start: update.audio_start_time as f32,
        end: update.audio_end_time as f32,
    };

    let segments = {
        let mut segs = LIVE_SEGMENTS.lock().unwrap();
        // De-dup on id in case of a re-emitted sequence (defensive; ordering preserved).
        if !segs.iter().any(|s| s.id == seg.id) {
            segs.push(seg);
        }
        segs.clone()
    };
    diarizer.set_segments(segments);
}

/// Reset the live diarization session state and stop any running diarizer. Idempotent;
/// safe to call when live diarization was never enabled.
fn teardown_live_diarizer() {
    if let Some(diarizer) = LIVE_DIARIZER.lock().unwrap().take() {
        diarizer.stop();
    }
    LIVE_SEGMENTS.lock().unwrap().clear();
}

// ============================================================================
// PUBLIC TYPES
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RecordingArgs {
    pub save_path: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct TranscriptionStatus {
    pub chunks_in_queue: usize,
    pub is_processing: bool,
    pub last_activity_ms: u64,
}

// ============================================================================
// RECORDING COMMANDS
// ============================================================================

/// Start recording with default devices
pub async fn start_recording<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    start_recording_with_meeting_name(app, None, None, None).await
}

/// specs/0037 — link a fresh manager to its DB row and, when a `resume_folder_path`
/// is supplied, put it in resume mode (reuse the folder, record a new segment).
/// Also records whether this session is a resume in `RESUMED_SESSION`, read at stop
/// to decide append-vs-save. Shared by both start paths.
fn apply_resume_context(
    manager: &mut RecordingManager,
    meeting_id: Option<String>,
    resume_folder_path: Option<String>,
) {
    manager.set_meeting_id(meeting_id.clone());
    match (meeting_id, resume_folder_path) {
        (Some(mid), Some(folder)) => {
            info!(
                "↩️ Resuming recording into meeting {} (folder {})",
                mid, folder
            );
            manager.set_resume_context(mid, std::path::PathBuf::from(folder));
            RESUMED_SESSION.store(true, Ordering::SeqCst);
        }
        _ => {
            RESUMED_SESSION.store(false, Ordering::SeqCst);
        }
    }
}

/// Start recording with default devices and optional meeting name.
///
/// specs/0037: `meeting_id` links the recording to its DB row (written into
/// `metadata.json` at start so crash recovery can reattach). When
/// `resume_folder_path` is also `Some`, the session RESUMES that meeting — the
/// saver reuses the folder and records a new numbered segment.
pub async fn start_recording_with_meeting_name<R: Runtime>(
    app: AppHandle<R>,
    meeting_name: Option<String>,
    meeting_id: Option<String>,
    resume_folder_path: Option<String>,
) -> Result<(), String> {
    info!(
        "Starting recording with default devices, meeting: {:?}",
        meeting_name
    );

    let engine_lifecycle_guard = super::common::acquire_engine_lifecycle_lock().await;

    // Check if already recording
    let current_recording_state = IS_RECORDING.load(Ordering::SeqCst);
    info!("🔍 IS_RECORDING state check: {}", current_recording_state);
    if current_recording_state {
        return Err("Recording already in progress".to_string());
    }
    // specs/0037: clear any stale resume flag up-front so a start that failed *after*
    // arming it (or an early-exit before apply_resume_context) can never mis-tag this
    // session's stop as a resume. apply_resume_context re-arms it only for a real resume.
    RESUMED_SESSION.store(false, Ordering::SeqCst);

    // Load recording preferences to get auto_save, live-transcription AND device prefs
    let (
        auto_save,
        live_transcription_enabled,
        low_power_on_battery,
        preferred_mic_name,
        preferred_system_name,
    ) = match super::recording_preferences::load_recording_preferences(&app).await {
        Ok(prefs) => {
            info!("📋 Loaded recording preferences: auto_save={}, live_transcription={}, low_power_on_battery={}, preferred_mic={:?}, preferred_system={:?}",
                      prefs.auto_save, prefs.live_transcription_enabled, prefs.low_power_on_battery, prefs.preferred_mic_device, prefs.preferred_system_device);
            (
                prefs.auto_save,
                prefs.live_transcription_enabled,
                prefs.low_power_on_battery,
                prefs.preferred_mic_device,
                prefs.preferred_system_device,
            )
        }
        Err(e) => {
            warn!(
                "Failed to load recording preferences, using defaults: {}",
                e
            );
            (true, true, true, None, None)
        }
    };

    // low-power-mode spec §§2-4: pick the effective live/defer mode from the global
    // preference, on-battery state, and this meeting's per-meeting override (if any).
    // Emits `processing-mode-changed` for the frontend.
    let live_transcription_enabled = super::live_toggle::decide_session_mode(
        &app,
        live_transcription_enabled,
        low_power_on_battery,
        meeting_id.as_deref(),
    )
    .await;

    // Validate that transcription models are available before starting recording.
    // specs/0029 WS7.2: skipped in record-only mode — no live STT runs, so a
    // missing/downloading model must not block a record-only recording.
    if live_transcription_enabled {
        info!("🔍 Validating transcription model availability before starting recording...");
        if let Err(validation_error) = transcription::validate_transcription_model_ready(&app).await
        {
            error!("Model validation failed: {}", validation_error);

            // Emit error event for frontend - actionable: false to show toast instead of modal
            // (download progress is already shown in top-right toast)
            let _ = app.emit("transcription-error", serde_json::json!({
                "error": validation_error,
                "userMessage": "Recording cannot start: Transcription model is still downloading. Please wait for the download to complete.",
                "actionable": false
            }));

            return Err(validation_error);
        }
        info!("✅ Transcription model validation passed");
    } else {
        info!("🎙️ Record-only mode: skipping transcription model validation (no live STT)");
    }

    // Async-first approach - no more blocking operations!
    info!("🚀 Starting async recording initialization");

    // Create new recording manager
    let mut manager = RecordingManager::new();

    // ============================================================================
    // DEVICE RESOLUTION: Preference → Default → Error/None (audio/device_resolution.rs)
    // ============================================================================
    let microphone_device = Some(device_resolution::resolve_microphone(preferred_mic_name)?);
    let system_device = device_resolution::resolve_system_audio(preferred_system_name);

    // Always ensure a meeting name is set so incremental saver initializes
    let effective_meeting_name = meeting_name.clone().unwrap_or_else(|| {
        // Example: Meeting 2025-10-03_08-25-23
        let now = chrono::Local::now();
        format!("Meeting {}", now.format("%Y-%m-%d_%H-%M-%S"))
    });
    manager.set_meeting_name(Some(effective_meeting_name.clone()));
    let resume_pair = meeting_id.clone().zip(resume_folder_path.clone());
    apply_resume_context(&mut manager, meeting_id, resume_folder_path);
    // specs/0037 (review-2): BEFORE the resumed session can write anything to the
    // folder, import the crashed session's transcripts.json into the meeting's DB rows
    // (guarded attach → a cleanly-stopped "Continue recording" is a natural no-op).
    if let Some((mid, folder)) = &resume_pair {
        super::recording_recovery::import_prior_folder_transcripts(
            &app,
            mid,
            std::path::Path::new(folder),
        )
        .await;
    }

    // Set up error callback
    let app_for_error = app.clone();
    manager.set_error_callback(move |error| {
        let _ = app_for_error.emit("recording-error", error.user_message());
    });

    // specs/0011 P3-B: construct the live diarizer (gated on settings + models). `None`
    // when live diarization is off (the default) → the live path stays fully inert.
    // specs/0029 WS7.2: also `None` in record-only mode — live diarization aligns
    // against live transcript segments, which don't exist without live STT.
    let live_diarizer = if live_transcription_enabled {
        build_live_diarizer(app.clone(), effective_meeting_name).await
    } else {
        None
    };
    *LIVE_DIARIZER.lock().unwrap() = live_diarizer.clone();
    LIVE_SEGMENTS.lock().unwrap().clear();

    // low-power-mode spec §3: shared live-STT flag the mid-meeting toggle can flip.
    let live_stt = super::live_toggle::register_session_flag(live_transcription_enabled);

    // Start recording with resolved devices (replaces start_recording_with_defaults_and_auto_save call)
    let transcription_receiver = manager
        .start_recording(
            microphone_device,
            system_device,
            auto_save,
            live_diarizer,
            live_stt,
        )
        .await
        .map_err(|e| format!("Failed to start recording: {}", e))?;

    // Store the manager globally to keep it alive
    {
        let mut global_manager = RECORDING_MANAGER.lock().unwrap();
        *global_manager = Some(manager);
    }

    // Set recording flag and reset speech detection flag
    info!("🔍 Setting IS_RECORDING to true and resetting SPEECH_DETECTED_EMITTED");
    IS_RECORDING.store(true, Ordering::SeqCst);
    drop(engine_lifecycle_guard);
    reset_speech_detected_flag(); // Reset for new recording session

    // Start optimized parallel transcription task and store handle.
    // specs/0029 WS7.2: in record-only mode the worker is NOT spawned (no STT engine
    // init, no model load) — the pipeline produces no transcription chunks anyway.
    // The live level/spectrum emitter is registered at app setup (lib.rs), so the
    // recording visualizers keep working without this task.
    if live_transcription_enabled {
        let task_handle =
            transcription::start_transcription_task(app.clone(), transcription_receiver);
        super::live_toggle::store_transcription_task(task_handle);
    } else {
        info!("🎙️ Deferred mode: transcription worker not spawned (stashing receiver for mid-meeting go-live)");
        super::live_toggle::stash_receiver(transcription_receiver);
    }

    // CRITICAL: Listen for transcript-update events and save to recording manager
    // This enables transcript history persistence for page reload sync
    // Store listener ID for cleanup during stop_recording to ensure microphone is released
    {
        use tauri::Listener;
        let listener_id = app.listen("transcript-update", move |event: tauri::Event| {
            // Parse the transcript update from the event payload
            if let Ok(update) = serde_json::from_str::<TranscriptUpdate>(event.payload()) {
                // Create structured transcript segment
                let segment = crate::audio::recording_saver::TranscriptSegment {
                    id: format!("seg_{}", update.sequence_id),
                    text: update.text.clone(),
                    audio_start_time: update.audio_start_time,
                    audio_end_time: update.audio_end_time,
                    duration: update.duration,
                    display_time: update.timestamp.clone(), // Use wall-clock timestamp for display
                    confidence: update.confidence,
                    sequence_id: update.sequence_id,
                    // specs/0029 WS3.4 follow-up: keep the capture-channel tag in the
                    // rehydration copy so a page reload mid-recording doesn't lose it.
                    channel: update.channel.clone(),
                };

                // Save to recording manager
                if let Ok(manager_guard) = RECORDING_MANAGER.lock() {
                    if let Some(manager) = manager_guard.as_ref() {
                        manager.add_transcript_segment(segment);
                    }
                }

                // specs/0011 P3-B: feed the live diarizer the accumulated segment windows
                // so its next pass can align turns to them. `segment_id` = `sequence_id`
                // (string), the key the live panel correlates `live-diarization-update` on.
                // Inert (no diarizer present) unless live diarization is enabled.
                feed_live_diarizer_segment(&update);
            }
        });
        let mut global_listener = TRANSCRIPT_LISTENER_ID.lock().unwrap();
        *global_listener = Some(listener_id);
        info!("✅ Transcript-update event listener registered for history persistence");
    }

    // Emit success event
    app.emit(
        "recording-started",
        serde_json::json!({
            "message": "Recording started successfully with parallel processing",
            "devices": ["Default Microphone", "Default System Audio"],
            "workers": 3
        }),
    )
    .map_err(|e| e.to_string())?;

    // Update tray menu to reflect recording state
    crate::tray::update_tray_menu(&app);

    info!("✅ Recording started successfully with async-first approach");

    Ok(())
}

/// Start recording with specific devices
pub async fn start_recording_with_devices<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
    system_device_name: Option<String>,
) -> Result<(), String> {
    start_recording_with_devices_and_meeting(
        app,
        mic_device_name,
        system_device_name,
        None,
        None,
        None,
    )
    .await
}

/// Start recording with specific devices and optional meeting name.
/// specs/0037: see [`start_recording_with_meeting_name`] for `meeting_id` /
/// `resume_folder_path`.
pub async fn start_recording_with_devices_and_meeting<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
    system_device_name: Option<String>,
    meeting_name: Option<String>,
    meeting_id: Option<String>,
    resume_folder_path: Option<String>,
) -> Result<(), String> {
    info!(
        "Starting recording with specific devices: mic={:?}, system={:?}, meeting={:?}",
        mic_device_name, system_device_name, meeting_name
    );

    let engine_lifecycle_guard = super::common::acquire_engine_lifecycle_lock().await;

    // Check if already recording
    let current_recording_state = IS_RECORDING.load(Ordering::SeqCst);
    info!("🔍 IS_RECORDING state check: {}", current_recording_state);
    if current_recording_state {
        return Err("Recording already in progress".to_string());
    }
    // specs/0037: clear any stale resume flag up-front so a start that failed *after*
    // arming it (or an early-exit before apply_resume_context) can never mis-tag this
    // session's stop as a resume. apply_resume_context re-arms it only for a real resume.
    RESUMED_SESSION.store(false, Ordering::SeqCst);

    // Load recording preferences to check auto_save + live-transcription settings
    let (auto_save, live_transcription_enabled, low_power_on_battery) =
        match super::recording_preferences::load_recording_preferences(&app).await {
            Ok(prefs) => {
                info!(
                    "📋 Loaded recording preferences: auto_save={}, live_transcription={}, low_power_on_battery={}",
                    prefs.auto_save, prefs.live_transcription_enabled, prefs.low_power_on_battery
                );
                (
                    prefs.auto_save,
                    prefs.live_transcription_enabled,
                    prefs.low_power_on_battery,
                )
            }
            Err(e) => {
                warn!(
                    "Failed to load recording preferences, defaulting to auto_save=true: {}",
                    e
                );
                (true, true, true) // Default to saving + live transcription if preferences can't be loaded
            }
        };

    // low-power-mode spec §§2-4: pick the effective live/defer mode from the global
    // preference, on-battery state, and this meeting's per-meeting override (if any).
    // Emits `processing-mode-changed` for the frontend.
    let live_transcription_enabled = super::live_toggle::decide_session_mode(
        &app,
        live_transcription_enabled,
        low_power_on_battery,
        meeting_id.as_deref(),
    )
    .await;

    // Validate that transcription models are available before starting recording.
    // specs/0029 WS7.2: skipped in record-only mode — no live STT runs, so a
    // missing/downloading model must not block a record-only recording.
    if live_transcription_enabled {
        info!("🔍 Validating transcription model availability before starting recording...");
        if let Err(validation_error) = transcription::validate_transcription_model_ready(&app).await
        {
            error!("Model validation failed: {}", validation_error);

            // Emit error event for frontend - actionable: false to show toast instead of modal
            // (download progress is already shown in top-right toast)
            let _ = app.emit("transcription-error", serde_json::json!({
                "error": validation_error,
                "userMessage": "Recording cannot start: Transcription model is still downloading. Please wait for the download to complete.",
                "actionable": false
            }));

            return Err(validation_error);
        }
        info!("✅ Transcription model validation passed");
    } else {
        info!("🎙️ Record-only mode: skipping transcription model validation (no live STT)");
    }

    // Parse devices
    let mic_device = if let Some(ref name) = mic_device_name {
        Some(Arc::new(parse_audio_device(name).map_err(|e| {
            format!("Invalid microphone device '{}': {}", name, e)
        })?))
    } else {
        None
    };

    let system_device = if let Some(ref name) = system_device_name {
        Some(Arc::new(parse_audio_device(name).map_err(|e| {
            format!("Invalid system device '{}': {}", name, e)
        })?))
    } else {
        None
    };

    // Async-first approach for custom devices - no more blocking operations!
    info!("🚀 Starting async recording initialization with custom devices");

    // Create new recording manager
    let mut manager = RecordingManager::new();

    // Always ensure a meeting name is set so incremental saver initializes
    let effective_meeting_name = meeting_name.clone().unwrap_or_else(|| {
        let now = chrono::Local::now();
        format!("Meeting {}", now.format("%Y-%m-%d_%H-%M-%S"))
    });
    manager.set_meeting_name(Some(effective_meeting_name.clone()));
    let resume_pair = meeting_id.clone().zip(resume_folder_path.clone());
    apply_resume_context(&mut manager, meeting_id, resume_folder_path);
    // specs/0037 (review-2): BEFORE the resumed session can write anything to the
    // folder, import the crashed session's transcripts.json into the meeting's DB rows
    // (guarded attach → a cleanly-stopped "Continue recording" is a natural no-op).
    if let Some((mid, folder)) = &resume_pair {
        super::recording_recovery::import_prior_folder_transcripts(
            &app,
            mid,
            std::path::Path::new(folder),
        )
        .await;
    }

    // Set up error callback
    let app_for_error = app.clone();
    manager.set_error_callback(move |error| {
        let _ = app_for_error.emit("recording-error", error.user_message());
    });

    // specs/0011 P3-B: construct the live diarizer (gated on settings + models). `None`
    // when live diarization is off (the default) → the live path stays fully inert.
    // specs/0029 WS7.2: also `None` in record-only mode (no live transcript to align).
    let live_diarizer = if live_transcription_enabled {
        build_live_diarizer(app.clone(), effective_meeting_name).await
    } else {
        None
    };
    *LIVE_DIARIZER.lock().unwrap() = live_diarizer.clone();
    LIVE_SEGMENTS.lock().unwrap().clear();

    // low-power-mode spec §3: shared live-STT flag the mid-meeting toggle can flip.
    let live_stt = super::live_toggle::register_session_flag(live_transcription_enabled);

    // Start recording with specified devices and auto_save setting
    let transcription_receiver = manager
        .start_recording(
            mic_device,
            system_device,
            auto_save,
            live_diarizer,
            live_stt,
        )
        .await
        .map_err(|e| format!("Failed to start recording: {}", e))?;

    // Store the manager globally to keep it alive
    {
        let mut global_manager = RECORDING_MANAGER.lock().unwrap();
        *global_manager = Some(manager);
    }

    // Set recording flag and reset speech detection flag
    info!("🔍 Setting IS_RECORDING to true and resetting SPEECH_DETECTED_EMITTED");
    IS_RECORDING.store(true, Ordering::SeqCst);
    drop(engine_lifecycle_guard);
    reset_speech_detected_flag(); // Reset for new recording session

    // Start optimized parallel transcription task and store handle.
    // specs/0029 WS7.2: not spawned in record-only mode (see the default-devices path).
    if live_transcription_enabled {
        let task_handle =
            transcription::start_transcription_task(app.clone(), transcription_receiver);
        super::live_toggle::store_transcription_task(task_handle);
    } else {
        info!("🎙️ Deferred mode: transcription worker not spawned (stashing receiver for mid-meeting go-live)");
        super::live_toggle::stash_receiver(transcription_receiver);
    }

    // CRITICAL: Listen for transcript-update events and save to recording manager
    // This enables transcript history persistence for page reload sync
    // Store listener ID for cleanup during stop_recording to ensure microphone is released
    {
        use tauri::Listener;
        let listener_id = app.listen("transcript-update", move |event: tauri::Event| {
            // Parse the transcript update from the event payload
            if let Ok(update) = serde_json::from_str::<TranscriptUpdate>(event.payload()) {
                // Create structured transcript segment
                let segment = crate::audio::recording_saver::TranscriptSegment {
                    id: format!("seg_{}", update.sequence_id),
                    text: update.text.clone(),
                    audio_start_time: update.audio_start_time,
                    audio_end_time: update.audio_end_time,
                    duration: update.duration,
                    display_time: update.timestamp.clone(), // Use wall-clock timestamp for display
                    confidence: update.confidence,
                    sequence_id: update.sequence_id,
                    // specs/0029 WS3.4 follow-up: keep the capture-channel tag in the
                    // rehydration copy so a page reload mid-recording doesn't lose it.
                    channel: update.channel.clone(),
                };

                // Save to recording manager
                if let Ok(manager_guard) = RECORDING_MANAGER.lock() {
                    if let Some(manager) = manager_guard.as_ref() {
                        manager.add_transcript_segment(segment);
                    }
                }

                // specs/0011 P3-B: feed the live diarizer the accumulated segment windows
                // (inert unless live diarization is enabled). See first start path.
                feed_live_diarizer_segment(&update);
            }
        });
        let mut global_listener = TRANSCRIPT_LISTENER_ID.lock().unwrap();
        *global_listener = Some(listener_id);
        info!("✅ Transcript-update event listener registered for history persistence");
    }

    // Emit success event
    app.emit(
        "recording-started",
        serde_json::json!({
            "message": "Recording started with custom devices and parallel processing",
            "devices": [
                mic_device_name.unwrap_or_else(|| "Default Microphone".to_string()),
                system_device_name.unwrap_or_else(|| "Default System Audio".to_string())
            ],
            "workers": 3
        }),
    )
    .map_err(|e| e.to_string())?;

    // Update tray menu to reflect recording state
    crate::tray::update_tray_menu(&app);

    info!("✅ Recording started with custom devices using async-first approach");

    Ok(())
}

/// Stop recording with optimized graceful shutdown ensuring NO transcript chunks are lost
pub async fn stop_recording<R: Runtime>(
    app: AppHandle<R>,
    _args: RecordingArgs,
) -> Result<serde_json::Value, String> {
    info!(
        "🛑 Starting optimized recording shutdown - ensuring ALL transcript chunks are preserved"
    );

    // Check if recording is active
    if !IS_RECORDING.load(Ordering::SeqCst) {
        info!("Recording was not active");
        return Ok(serde_json::json!({ "message": "Recording was not active" }));
    }

    // Emit shutdown progress to frontend
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "stopping_audio",
            "message": "Stopping audio capture...",
            "progress": 20
        }),
    );

    // Step 1: Stop audio capture immediately (no more new chunks) with proper error handling
    let manager_for_cleanup = {
        let mut global_manager = RECORDING_MANAGER.lock().unwrap();
        global_manager.take()
    };

    let stop_result = if let Some(mut manager) = manager_for_cleanup {
        // Use FORCE FLUSH to immediately process all accumulated audio - eliminates 30s delay!
        info!("🚀 Using FORCE FLUSH to eliminate pipeline accumulation delays");
        let result = manager.stop_streams_and_force_flush().await;
        // Store manager back for later cleanup
        let manager_for_cleanup = Some(manager);
        (result, manager_for_cleanup)
    } else {
        warn!("No recording manager found to stop");
        (Ok(()), None)
    };

    let (stop_result, manager_for_cleanup) = stop_result;

    match stop_result {
        Ok(_) => {
            info!("✅ Audio streams stopped successfully - no more chunks will be created");
        }
        Err(e) => {
            error!("❌ Failed to stop audio streams: {}", e);
            return Err(format!("Failed to stop audio streams: {}", e));
        }
    }

    // Step 1.5: Clean up transcript listener to release microphone
    // Unlisten transcript-update event to prevent lingering references
    {
        use tauri::Listener;
        if let Some(listener_id) = TRANSCRIPT_LISTENER_ID.lock().unwrap().take() {
            app.unlisten(listener_id);
            info!("✅ Transcript-update listener removed");
        }
    }

    // Step 1.6: Stop live diarization (specs/0011 P3-B). The authoritative offline pass
    // runs after this (frontend-driven on `recording-stopped` → `api_diarize_meeting`,
    // P1-C), persists `transcripts.speaker`, and emits `diarization-complete`, which the
    // frontend re-fetches — so live never fights the offline source of truth. Idempotent
    // and a no-op when live diarization was never enabled.
    teardown_live_diarizer();

    // low-power-mode spec §§3-4: drop this session's live-STT flag and any stashed
    // (deferred-mode) transcription receiver — both are scoped to one recording.
    super::live_toggle::clear_session();

    // Step 2: Signal transcription workers to finish processing ALL queued chunks
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "processing_transcripts",
            "message": "Processing remaining transcript chunks...",
            "progress": 40
        }),
    );

    // Wait for transcription task with enhanced progress monitoring (NO TIMEOUT - we must process all chunks)
    let transcription_task = super::live_toggle::take_transcription_task();

    if let Some(task_handle) = transcription_task {
        info!("⏳ Waiting for ALL transcription chunks to be processed (no timeout - preserving every chunk)");

        // Enhanced progress monitoring during shutdown
        let progress_app = app.clone();
        let progress_task = tokio::spawn(async move {
            let last_update = std::time::Instant::now();

            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                // Emit periodic progress updates during shutdown
                let elapsed = last_update.elapsed().as_secs();
                let _ = progress_app.emit(
                    "recording-shutdown-progress",
                    serde_json::json!({
                        "stage": "processing_transcripts",
                        "message": format!("Processing transcripts... ({}s elapsed)", elapsed),
                        "progress": 40,
                        "detailed": true,
                        "elapsed_seconds": elapsed
                    }),
                );
            }
        });

        // Wait up to 10 minutes for transcription completion to prevent indefinite hangs
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(600), // 10 minutes max
            task_handle,
        )
        .await
        {
            Ok(Ok(())) => {
                info!("✅ ALL transcription chunks processed successfully - no data lost");
            }
            Ok(Err(e)) => {
                warn!("⚠️ Transcription task completed with error: {:?}", e);
                // Continue anyway - the worker may have processed most chunks
            }
            Err(_) => {
                warn!("⏱️ Transcription timeout (10 minutes) reached, continuing shutdown to prevent indefinite hang");
                // Continue shutdown even on timeout - better to lose some chunks than hang forever
            }
        }

        // Stop progress monitoring
        progress_task.abort();
    } else {
        info!("ℹ️ No transcription task found to wait for");
    }

    // Step 3: Now safely unload Whisper model after ALL chunks are processed
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "unloading_model",
            "message": "Unloading speech recognition model...",
            "progress": 70
        }),
    );

    info!("🧠 All transcript chunks processed. Now safely unloading transcription model...");

    // Determine which provider was used and unload the appropriate model (with timeout)
    let config = match tokio::time::timeout(
        tokio::time::Duration::from_secs(30), // 30 seconds max for DB operation
        crate::settings::api_get_transcript_config(app.clone(), app.clone().state()),
    )
    .await
    {
        Ok(Ok(Some(config))) => Some(config.provider),
        Ok(Ok(None)) => None,
        Ok(Err(e)) => {
            warn!("⚠️ Failed to get transcript config: {:?}", e);
            None
        }
        Err(_) => {
            warn!("⏱️ Transcript config timeout (30s), continuing shutdown");
            None
        }
    };

    match config.as_deref() {
        Some("parakeet") => {
            info!("🦜 Unloading Parakeet model...");
            let engine_clone = {
                let engine_guard = crate::parakeet_engine::commands::PARAKEET_ENGINE
                    .lock()
                    .unwrap();
                engine_guard.as_ref().cloned()
            };

            if let Some(engine) = engine_clone {
                let current_model = engine
                    .get_current_model()
                    .await
                    .unwrap_or_else(|| "unknown".to_string());
                info!("Current Parakeet model before unload: '{}'", current_model);

                if engine.unload_model().await {
                    info!(
                        "✅ Parakeet model '{}' unloaded successfully",
                        current_model
                    );
                } else {
                    warn!("⚠️ Failed to unload Parakeet model '{}'", current_model);
                }
            } else {
                warn!("⚠️ No Parakeet engine found to unload model");
            }
        }
        _ => {
            // Default to Whisper
            info!("🎤 Unloading Whisper model...");
            let engine_clone = {
                let engine_guard = crate::whisper_engine::commands::WHISPER_ENGINE
                    .lock()
                    .unwrap();
                engine_guard.as_ref().cloned()
            };

            if let Some(engine) = engine_clone {
                let current_model = engine
                    .get_current_model()
                    .await
                    .unwrap_or_else(|| "unknown".to_string());
                info!("Current Whisper model before unload: '{}'", current_model);

                if engine.unload_model().await {
                    info!("✅ Whisper model '{}' unloaded successfully", current_model);
                } else {
                    warn!("⚠️ Failed to unload Whisper model '{}'", current_model);
                }
            } else {
                warn!("⚠️ No Whisper engine found to unload model");
            }
        }
    }

    // Step 4: Finalize recording state and cleanup resources safely
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "finalizing",
            "message": "Finalizing recording and cleaning up resources...",
            "progress": 90
        }),
    );

    // Perform final cleanup with the manager if available
    let (meeting_folder, meeting_name, prior_audio_duration, recording_meeting_id) =
        if let Some(mut manager) = manager_for_cleanup {
            info!("🧹 Performing final cleanup and saving recording data");

            // Extract meeting info BEFORE async operations. specs/0037: the prior audio
            // duration (0.0 unless this was a resume) is the exact offset the frontend
            // applies when APPENDING this session's transcripts to the meeting.
            let meeting_folder = manager.get_meeting_folder();
            let meeting_name = manager.get_meeting_name();
            let prior_audio_duration = manager.prior_audio_duration_seconds();
            // v1.6.1: the recording's AUTHORITATIVE meeting id (set at start, in metadata).
            // The frontend saves transcripts to THIS, not its mutable current-meeting
            // selection — which drifts if the user opens another meeting while recording,
            // sending the transcript to the wrong meeting.
            let recording_meeting_id = manager.get_meeting_id();

            match tokio::time::timeout(
                tokio::time::Duration::from_secs(300), // 5 minutes max for file I/O
                manager.save_recording_only(&app),
            )
            .await
            {
                Ok(Ok(_)) => {
                    info!("✅ Recording data saved successfully during cleanup");
                }
                Ok(Err(e)) => {
                    warn!(
                        "⚠️ Error during recording cleanup (transcripts preserved): {}",
                        e
                    );
                    // Don't fail shutdown - transcripts are already preserved
                }
                Err(_) => {
                    warn!(
                        "⏱️ File I/O timeout (5 minutes) reached during save, continuing shutdown"
                    );
                    // Don't fail shutdown - transcripts are already preserved
                }
            }

            (
                meeting_folder,
                meeting_name,
                prior_audio_duration,
                recording_meeting_id,
            )
        } else {
            info!("ℹ️ No recording manager available for cleanup");
            (None, None, 0.0, None)
        };

    // Set recording flag to false
    info!("🔍 Setting IS_RECORDING to false");
    IS_RECORDING.store(false, Ordering::SeqCst);
    // specs/0037: was this a resumed session? Read-and-reset so the next start is clean.
    let resumed = RESUMED_SESSION.swap(false, Ordering::SeqCst);

    // Step 4.5: Prepare metadata for frontend (NO database save)
    // NOTE: We do NOT save to database here. The frontend will save after all transcripts are displayed.
    // This ensures the user sees all transcripts streaming in before the database save happens.
    let (folder_path_str, meeting_name_str) = match (&meeting_folder, &meeting_name) {
        (Some(path), Some(name)) => (Some(path.to_string_lossy().to_string()), Some(name.clone())),
        _ => (None, None),
    };

    info!("📤 Preparing recording metadata for frontend save");
    info!("   folder_path: {:?}", folder_path_str);
    info!("   meeting_name: {:?}", meeting_name_str);

    // Database save removed - frontend will handle this after receiving all transcripts
    info!("ℹ️ Skipping database save in Rust - frontend will save after all transcripts received");

    // Step 5: Complete shutdown
    let _ = app.emit(
        "recording-shutdown-progress",
        serde_json::json!({
            "stage": "complete",
            "message": "Recording stopped successfully",
            "progress": 100
        }),
    );

    // The stop payload — emitted as `recording-stopped` AND returned from the command.
    // specs/0037 (review-2): a resumed session's stop APPENDS to the existing meeting with
    // this audio offset (the prior segments' total duration); a fresh session sends
    // resumed=false / 0.0. Returning it makes the invoke's resolved value the frontend's
    // race-free primary source — the event alone arrives only after the (possibly slow,
    // multi-segment-concat) save, and could lose a 5s wait race on long meetings.
    let stop_info = serde_json::json!({
        "message": "Recording stopped - frontend will save after all transcripts received",
        "folder_path": folder_path_str,
        "meeting_name": meeting_name_str,
        "resumed": resumed,
        "prior_audio_duration_seconds": prior_audio_duration,
        // v1.6.1: authoritative save target — see recording_meeting_id above.
        "meeting_id": recording_meeting_id
    });

    // Emit final stop event with folder_path and meeting_name for frontend to save
    app.emit("recording-stopped", stop_info.clone())
        .map_err(|e| e.to_string())?;

    // Update tray menu to reflect stopped state
    crate::tray::update_tray_menu(&app);

    info!("🎉 Recording stopped successfully with ZERO transcript chunks lost");
    Ok(stop_info)
}

/// Check if recording is active
pub async fn is_recording() -> bool {
    IS_RECORDING.load(Ordering::SeqCst)
}

/// Get recording statistics
pub async fn get_transcription_status() -> TranscriptionStatus {
    TranscriptionStatus {
        chunks_in_queue: 0,
        is_processing: IS_RECORDING.load(Ordering::SeqCst),
        last_activity_ms: 0,
    }
}

/// Pause the current recording
#[tauri::command]
pub async fn pause_recording<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    info!("Pausing recording");

    // Check if currently recording
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return Err("No recording is currently active".to_string());
    }

    // Access the recording manager and pause it
    let manager_guard = RECORDING_MANAGER.lock().unwrap();
    if let Some(manager) = manager_guard.as_ref() {
        manager.pause_recording().map_err(|e| e.to_string())?;

        // Emit pause event to frontend
        app.emit(
            "recording-paused",
            serde_json::json!({
                "message": "Recording paused"
            }),
        )
        .map_err(|e| e.to_string())?;

        // Update tray menu to reflect paused state
        crate::tray::update_tray_menu(&app);

        info!("Recording paused successfully");
        Ok(())
    } else {
        Err("No recording manager found".to_string())
    }
}

/// Resume the current recording
#[tauri::command]
pub async fn resume_recording<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    info!("Resuming recording");

    // Check if currently recording
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return Err("No recording is currently active".to_string());
    }

    // Access the recording manager and resume it
    let manager_guard = RECORDING_MANAGER.lock().unwrap();
    if let Some(manager) = manager_guard.as_ref() {
        manager.resume_recording().map_err(|e| e.to_string())?;

        // Emit resume event to frontend
        app.emit(
            "recording-resumed",
            serde_json::json!({
                "message": "Recording resumed"
            }),
        )
        .map_err(|e| e.to_string())?;

        // Update tray menu to reflect resumed state
        crate::tray::update_tray_menu(&app);

        info!("Recording resumed successfully");
        Ok(())
    } else {
        Err("No recording manager found".to_string())
    }
}

/// Check if recording is currently paused
#[tauri::command]
pub async fn is_recording_paused() -> bool {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();
    if let Some(manager) = manager_guard.as_ref() {
        manager.is_paused()
    } else {
        false
    }
}

/// Get detailed recording state
#[tauri::command]
pub async fn get_recording_state() -> serde_json::Value {
    let is_recording = IS_RECORDING.load(Ordering::SeqCst);
    let manager_guard = RECORDING_MANAGER.lock().unwrap();

    if let Some(manager) = manager_guard.as_ref() {
        serde_json::json!({
            "is_recording": is_recording,
            "is_paused": manager.is_paused(),
            "is_active": manager.is_active(),
            "recording_duration": manager.get_recording_duration(),
            "active_duration": manager.get_active_recording_duration(),
            "total_pause_duration": manager.get_total_pause_duration(),
            "current_pause_duration": manager.get_current_pause_duration()
        })
    } else {
        serde_json::json!({
            "is_recording": is_recording,
            "is_paused": false,
            "is_active": false,
            "recording_duration": null,
            "active_duration": null,
            "total_pause_duration": 0.0,
            "current_pause_duration": null
        })
    }
}

/// Get the meeting folder path for the current recording
/// Returns the path if a meeting name was set and folder structure initialized
#[tauri::command]
pub async fn get_meeting_folder_path() -> Result<Option<String>, String> {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();
    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager
            .get_meeting_folder()
            .map(|p| p.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

/// Get accumulated transcript segments from current recording session
/// Used for syncing frontend state after page reload during active recording
#[tauri::command]
pub async fn get_transcript_history(
) -> Result<Vec<crate::audio::recording_saver::TranscriptSegment>, String> {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();

    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager.get_transcript_segments())
    } else {
        Ok(Vec::new()) // No recording active, return empty
    }
}

/// Get meeting name from current recording session
/// Used for syncing frontend state after page reload during active recording
#[tauri::command]
pub async fn get_recording_meeting_name() -> Result<Option<String>, String> {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();

    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager.get_meeting_name())
    } else {
        Ok(None)
    }
}

// ============================================================================
// DEVICE MONITORING COMMANDS (AirPods/Bluetooth disconnect/reconnect support)
// ============================================================================

/// Response structure for device events
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type")]
pub enum DeviceEventResponse {
    DeviceDisconnected {
        device_name: String,
        device_type: String,
    },
    DeviceReconnected {
        device_name: String,
        device_type: String,
    },
    DeviceListChanged,
}

impl From<DeviceEvent> for DeviceEventResponse {
    fn from(event: DeviceEvent) -> Self {
        match event {
            DeviceEvent::DeviceDisconnected {
                device_name,
                device_type,
            } => DeviceEventResponse::DeviceDisconnected {
                device_name,
                device_type: format!("{:?}", device_type),
            },
            DeviceEvent::DeviceReconnected {
                device_name,
                device_type,
            } => DeviceEventResponse::DeviceReconnected {
                device_name,
                device_type: format!("{:?}", device_type),
            },
            DeviceEvent::DeviceListChanged => DeviceEventResponse::DeviceListChanged,
        }
    }
}

/// Reconnection status information
#[derive(Debug, Serialize, Clone)]
pub struct ReconnectionStatus {
    pub is_reconnecting: bool,
    pub disconnected_device: Option<DisconnectedDeviceInfo>,
}

/// Information about a disconnected device
#[derive(Debug, Serialize, Clone)]
pub struct DisconnectedDeviceInfo {
    pub name: String,
    pub device_type: String,
}

/// Poll for audio device events (disconnect/reconnect)
/// Should be called periodically (every 1-2 seconds) by frontend during recording
#[tauri::command]
pub async fn poll_audio_device_events() -> Result<Option<DeviceEventResponse>, String> {
    let mut manager_guard = RECORDING_MANAGER.lock().unwrap();

    if let Some(manager) = manager_guard.as_mut() {
        if let Some(event) = manager.poll_device_events() {
            info!("📱 Device event polled: {:?}", event);
            Ok(Some(event.into()))
        } else {
            Ok(None)
        }
    } else {
        // Not recording, no events
        Ok(None)
    }
}

/// Get current reconnection status
/// Returns whether the system is attempting to reconnect and which device
#[tauri::command]
pub async fn get_reconnection_status() -> Result<ReconnectionStatus, String> {
    let manager_guard = RECORDING_MANAGER.lock().unwrap();

    if let Some(manager) = manager_guard.as_ref() {
        let state = manager.get_state();
        let disconnected_device = state
            .get_disconnected_device()
            .map(|(device, device_type)| DisconnectedDeviceInfo {
                name: device.name.clone(),
                device_type: format!("{:?}", device_type),
            });

        Ok(ReconnectionStatus {
            is_reconnecting: manager.is_reconnecting(),
            disconnected_device,
        })
    } else {
        // Not recording, no reconnection in progress
        Ok(ReconnectionStatus {
            is_reconnecting: false,
            disconnected_device: None,
        })
    }
}

/// Get information about the active audio output device
/// Used to warn users about Bluetooth playback issues
#[tauri::command]
pub async fn get_active_audio_output() -> Result<super::playback_monitor::AudioOutputInfo, String> {
    super::playback_monitor::get_active_audio_output()
        .await
        .map_err(|e| format!("Failed to get audio output info: {}", e))
}

/// Manually trigger device reconnection attempt
/// Useful for UI "Retry" button
#[tauri::command]
#[allow(clippy::await_holding_lock)] // pre-existing: isolated spawn_blocking device-reconnect task; guard lifetime is bounded here
pub async fn attempt_device_reconnect(
    device_name: String,
    device_type: String,
) -> Result<bool, String> {
    // Parse device type first
    let monitor_type = match device_type.as_str() {
        "Microphone" => DeviceMonitorType::Microphone,
        "SystemAudio" => DeviceMonitorType::SystemAudio,
        _ => return Err(format!("Invalid device type: {}", device_type)),
    };

    // Check if recording is active
    {
        let manager_guard = RECORDING_MANAGER.lock().unwrap();
        if manager_guard.is_none() {
            return Err("Recording not active".to_string());
        }
    } // Release lock

    // Spawn blocking task to handle the async reconnection
    let result = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async {
            let mut manager_guard = RECORDING_MANAGER.lock().unwrap();
            if let Some(manager) = manager_guard.as_mut() {
                manager
                    .attempt_device_reconnect(&device_name, monitor_type)
                    .await
            } else {
                Err(anyhow::anyhow!("Recording not active"))
            }
        })
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?;

    match result {
        Ok(success) => {
            if success {
                info!("✅ Manual reconnection successful");
            } else {
                warn!("❌ Manual reconnection failed - device not available");
            }
            Ok(success)
        }
        Err(e) => {
            error!("Manual reconnection error: {}", e);
            Err(e.to_string())
        }
    }
}
