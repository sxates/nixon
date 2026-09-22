use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;

use super::buffer_pool::AudioBufferPool;
use super::recording_duration::DurationAccounting;
use super::devices::AudioDevice;

/// Device type for audio chunks
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceType {
    Microphone,
    System,
}

/// Audio chunk with metadata for processing
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub timestamp: f64,
    pub chunk_id: u64,
    pub device_type: DeviceType,
}

/// Processed audio chunk (post-VAD) for recording
#[derive(Debug, Clone)]
pub struct ProcessedAudioChunk {
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub timestamp: f64,
    pub device_type: DeviceType,
}

/// Comprehensive error types for audio system
#[derive(Debug, Clone)]
pub enum AudioError {
    DeviceDisconnected,
    StreamFailed,
    ProcessingFailed,
    TranscriptionFailed,
    ChannelClosed,
    InitializationFailed,
    ConfigurationError,
    PermissionDenied,
    BufferOverflow,
    SampleRateUnsupported,
}

impl AudioError {
    /// Check if error is recoverable (can attempt reconnection)
    pub fn is_recoverable(&self) -> bool {
        match self {
            // Device disconnect is now recoverable - we can attempt reconnection
            AudioError::DeviceDisconnected => true,
            AudioError::StreamFailed => true,
            AudioError::ProcessingFailed => true,
            AudioError::TranscriptionFailed => true,
            AudioError::ChannelClosed => false,
            AudioError::InitializationFailed => false,
            AudioError::ConfigurationError => false,
            AudioError::PermissionDenied => false,
            AudioError::BufferOverflow => true,
            AudioError::SampleRateUnsupported => false,
        }
    }

    /// Get user-friendly error message
    pub fn user_message(&self) -> &'static str {
        match self {
            AudioError::DeviceDisconnected => "Audio device was disconnected",
            AudioError::StreamFailed => "Audio stream encountered an error",
            AudioError::ProcessingFailed => "Audio processing failed",
            AudioError::TranscriptionFailed => "Speech transcription failed",
            AudioError::ChannelClosed => "Audio channel was closed unexpectedly",
            AudioError::InitializationFailed => "Failed to initialize audio system",
            AudioError::ConfigurationError => "Audio configuration error",
            AudioError::PermissionDenied => "Microphone permission denied",
            AudioError::BufferOverflow => "Audio buffer overflow",
            AudioError::SampleRateUnsupported => "Audio sample rate not supported",
        }
    }
}

/// Recording statistics
#[derive(Debug, Default)]
pub struct RecordingStats {
    pub chunks_processed: u64,
    pub total_duration: f64,
    pub last_activity: Option<Instant>,
}

/// Unified state management for audio recording
pub struct RecordingState {
    // Core recording state
    is_recording: AtomicBool,
    is_paused: AtomicBool,
    is_reconnecting: AtomicBool, // NEW: Attempting to reconnect to device

    // Audio devices
    microphone_device: Mutex<Option<Arc<AudioDevice>>>,
    system_device: Mutex<Option<Arc<AudioDevice>>>,
    // Track which device is disconnected for reconnection attempts
    disconnected_device: Mutex<Option<(Arc<AudioDevice>, DeviceType)>>,

    // Audio pipeline.
    // specs/0028: BOUNDED capture→pipeline channel (was unbounded) so a stalled pipeline
    // consumer can't grow this queue without limit. `send_audio_chunk` applies an explicit
    // drop-newest policy on saturation and counts drops in `audio_chunks_dropped`.
    audio_sender: Mutex<Option<mpsc::Sender<AudioChunk>>>,
    // Count of capture chunks shed because the bounded pipeline queue was full (specs/0028).
    audio_chunks_dropped: AtomicU64,

    // Memory optimization
    buffer_pool: AudioBufferPool,

    // Error handling
    error_count: AtomicU32,
    recoverable_error_count: AtomicU32,
    last_error: Mutex<Option<AudioError>>,
    #[allow(clippy::type_complexity)]
    // cohesive channel/callback tuple; extracting a type alias adds no clarity
    error_callback: Mutex<Option<Box<dyn Fn(&AudioError) + Send + Sync>>>,

    // Statistics
    stats: Mutex<RecordingStats>,

    // Recording start time for accurate timestamps
    recording_start: Mutex<Option<Instant>>,
    /// The duration accounting captured the moment recording ended, so the save path can
    /// still read it after `cleanup()` has wiped the clocks (specs/0071 W5 follow-up).
    final_duration: Mutex<Option<DurationAccounting>>,
    // Pause time tracking
    pause_start: Mutex<Option<Instant>>,
    total_pause_duration: Mutex<std::time::Duration>,
}

