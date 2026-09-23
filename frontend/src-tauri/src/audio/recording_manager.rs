use anyhow::Result;
use log::{debug, error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use super::devices::{list_audio_devices, AudioDevice};

#[cfg(target_os = "macos")]
use super::devices::get_safe_recording_devices_macos;

use super::device_monitor::{AudioDeviceMonitor, DeviceEvent, DeviceMonitorType};
#[cfg(not(target_os = "macos"))]
use super::devices::{default_input_device, default_output_device};
use super::pipeline::{AudioPipelineManager, TranscriptionChunk};
use super::recording_saver::RecordingSaver;
use super::recording_state::{DeviceType as RecordingDeviceType, RecordingState};
use super::stream::AudioStreamManager;

/// Bounded capacity of the pipeline→transcription channel (specs/0028). Was unbounded,
/// which let a slow transcriber grow this queue for the whole meeting (memory blow-up +
/// multi-minute stop hang). Tunable — see the spec's soak-test risk note.
const TRANSCRIPTION_QUEUE_CAPACITY: usize = 512;

/// Simplified recording manager that coordinates all audio components
pub struct RecordingManager {
    state: Arc<RecordingState>,
    stream_manager: AudioStreamManager,
    pipeline_manager: AudioPipelineManager,
    recording_saver: RecordingSaver,
    device_monitor: Option<AudioDeviceMonitor>,
    device_event_receiver: Option<mpsc::UnboundedReceiver<DeviceEvent>>,
}

// SAFETY (audited specs/0028): `RecordingManager` is the single logical owner of the audio
// stack (stream manager, pipeline, saver, device monitor). It is stored behind a Tokio mutex
// in Tauri state and its non-`Send` innards (cpal streams via `AudioStreamManager`) are only
// ever accessed through that exclusive lock — never concurrently from two threads. See the
// detailed invariant + deferred dedicated-thread refactor note on `StreamBackend` in stream.rs.
unsafe impl Send for RecordingManager {}

impl RecordingManager {
    /// Create a new recording manager
    pub fn new() -> Self {
        let state = RecordingState::new();
        let stream_manager = AudioStreamManager::new(state.clone());
        let pipeline_manager = AudioPipelineManager::new();
        let (device_monitor, device_event_receiver) = AudioDeviceMonitor::new();

        Self {
            state,
            stream_manager,
            pipeline_manager,
            recording_saver: RecordingSaver::new(),
            device_monitor: Some(device_monitor),
            device_event_receiver: Some(device_event_receiver),
        }
    }

    /// Start recording with specified devices
    ///
    /// # Arguments
    /// * `microphone_device` - Optional microphone device to use
    /// * `system_device` - Optional system audio device to use
    /// * `auto_save` - Whether to save audio checkpoints (true) or just transcripts/metadata (false)
    pub async fn start_recording(
        &mut self,
        microphone_device: Option<Arc<AudioDevice>>,
        system_device: Option<Arc<AudioDevice>>,
        auto_save: bool,
        // specs/0011 P3-B: when live diarization is enabled the caller constructs the
        // `LiveDiarizer` (gated on settings + models) and passes it here so the pipeline
        // can fan the pre-mix system window into it. `None` keeps live diarization off
        // (the default) — zero behaviour change vs v0.4.0.
        live_diarizer: Option<Arc<crate::diarization::live::LiveDiarizer>>,
        // specs/0029 WS7.2 + low-power-mode spec §3: the session's shared live-STT flag.
        // `false` = record-only mode (pipeline skips the VAD/STT stage; audio save,
        // per-channel WAVs, and the live level/spectrum feed are unaffected). Flippable
        // mid-recording via the toggle command.
        live_stt: Arc<AtomicBool>,
    ) -> Result<mpsc::Receiver<TranscriptionChunk>> {
        info!(
            "Starting recording manager (auto_save: {}, live_transcription: {})",
            auto_save,
            live_stt.load(Ordering::SeqCst)
        );

        // Set up transcription channel.
        // specs/0028: BOUNDED (was unbounded) so a slow transcriber can't grow this queue
        // without limit. The pipeline applies an explicit drop policy on saturation
        // (see pipeline.rs); the worker emits `transcription-falling-behind`.
        // specs/0029 WS3.4: carries `TranscriptionChunk` (audio + capture-channel tag).
        let (transcription_sender, transcription_receiver) =
            mpsc::channel::<TranscriptionChunk>(TRANSCRIPTION_QUEUE_CAPACITY);

        // CRITICAL FIX: Create recording sender for pre-mixed audio from pipeline
        // Pipeline will mix mic + system audio professionally and send to this channel
        // Pass auto_save to control whether audio checkpoints are created.
        // specs/0037: `?` propagates a failed RESUME-folder init — a resumed session
        // must never appear live while recording nothing into the meeting it claims to
        // resume (fresh-session folder init stays best-effort inside the saver).
        let recording_sender = self.recording_saver.start_accumulation(auto_save)?;

        // specs/0037: any failure from here on (pipeline/stream start) must not leave a
        // resumed meeting folder destructively mutated — roll the saver's journaled
        // resume-init back so e.g. a cleanly-completed folder keeps status "completed",
        // its canonical audio.mp4/system.wav/mic.wav names, and no phantom
        // `.checkpoints/` ("Unfinished recording" offers at every launch).
        match self
            .start_recording_inner(
                microphone_device,
                system_device,
                live_diarizer,
                live_stt,
                transcription_sender,
                recording_sender,
            )
            .await
        {
            Ok(()) => {
                self.recording_saver.commit_start();
                Ok(transcription_receiver)
            }
            Err(e) => {
                error!("Recording start failed after saver init: {}", e);
                self.recording_saver.rollback_failed_start().await;
                Err(e)
            }
        }
    }

    /// The fallible tail of [`start_recording`](Self::start_recording): everything after
    /// the saver's accumulation/folder init. Split out so the caller can roll the
    /// saver's resume-init back when any of these steps fail (specs/0037).
    async fn start_recording_inner(
        &mut self,
        microphone_device: Option<Arc<AudioDevice>>,
        system_device: Option<Arc<AudioDevice>>,
        live_diarizer: Option<Arc<crate::diarization::live::LiveDiarizer>>,
        live_stt: Arc<AtomicBool>,
        transcription_sender: mpsc::Sender<TranscriptionChunk>,
        recording_sender: mpsc::UnboundedSender<super::recording_state::AudioChunk>,
    ) -> Result<()> {
        // specs/0010 P1-B1: the meeting folder is created during start_accumulation.
        // Pass it to the pipeline so it can additionally write per-channel system.wav /
        // mic.wav (16 kHz mono) for offline diarization. None when there is no folder.
        let diarization_meeting_folder = self.recording_saver.get_meeting_folder().cloned();

        // Start recording state first
        self.state.start_recording()?;

        // Get device information for adaptive mixing
        // The pipeline uses device kind (Bluetooth vs Wired) to apply adaptive buffering:
        // - Bluetooth: Larger buffers (80-200ms) to handle jitter
        // - Wired: Smaller buffers (20-50ms) for low latency
        let (mic_name, mic_kind) = if let Some(ref mic) = microphone_device {
            let device_kind =
                super::device_detection::InputDeviceKind::detect(&mic.name, 512, 48000);
            (mic.name.clone(), device_kind)
        } else {
            (
                "No Microphone".to_string(),
                super::device_detection::InputDeviceKind::Unknown,
            )
        };

        let (sys_name, sys_kind) = if let Some(ref sys) = system_device {
            let device_kind =
                super::device_detection::InputDeviceKind::detect(&sys.name, 512, 48000);
            (sys.name.clone(), device_kind)
        } else {
            (
                "No System Audio".to_string(),
                super::device_detection::InputDeviceKind::Unknown,
            )
        };

        // Update recording metadata with device information
        self.recording_saver.set_device_info(
            microphone_device.as_ref().map(|d| d.name.clone()),
            system_device.as_ref().map(|d| d.name.clone()),
        );

        // Start the audio processing pipeline with FFmpeg adaptive mixer
        // Pipeline will: 1) Mix mic+system audio with adaptive buffering, 2) Send mixed to recording_sender,
        // 3) Apply VAD and send speech segments to transcription
        self.pipeline_manager.start(
            self.state.clone(),
            transcription_sender,
            0,                      // Ignored - using dynamic sizing internally
            48000,                  // 48kHz sample rate
            Some(recording_sender), // CRITICAL: Pass recording sender to receive pre-mixed audio
            mic_name,
            mic_kind,
            sys_name,
            sys_kind,
            diarization_meeting_folder,
            // specs/0011 P3-B: live diarizer (constructed + gated by the caller).
            // `None` keeps live diarization off by default / no behaviour change.
            live_diarizer,
            // specs/0029 WS7.2 + low-power-mode spec §3: shared live-STT flag gating
            // the pipeline's VAD/STT stage (attach/detach mid-recording).
            live_stt,
        )?;

        // Give the pipeline a moment to fully initialize before starting streams
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Start audio streams - they send RAW unmixed chunks to pipeline for mixing
        // Pipeline handles mixing and distribution to both recording and transcription
        self.stream_manager
            .start_streams(microphone_device.clone(), system_device.clone(), None)
            .await?;

        // Start device monitoring to detect disconnects
        if let Some(ref mut monitor) = self.device_monitor {
            if let Err(e) = monitor.start_monitoring(microphone_device, system_device) {
                warn!("Failed to start device monitoring: {}", e);
                // Non-fatal - continue without monitoring
            } else {
                info!("✅ Device monitoring started");
            }
        }

        info!(
            "Recording manager started successfully with {} active streams",
            self.stream_manager.active_stream_count()
        );

        Ok(())
    }

    /// Start recording with default devices and auto_save setting
    ///
    /// # Arguments
    /// * `auto_save` - Whether to save audio checkpoints (true) or just transcripts/metadata (false)
    ///
    /// # Platform-Specific Behavior
    ///
    /// **macOS**: Uses smart device selection that automatically overrides
    /// Bluetooth devices to built-in wired devices for stable, consistent sample rates.
    /// This prevents Core Audio/ScreenCaptureKit from delivering variable sample rate
    /// streams that cause sync issues when mixing mic + system audio.
    ///
    /// **Windows/Linux**: Uses system default devices directly without override.
    ///
    /// # macOS Bluetooth Override Strategy
    ///
    /// - Microphone: If Bluetooth → Use built-in MacBook mic
    /// - Speaker: If Bluetooth → Use built-in MacBook speaker (for ScreenCaptureKit)
    /// - Each device is checked INDEPENDENTLY
    ///
    /// Rationale: Bluetooth devices on macOS can have variable sample rates as Core Audio
    /// and the Bluetooth stack may resample dynamically. Built-in devices provide
    /// fixed, consistent sample rates for reliable audio mixing.
    ///
    /// User still hears audio via Bluetooth (playback), but recording captures
    /// via stable wired path for best quality.
    pub async fn start_recording_with_defaults_and_auto_save(
        &mut self,
        auto_save: bool,
    ) -> Result<mpsc::Receiver<TranscriptionChunk>> {
        #[cfg(target_os = "macos")]
        {
            info!("🎙️ [macOS] Starting recording with smart device selection (Bluetooth override enabled)");

            // Get safe recording devices with automatic Bluetooth fallback
            // This function handles all the detection and override logic for macOS
            let (microphone_device, system_device) = get_safe_recording_devices_macos()?;

            // Wrap in Arc for sharing across threads
            let microphone_device = microphone_device.map(Arc::new);
            let system_device = system_device.map(Arc::new);

            // Ensure at least microphone is available
            if microphone_device.is_none() {
                return Err(anyhow::anyhow!(
                    "❌ No microphone device available for recording"
                ));
            }

            // Start recording with selected devices and auto_save setting.
            // This convenience path does not wire live diarization (the IPC commands do)
            // and always transcribes live (record-only mode is an IPC-path setting).
            self.start_recording(
                microphone_device,
                system_device,
                auto_save,
                None,
                Arc::new(AtomicBool::new(true)),
            )
            .await
        }

        #[cfg(not(target_os = "macos"))]
        {
            info!("Starting recording with default devices");

            // Get default devices (no Bluetooth override on Windows/Linux)
            let microphone_device = match default_input_device() {
                Ok(device) => {
                    info!("Using default microphone: {}", device.name);
                    Some(Arc::new(device))
                }
                Err(e) => {
                    warn!("No default microphone available: {}", e);
                    None
                }
            };

            let system_device = match default_output_device() {
                Ok(device) => {
                    info!("Using default system audio: {}", device.name);
                    Some(Arc::new(device))
                }
                Err(e) => {
                    warn!("No default system audio available: {}", e);
                    None
                }
            };

            // Ensure at least microphone is available
            if microphone_device.is_none() {
                return Err(anyhow::anyhow!("No microphone device available"));
            }

            self.start_recording(
                microphone_device,
                system_device,
                auto_save,
                None,
                Arc::new(AtomicBool::new(true)),
            )
            .await
        }
    }

    /// Stop recording streams without saving (for use when waiting for transcription)
    pub async fn stop_streams_only(&mut self) -> Result<()> {
        info!("Stopping recording streams only");

        // Stop device monitoring
        if let Some(ref mut monitor) = self.device_monitor {
            monitor.stop_monitoring().await;
        }

        // Stop recording state first
        self.state.stop_recording();

        // Stop audio streams
        if let Err(e) = self.stream_manager.stop_streams() {
            error!("Error stopping audio streams: {}", e);
        }

        // Stop audio pipeline
        if let Err(e) = self.pipeline_manager.stop().await {
            error!("Error stopping audio pipeline: {}", e);
        }

        debug!("Recording streams stopped successfully");
        Ok(())
    }

    /// Stop streams and force immediate pipeline flush to process all accumulated audio
    pub async fn stop_streams_and_force_flush(&mut self) -> Result<()> {
        info!("🚀 Stopping recording streams with IMMEDIATE pipeline flush");

        // CRITICAL: Stop device monitor FIRST to prevent continuous WASAPI polling on Windows
        // This fixes the slow shutdown issue where device enumeration runs for 90+ seconds
        if let Some(ref mut monitor) = self.device_monitor {
            info!("Stopping device monitor first...");
            monitor.stop_monitoring().await;
        }

        // Stop recording state first - this clears device references
        self.state.stop_recording();

        // Stop audio streams immediately
        if let Err(e) = self.stream_manager.stop_streams() {
            error!("Error stopping audio streams: {}", e);
        }

        // CRITICAL: Force pipeline to flush ALL accumulated audio before stopping
        debug!("💨 Forcing pipeline to flush accumulated audio immediately");
        if let Err(e) = self.pipeline_manager.force_flush_and_stop().await {
            error!("Error during force flush: {}", e);
        }

        // CRITICAL: Full cleanup to release all Arc references and resources
        // This ensures microphone is released even if Drop is delayed
        self.state.cleanup();

        info!("✅ Recording streams stopped with immediate flush completed");
        Ok(())
    }

    /// Save recording after transcription is complete
    pub async fn save_recording_only<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
    ) -> Result<()> {
        debug!("Saving recording with transcript chunks");

        // Get actual recording duration from state
        // specs/0071 W5 — logs every term behind the number, not just the number.
        let recording_duration = self.state.duration_for_save("save_recording_only");
        info!("Recording duration from state: {:?}s", recording_duration);

        // Save the recording with actual duration
        match self
            .recording_saver
            .stop_and_save(app, recording_duration)
            .await
        {
            Ok(Some(file_path)) => {
                info!("Recording saved successfully to: {}", file_path);
            }
            Ok(None) => {
                debug!("Recording not saved (auto-save disabled or no audio data)");
            }
            Err(e) => {
                error!("Failed to save recording: {}", e);
                // Don't fail the stop operation if saving fails
            }
        }

        debug!("Recording save operation completed");
        Ok(())
    }

    /// Stop recording and save audio (legacy method)
    pub async fn stop_recording<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
    ) -> Result<()> {
        info!("Stopping recording manager");

        // Get recording duration BEFORE stopping (important!)
        let recording_duration = self.state.duration_for_save("stop_and_save");
        info!("Recording duration before stop: {:?}s", recording_duration);

        // Stop recording state first
        self.state.stop_recording();

        // Stop audio streams
        if let Err(e) = self.stream_manager.stop_streams() {
            error!("Error stopping audio streams: {}", e);
        }

        // Stop audio pipeline
        if let Err(e) = self.pipeline_manager.stop().await {
            error!("Error stopping audio pipeline: {}", e);
        }

        // Save the recording with actual duration
        match self
            .recording_saver
            .stop_and_save(app, recording_duration)
            .await
        {
            Ok(Some(file_path)) => {
                info!("Recording saved successfully to: {}", file_path);
            }
            Ok(None) => {
                info!("Recording not saved (auto-save disabled or no audio data)");
            }
            Err(e) => {
                error!("Failed to save recording: {}", e);
                // Don't fail the stop operation if saving fails
            }
        }

        info!("Recording manager stopped");
        Ok(())
    }

    /// Get recording stats from the saver
    pub fn get_recording_stats(&self) -> (usize, u32) {
        self.recording_saver.get_stats()
    }

    /// Check if currently recording
    pub fn is_recording(&self) -> bool {
        self.state.is_recording()
    }

    /// Pause the current recording session
    pub fn pause_recording(&self) -> Result<()> {
        info!("Pausing recording");
        self.state.pause_recording()
    }

    /// Resume the current recording session
    pub fn resume_recording(&self) -> Result<()> {
        info!("Resuming recording");
        self.state.resume_recording()
    }

    /// Check if recording is currently paused
    pub fn is_paused(&self) -> bool {
        self.state.is_paused()
    }

    /// Check if recording is active (recording and not paused)
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Get recording statistics
    pub fn get_stats(&self) -> super::recording_state::RecordingStats {
        self.state.get_stats()
    }

    /// Get recording duration
    pub fn get_recording_duration(&self) -> Option<f64> {
        self.state.get_recording_duration()
    }

    /// Get active recording duration (excluding pauses)
    pub fn get_active_recording_duration(&self) -> Option<f64> {
        self.state.get_active_recording_duration()
    }

    /// Get total pause duration
    pub fn get_total_pause_duration(&self) -> f64 {
        self.state.get_total_pause_duration()
    }

    /// Get current pause duration if paused
    pub fn get_current_pause_duration(&self) -> Option<f64> {
        self.state.get_current_pause_duration()
    }

    /// Get error information
    pub fn get_error_info(&self) -> (u32, Option<super::recording_state::AudioError>) {
        (self.state.get_error_count(), self.state.get_last_error())
    }

    /// Get active stream count
    pub fn active_stream_count(&self) -> usize {
        self.stream_manager.active_stream_count()
    }

    /// Set error callback for handling errors
    pub fn set_error_callback<F>(&self, callback: F)
    where
        F: Fn(&super::recording_state::AudioError) + Send + Sync + 'static,
    {
        self.state.set_error_callback(callback);
    }

    /// Check if there's a fatal error
    pub fn has_fatal_error(&self) -> bool {
        self.state.has_fatal_error()
    }

    /// Set the meeting name for this recording session
    pub fn set_meeting_name(&mut self, name: Option<String>) {
        self.recording_saver.set_meeting_name(name);
    }

    /// Set the DB meeting id to link the folder to its row (specs/0037). Delegates to the
    /// saver, which persists it into `metadata.json` at init.
    pub fn set_meeting_id(&mut self, id: Option<String>) {
        self.recording_saver.set_meeting_id(id);
    }

    /// The authoritative DB `meeting_id` this recording is bound to (v1.6.1). Delegates to
    /// the saver. Included in the stop payload so the save can't be misdirected by the
    /// frontend's mutable current-meeting selection.
    pub fn get_meeting_id(&self) -> Option<String> {
        self.recording_saver.get_meeting_id()
    }

    /// Enable RESUME mode (specs/0037): the next `start_recording` reuses `resume_folder`
    /// for the given `meeting_id` and appends a new segment. Delegates to the saver.
    pub fn set_resume_context(&mut self, meeting_id: String, resume_folder: std::path::PathBuf) {
        self.recording_saver
            .set_resume_context(meeting_id, resume_folder);
    }

    /// Hand the saver its meeting's folder lease (specs/0073). Delegates to the saver.
    pub fn set_folder_lease(&mut self, lease: Option<super::folder_lease::FolderLease>) {
        self.recording_saver.set_folder_lease(lease);
    }

    /// Audio offset (seconds) for the current session in resume mode = sum of prior
    /// segments' durations (`0.0` for a fresh recording). The stop path passes this to the
    /// DB as the transcript-append offset. Delegates to the saver.
    pub fn prior_audio_duration_seconds(&self) -> f64 {
        self.recording_saver.prior_audio_duration_seconds()
    }

    /// Add a structured transcript segment to be saved later
    pub fn add_transcript_segment(&self, segment: super::recording_saver::TranscriptSegment) {
        self.recording_saver.add_transcript_segment(segment);
    }

    /// Add a transcript chunk to be saved later (legacy method)
    pub fn add_transcript_chunk(&self, text: String) {
        self.recording_saver.add_transcript_chunk(text);
    }

    /// Get accumulated transcript segments from current recording session
    /// Used for syncing frontend state after page reload during active recording
    pub fn get_transcript_segments(&self) -> Vec<super::recording_saver::TranscriptSegment> {
        self.recording_saver.get_transcript_segments()
    }

    /// Get meeting name from current recording session
    /// Used for syncing frontend state after page reload during active recording
    pub fn get_meeting_name(&self) -> Option<String> {
        self.recording_saver.get_meeting_name()
    }

    /// Cleanup all resources without saving
    pub async fn cleanup_without_save(&mut self) {
        if self.is_recording() {
            debug!("Stopping recording without saving during cleanup");

            // Stop recording state first
            self.state.stop_recording();

            // Stop audio streams
            if let Err(e) = self.stream_manager.stop_streams() {
                error!("Error stopping audio streams during cleanup: {}", e);
            }

            // Stop audio pipeline
            if let Err(e) = self.pipeline_manager.stop().await {
                error!("Error stopping audio pipeline during cleanup: {}", e);
            }
        }
        self.state.cleanup();
    }

    /// Get the meeting folder path (if available)
    /// Returns None if no meeting name was set or folder structure not initialized
    pub fn get_meeting_folder(&self) -> Option<std::path::PathBuf> {
        self.recording_saver.get_meeting_folder().cloned()
    }

    /// Check for device events (disconnects/reconnects)
    /// Returns Some(DeviceEvent) if an event occurred, None otherwise
    pub fn poll_device_events(&mut self) -> Option<DeviceEvent> {
        if let Some(ref mut receiver) = self.device_event_receiver {
            receiver.try_recv().ok()
        } else {
            None
        }
    }

    /// Attempt to reconnect a disconnected device
    /// Returns true if reconnection successful
    pub async fn attempt_device_reconnect(
        &mut self,
        device_name: &str,
        device_type: DeviceMonitorType,
    ) -> Result<bool> {
        info!(
            "🔄 Attempting to reconnect device: {} ({:?})",
            device_name, device_type
        );

        // List current devices
        let available_devices = list_audio_devices().await?;

        // Find the device by name
        let device = available_devices
            .iter()
            .find(|d| d.name == device_name)
            .cloned();

        if let Some(device) = device {
            info!("✅ Device '{}' found, recreating stream...", device_name);

            // Determine which device to reconnect based on type
            let device_arc: Arc<AudioDevice> = Arc::new(device);
            match device_type {
                DeviceMonitorType::Microphone => {
                    // Stop existing mic stream and start new one
                    // We need to keep system audio running if it exists
                    let system_device = self.state.get_system_device();

                    // Restart streams with new microphone
                    self.stream_manager.stop_streams()?;
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    self.stream_manager
                        .start_streams(Some(device_arc.clone()), system_device, None)
                        .await?;
                    self.state.set_microphone_device(device_arc);

                    info!("✅ Microphone reconnected successfully");
                    Ok(true)
                }
                DeviceMonitorType::SystemAudio => {
                    // Stop existing system audio stream and start new one
                    let microphone_device = self.state.get_microphone_device();

                    // Restart streams with new system audio
                    self.stream_manager.stop_streams()?;
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    self.stream_manager
                        .start_streams(microphone_device, Some(device_arc.clone()), None)
                        .await?;
                    self.state.set_system_device(device_arc);

                    info!("✅ System audio reconnected successfully");
                    Ok(true)
                }
            }
        } else {
            warn!("❌ Device '{}' not yet available", device_name);
            Ok(false)
        }
    }

    /// Handle a device disconnect event
    /// Pauses recording and attempts reconnection
    pub async fn handle_device_disconnect(
        &mut self,
        device_name: String,
        device_type: DeviceMonitorType,
    ) {
        warn!(
            "📱 Device disconnected: {} ({:?})",
            device_name, device_type
        );

        // Mark state as reconnecting (keeps recording alive but in waiting state)
        let device = match device_type {
            DeviceMonitorType::Microphone => self.state.get_microphone_device(),
            DeviceMonitorType::SystemAudio => self.state.get_system_device(),
        };

        if let Some(device) = device {
            let recording_device_type = match device_type {
                DeviceMonitorType::Microphone => RecordingDeviceType::Microphone,
                DeviceMonitorType::SystemAudio => RecordingDeviceType::System,
            };
            self.state.start_reconnecting(device, recording_device_type);
        }
    }

    /// Handle a device reconnect event
    pub async fn handle_device_reconnect(
        &mut self,
        device_name: String,
        device_type: DeviceMonitorType,
    ) -> Result<()> {
        info!("📱 Device reconnected: {} ({:?})", device_name, device_type);

        // Attempt to reconnect the device
        match self
            .attempt_device_reconnect(&device_name, device_type)
            .await
        {
            Ok(true) => {
                info!("✅ Successfully reconnected device: {}", device_name);
                self.state.stop_reconnecting();
                Ok(())
            }
            Ok(false) => {
                warn!("Device reconnect attempt failed (device not yet available)");
                Err(anyhow::anyhow!("Device not available"))
            }
            Err(e) => {
                error!("Device reconnect failed: {}", e);
                Err(e)
            }
        }
    }

    /// Check if currently attempting to reconnect
    pub fn is_reconnecting(&self) -> bool {
        self.state.is_reconnecting()
    }

    /// Get reference to recording state for external access
    pub fn get_state(&self) -> &Arc<RecordingState> {
        &self.state
    }
}

impl Default for RecordingManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RecordingManager {
    fn drop(&mut self) {
        // Note: Can't call async cleanup in Drop, but streams have their own Drop implementations
        self.state.cleanup();
    }
}