impl RecordingState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            is_recording: AtomicBool::new(false),
            is_paused: AtomicBool::new(false),
            is_reconnecting: AtomicBool::new(false),
            microphone_device: Mutex::new(None),
            system_device: Mutex::new(None),
            disconnected_device: Mutex::new(None),
            audio_sender: Mutex::new(None),
            audio_chunks_dropped: AtomicU64::new(0),
            buffer_pool: AudioBufferPool::new(16, 48000), // Pool of 16 buffers with 48kHz samples capacity
            error_count: AtomicU32::new(0),
            recoverable_error_count: AtomicU32::new(0),
            last_error: Mutex::new(None),
            error_callback: Mutex::new(None),
            stats: Mutex::new(RecordingStats::default()),
            recording_start: Mutex::new(None),
            final_duration: Mutex::new(None),
            pause_start: Mutex::new(None),
            total_pause_duration: Mutex::new(std::time::Duration::ZERO),
        })
    }

    // Recording control
    pub fn start_recording(&self) -> Result<()> {
        self.is_recording.store(true, Ordering::SeqCst);
        *self.recording_start.lock().unwrap() = Some(Instant::now());
        // A new session must not inherit the previous one's captured duration.
        *self.final_duration.lock().unwrap() = None;
        self.error_count.store(0, Ordering::SeqCst);
        self.recoverable_error_count.store(0, Ordering::SeqCst);
        *self.last_error.lock().unwrap() = None;
        Ok(())
    }

    pub fn stop_recording(&self) {
        // FIRST, before anything below clears a clock: capture what the recording's duration
        // actually was.
        //
        // `cleanup()` (called from the stop path via `stop_streams_only`) wipes
        // `recording_start`, and it runs BEFORE `save_recording_only` reads the duration — so
        // the save saw `None` and `recording_saver.rs` fell back to the last transcript
        // segment's end time, i.e. "when speech last stopped" rather than "how long we
        // recorded". A meeting with trailing silence, or a paused transcript, therefore
        // stored a plausible-looking wrong number (63.19s for 67.22s of audio; 68.85s for
        // 114.6s). Never overwritten with `None`, so the second `stop_recording()` that
        // `cleanup()` performs cannot destroy what the first one captured.
        if let Some(acct) = self.duration_accounting() {
            *self.final_duration.lock().unwrap() = Some(acct);
        }
        self.is_recording.store(false, Ordering::SeqCst);
        self.is_paused.store(false, Ordering::SeqCst);
        super::mute_gate::set_muted(false); // clear the Zoom-mute gate (specs/0049)
                                            // Clear pause tracking when stopping
        *self.pause_start.lock().unwrap() = None;
        // CRITICAL: Clear audio sender to close the pipeline channel
        // This ensures the pipeline loop exits properly after processing all chunks
        *self.audio_sender.lock().unwrap() = None;
        // CRITICAL: Clear device references to release microphone/speaker
        // Without this, Arc<AudioDevice> references persist and keep the mic active
        *self.microphone_device.lock().unwrap() = None;
        *self.system_device.lock().unwrap() = None;
        *self.disconnected_device.lock().unwrap() = None;
        log::info!("Recording stopped, device references cleared");
    }

    pub fn pause_recording(&self) -> Result<()> {
        if !self.is_recording() {
            return Err(anyhow::anyhow!("Cannot pause when not recording"));
        }
        if self.is_paused() {
            return Err(anyhow::anyhow!("Recording is already paused"));
        }

        self.is_paused.store(true, Ordering::SeqCst);
        *self.pause_start.lock().unwrap() = Some(Instant::now());
        log::info!("Recording paused");
        Ok(())
    }

    pub fn resume_recording(&self) -> Result<()> {
        if !self.is_recording() {
            return Err(anyhow::anyhow!("Cannot resume when not recording"));
        }
        if !self.is_paused() {
            return Err(anyhow::anyhow!("Recording is not paused"));
        }

        // Calculate pause duration and add to total
        if let Some(pause_start) = self.pause_start.lock().unwrap().take() {
            let pause_duration = pause_start.elapsed();
            *self.total_pause_duration.lock().unwrap() += pause_duration;
            log::info!(
                "Recording resumed after pause of {:.2}s",
                pause_duration.as_secs_f64()
            );
        }

        self.is_paused.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::SeqCst)
    }

    pub fn is_active(&self) -> bool {
        self.is_recording() && !self.is_paused()
    }

    // Reconnection state management
    pub fn start_reconnecting(&self, device: Arc<AudioDevice>, device_type: DeviceType) {
        self.is_reconnecting.store(true, Ordering::SeqCst);
        *self.disconnected_device.lock().unwrap() = Some((device, device_type));
        log::info!("Started reconnection attempt for device");
    }

    pub fn stop_reconnecting(&self) {
        self.is_reconnecting.store(false, Ordering::SeqCst);
        *self.disconnected_device.lock().unwrap() = None;
        log::info!("Stopped reconnection attempt");
    }

    pub fn is_reconnecting(&self) -> bool {
        self.is_reconnecting.load(Ordering::SeqCst)
    }

    pub fn get_disconnected_device(&self) -> Option<(Arc<AudioDevice>, DeviceType)> {
        self.disconnected_device.lock().unwrap().clone()
    }

    // Device management
    pub fn set_microphone_device(&self, device: Arc<AudioDevice>) {
        *self.microphone_device.lock().unwrap() = Some(device);
    }

    pub fn set_system_device(&self, device: Arc<AudioDevice>) {
        *self.system_device.lock().unwrap() = Some(device);
    }

    pub fn get_microphone_device(&self) -> Option<Arc<AudioDevice>> {
        self.microphone_device.lock().unwrap().clone()
    }

    pub fn get_system_device(&self) -> Option<Arc<AudioDevice>> {
        self.system_device.lock().unwrap().clone()
    }

    // Audio pipeline management
    pub fn set_audio_sender(&self, sender: mpsc::Sender<AudioChunk>) {
        *self.audio_sender.lock().unwrap() = Some(sender);
    }

    pub fn send_audio_chunk(&self, chunk: AudioChunk) -> Result<()> {
        // Don't send audio chunks when paused
        if self.is_paused() {
            return Ok(()); // Silently discard chunks while paused
        }

        // specs/0049: while the owner is muted (in Zoom), drop MICROPHONE chunks so the
        // mixer zero-pads the mic window — muted spans become system-only in the
        // transcript and silent in mic.wav. System/remote audio is unaffected. The flag
        // is a process-wide gate (audio/mute_gate.rs) the Zoom-mute poll sets.
        if chunk.device_type == DeviceType::Microphone && super::mute_gate::is_muted() {
            return Ok(());
        }

        if let Some(sender) = self.audio_sender.lock().unwrap().as_ref() {
            // specs/0028: BOUNDED channel with an explicit drop-newest policy. `try_send`
            // never blocks the capture path; if the pipeline consumer has stalled and the
            // queue is full we shed this chunk (bounding memory) rather than growing without
            // limit. Under normal load there is always space, so nothing is dropped.
            match sender.try_send(chunk) {
                Ok(()) => {
                    // Update statistics
                    let mut stats = self.stats.lock().unwrap();
                    stats.chunks_processed += 1;
                    stats.last_activity = Some(Instant::now());
                    Ok(())
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let dropped = self.audio_chunks_dropped.fetch_add(1, Ordering::Relaxed) + 1;
                    if dropped == 1 || dropped.is_multiple_of(200) {
                        log::warn!("⚠️ Capture→pipeline queue saturated; dropped {} chunk(s) (pipeline falling behind)", dropped);
                    }
                    Ok(())
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(anyhow::anyhow!(
                    "Failed to send audio chunk: pipeline channel closed"
                )),
            }
        } else {
            // Return an error when no sender is available (pipeline not ready)
            Err(anyhow::anyhow!(
                "Audio pipeline not ready - no sender available"
            ))
        }
    }

    // Error handling
    pub fn set_error_callback<F>(&self, callback: F)
    where
        F: Fn(&AudioError) + Send + Sync + 'static,
    {
        *self.error_callback.lock().unwrap() = Some(Box::new(callback));
    }

    pub fn report_error(&self, error: AudioError) {
        let count = self.error_count.fetch_add(1, Ordering::SeqCst) + 1;

        // Track recoverable vs non-recoverable errors separately
        if error.is_recoverable() {
            let recoverable_count = self.recoverable_error_count.fetch_add(1, Ordering::SeqCst) + 1;
            log::warn!(
                "Recoverable audio error ({}): {:?}",
                recoverable_count,
                error
            );

            // Allow more recoverable errors before stopping
            if recoverable_count >= 10 {
                log::error!(
                    "Too many recoverable errors ({}), stopping recording",
                    recoverable_count
                );
                self.stop_recording();
            }
        } else {
            log::error!("Non-recoverable audio error: {:?}", error);
            // Stop immediately for non-recoverable errors
            self.stop_recording();
        }

        *self.last_error.lock().unwrap() = Some(error.clone());

        // Call error callback if set
        if let Some(callback) = self.error_callback.lock().unwrap().as_ref() {
            callback(&error);
        }

        // Fallback: stop recording after too many total errors
        if count >= 15 {
            log::error!(
                "Too many total audio errors ({}), stopping recording",
                count
            );
            self.stop_recording();
        }
    }

    pub fn get_error_count(&self) -> u32 {
        self.error_count.load(Ordering::SeqCst)
    }

    pub fn get_recoverable_error_count(&self) -> u32 {
        self.recoverable_error_count.load(Ordering::SeqCst)
    }

    pub fn get_last_error(&self) -> Option<AudioError> {
        self.last_error.lock().unwrap().clone()
    }

    pub fn has_fatal_error(&self) -> bool {
        if let Some(error) = &*self.last_error.lock().unwrap() {
            !error.is_recoverable() && self.error_count.load(Ordering::SeqCst) > 0
        } else {
            false
        }
    }

    // Statistics
    pub fn get_stats(&self) -> RecordingStats {
        self.stats.lock().unwrap().clone()
    }

    pub fn get_recording_duration(&self) -> Option<f64> {
        self.recording_start
            .lock()
            .unwrap()
            .map(|start| start.elapsed().as_secs_f64())
    }

    pub fn get_active_recording_duration(&self) -> Option<f64> {
        self.recording_start.lock().unwrap().map(|start| {
            let total_duration = start.elapsed().as_secs_f64();
            let pause_duration = self.get_total_pause_duration();
            let current_pause = if self.is_paused() {
                self.pause_start
                    .lock()
                    .unwrap()
                    .map(|p| p.elapsed().as_secs_f64())
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            total_duration - pause_duration - current_pause
        })
    }

    pub fn get_total_pause_duration(&self) -> f64 {
        self.total_pause_duration.lock().unwrap().as_secs_f64()
    }

    /// The duration that gets stored, having first logged every term behind it.
    ///
    /// One call rather than a log statement beside each `get_active_recording_duration()`:
    /// the logging belongs to the number, and `recording_manager.rs` sits exactly on the
    /// 800-line cap (specs/0065), so it cannot afford four lines per stop path for something
    /// this mechanical.
    pub fn duration_for_save(&self, context: &str) -> Option<f64> {
        // Live state first (a save that somehow runs before cleanup), then the value captured
        // at `stop_recording`. The log fires in EVERY branch, including the one where there is
        // nothing to report — the first version of this only logged inside `if let Some(..)`,
        // which made it silent in exactly the case that was broken.
        if let Some(acct) = self.duration_accounting() {
            acct.log(context, "live state");
            return Some(acct.active);
        }
        if let Some(acct) = *self.final_duration.lock().unwrap() {
            acct.log(context, "captured at stop");
            return Some(acct.active);
        }
        log::warn!(
            "recording duration accounting [{context}]: no clocks and no captured value — \
             the saver will fall back to the last transcript segment's end time, which \
             measures when speech stopped, not how long the recording ran"
        );
        None
    }

    /// Every input to [`Self::get_active_recording_duration`], so a wrong recorded duration
    /// can be attributed instead of guessed at.
    ///
    /// specs/0071 W5. A recording on 2026-09-21 stored `duration_seconds: 68.85` for 114.6s
    /// of audio (ffmpeg-measured on all three files). The arithmetic here is
    /// `elapsed - pauses - current_pause`, so the 46-second shortfall is either a short
    /// `elapsed` or ~46s that accrued as pause — and those have completely different causes.
    /// It could not be told apart after the fact: the Zoom mute gate is exonerated (it gates
    /// the mic and never pauses), `pause_recording` has only the HOLD caller, and the
    /// session's log had been truncated past the recording.
    ///
    /// So this is deliberately NOT a fix. It is the evidence the next occurrence needs.
    pub fn duration_accounting(&self) -> Option<DurationAccounting> {
        self.recording_start.lock().unwrap().map(|start| {
            let elapsed = start.elapsed().as_secs_f64();
            let pauses = self.get_total_pause_duration();
            let current_pause = if self.is_paused() {
                self.pause_start
                    .lock()
                    .unwrap()
                    .map(|p| p.elapsed().as_secs_f64())
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            DurationAccounting {
                elapsed,
                pauses,
                current_pause,
                active: elapsed - pauses - current_pause,
            }
        })
    }

    pub fn get_current_pause_duration(&self) -> Option<f64> {
        if self.is_paused() {
            self.pause_start
                .lock()
                .unwrap()
                .map(|start| start.elapsed().as_secs_f64())
        } else {
            None
        }
    }

    // Memory management
    pub fn get_buffer_pool(&self) -> AudioBufferPool {
        self.buffer_pool.clone()
    }

    // Cleanup
    pub fn cleanup(&self) {
        self.stop_recording();
        self.stop_reconnecting();
        *self.microphone_device.lock().unwrap() = None;
        *self.system_device.lock().unwrap() = None;
        *self.disconnected_device.lock().unwrap() = None;
        *self.audio_sender.lock().unwrap() = None;
        *self.last_error.lock().unwrap() = None;
        *self.error_callback.lock().unwrap() = None;
        *self.stats.lock().unwrap() = RecordingStats::default();
        *self.recording_start.lock().unwrap() = None;
        *self.pause_start.lock().unwrap() = None;
        *self.total_pause_duration.lock().unwrap() = std::time::Duration::ZERO;
        self.error_count.store(0, Ordering::SeqCst);
        self.recoverable_error_count.store(0, Ordering::SeqCst);

        // Clear buffer pool to free memory
        self.buffer_pool.clear();
    }
}

impl Default for RecordingState {
    fn default() -> Self {
        Self {
            is_recording: AtomicBool::new(false),
            is_paused: AtomicBool::new(false),
            is_reconnecting: AtomicBool::new(false),
            microphone_device: Mutex::new(None),
            system_device: Mutex::new(None),
            disconnected_device: Mutex::new(None),
            audio_sender: Mutex::new(None),
            audio_chunks_dropped: AtomicU64::new(0),
            buffer_pool: AudioBufferPool::new(16, 48000), // Pool of 16 buffers with 48kHz samples capacity
            error_count: AtomicU32::new(0),
            recoverable_error_count: AtomicU32::new(0),
            last_error: Mutex::new(None),
            error_callback: Mutex::new(None),
            stats: Mutex::new(RecordingStats::default()),
            recording_start: Mutex::new(None),
            final_duration: Mutex::new(None),
            pause_start: Mutex::new(None),
            total_pause_duration: Mutex::new(std::time::Duration::ZERO),
        }
    }
}

// Thread-safe cloning for RecordingStats
impl Clone for RecordingStats {
    fn clone(&self) -> Self {
        Self {
            chunks_processed: self.chunks_processed,
            total_duration: self.total_duration,
            last_activity: self.last_activity,
        }
    }
}

#[cfg(test)]
mod mute_gate_tests {
    use super::*;

    fn chunk(device_type: DeviceType) -> AudioChunk {
        AudioChunk {
            data: vec![0.1; 4],
            sample_rate: 48_000,
            timestamp: 0.0,
            chunk_id: 0,
            device_type,
        }
    }

    #[test]
    fn muted_drops_microphone_chunks_but_keeps_system() {
        // specs/0049: while the process-wide mute gate is set, owner-mic chunks are
        // dropped so the transcript is system-only; system/remote audio is unaffected;
        // clearing the gate restores the mic. Uses the global gate, so set/reset it
        // within this test only.
        let state = RecordingState::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AudioChunk>(16);
        state.set_audio_sender(tx);

        crate::audio::mute_gate::set_muted(true);
        state
            .send_audio_chunk(chunk(DeviceType::Microphone))
            .unwrap();
        assert!(
            rx.try_recv().is_err(),
            "a muted microphone chunk must be dropped"
        );

        state.send_audio_chunk(chunk(DeviceType::System)).unwrap();
        assert_eq!(
            rx.try_recv().unwrap().device_type,
            DeviceType::System,
            "system audio keeps flowing while muted"
        );

        crate::audio::mute_gate::set_muted(false);
        state
            .send_audio_chunk(chunk(DeviceType::Microphone))
            .unwrap();
        assert_eq!(
            rx.try_recv().unwrap().device_type,
            DeviceType::Microphone,
            "clearing the gate restores the microphone"
        );
    }
}

