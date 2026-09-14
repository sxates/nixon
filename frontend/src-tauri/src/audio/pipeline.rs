use super::batch_processor::AudioMetricsBatcher;
use crate::batch_audio_metric;
use anyhow::Result;
use log::{debug, error, info, warn};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::audio_processing::{
    audio_to_mono, HighPassFilter, LoudnessNormalizer, NoiseSuppressionProcessor,
};
use super::devices::AudioDevice;
use super::recording_state::{AudioChunk, AudioError, DeviceType, RecordingState};
use super::stt_stage::{SttStage, SyncOutcome};
use super::vad::SpeechSegment;

// specs/0029 WS3.4: re-export the channel-attribution types alongside the pipeline
// that produces them (`common` is a crate-private module; integration tests and the
// rest of the crate reach these through here).
pub use super::common::{ChannelRun, ChannelTag, TranscriptionChunk};
// specs/0029 WS3.4 / 0055 — capture-channel attribution lives in its own module
// (file-size ratchet, specs/0042); re-exported here because `owner_turns.rs` and
// `retranscription_channels.rs` reach for it through this path.
pub(crate) use super::channel_attribution::{
    channel_runs_for_span, classify_window_channel, dominant_channel_for_span,
    CHANNEL_WINDOW_HISTORY_MAX,
};

/// specs/0029 WS4.2: type-erased, process-lifetime emitter for the best-effort live
/// audio UI events (`recording-level`, `recording-spectrum`). The pipeline can't hold a
/// `tauri::AppHandle` itself (it is constructed below the Tauri layer and runs
/// handle-less in the integration tests), so whoever owns an AppHandle registers a thin
/// emit closure here. Registered at app setup in `lib.rs` (specs/0029 WS7.2 — it used
/// to be registered from the transcription worker's startup, but record-only mode
/// doesn't spawn that worker and the spectrometer must keep working without it).
type LiveEventEmitter = Box<dyn Fn(&str, serde_json::Value) + Send + Sync>;
static LIVE_EVENT_EMITTER: std::sync::OnceLock<LiveEventEmitter> = std::sync::OnceLock::new();

/// Register the process-lifetime live-event emitter. First registration wins; later
/// calls are no-ops (the first AppHandle stays valid for the whole process, so this is
/// registered once at app setup and reused thereafter).
pub fn register_live_event_emitter<F>(emit: F)
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let _ = LIVE_EVENT_EMITTER.set(Box::new(emit));
}

fn live_event_emitter() -> Option<&'static LiveEventEmitter> {
    LIVE_EVENT_EMITTER.get()
}

/// Ring buffer for synchronized audio mixing
/// Accumulates samples from mic and system streams until we have aligned windows
struct AudioMixerRingBuffer {
    mic_buffer: VecDeque<f32>,
    system_buffer: VecDeque<f32>,
    window_size_samples: usize, // Fixed mixing window (600 ms — see `new`)
    max_buffer_size: usize,     // Safety limit (8× window ≈ 4800 ms)
    // specs/0028: per-instance diagnostic counter (replaces a `static mut`, which is UB).
    sample_log_counter: u64,
}

impl AudioMixerRingBuffer {
    fn new(sample_rate: u32) -> Self {
        // 600 ms mixing window. Large windows tolerate the substantial jitter of Core Audio
        // system capture (sample-by-sample streaming → batching → channel transmission).
        let window_ms = 600.0;
        let window_size_samples = (sample_rate as f32 * window_ms / 1000.0) as usize;

        // Safety cap at 8× the window (~4800 ms) so a stalled/jittery channel can't grow the
        // mix buffers without bound; oldest samples are dropped past this limit.
        // Accounts for: RNNoise buffering + Core Audio jitter + processing delays.
        let max_buffer_size = window_size_samples * 8; // ~4800 ms

        info!(
            "🔊 Ring buffer initialized: window={}ms ({} samples), max={}ms ({} samples)",
            window_ms,
            window_size_samples,
            window_ms * 8.0,
            max_buffer_size
        );

        Self {
            mic_buffer: VecDeque::with_capacity(max_buffer_size),
            system_buffer: VecDeque::with_capacity(max_buffer_size),
            window_size_samples,
            max_buffer_size,
            sample_log_counter: 0,
        }
    }

    fn add_samples(&mut self, device_type: DeviceType, samples: Vec<f32>) {
        // Log buffer health periodically for diagnostics (specs/0028: was a `static mut`).
        self.sample_log_counter = self.sample_log_counter.wrapping_add(1);
        if self.sample_log_counter.is_multiple_of(200) {
            debug!(
                "📊 Ring buffer status: mic={} samples, sys={} samples (max={})",
                self.mic_buffer.len(),
                self.system_buffer.len(),
                self.max_buffer_size
            );
        }

        match device_type {
            DeviceType::Microphone => self.mic_buffer.extend(samples),
            DeviceType::System => self.system_buffer.extend(samples),
        }

        // CRITICAL FIX: Add warnings before dropping samples
        // This helps diagnose timing issues in production
        if self.mic_buffer.len() > self.max_buffer_size {
            warn!(
                "⚠️ Microphone buffer overflow: {} > {} samples, dropping oldest {} samples",
                self.mic_buffer.len(),
                self.max_buffer_size,
                self.mic_buffer.len() - self.max_buffer_size
            );
        }
        if self.system_buffer.len() > self.max_buffer_size {
            error!("🔴 SYSTEM AUDIO BUFFER OVERFLOW: {} > {} samples, dropping {} samples - THIS CAUSES DISTORTION!",
                  self.system_buffer.len(), self.max_buffer_size,
                  self.system_buffer.len() - self.max_buffer_size);
        }

        // Safety: prevent buffer overflow (keep only the last ~4800 ms / max_buffer_size)
        while self.mic_buffer.len() > self.max_buffer_size {
            self.mic_buffer.pop_front();
        }
        while self.system_buffer.len() > self.max_buffer_size {
            self.system_buffer.pop_front();
        }
    }

    fn can_mix(&self) -> bool {
        // TODO(0028): timestamp-based alignment. Today the mic and system windows are aligned
        // by BUFFER LENGTH only — `add_samples` discards each chunk's `timestamp`, so if one
        // stream starts late or drops a chunk, the two windows can drift out of temporal sync
        // (mic word N mixed against system word N±k). A correct fix threads the per-chunk
        // timestamps into the ring buffer and drops/zero-pads to a shared time base before
        // extracting a window. Deferred here (MEDIUM, audio-quality-sensitive, needs a
        // real-audio A/B soak test) to avoid regressing the mix on the hot path.
        self.mic_buffer.len() >= self.window_size_samples
            || self.system_buffer.len() >= self.window_size_samples
    }

    fn extract_window(&mut self) -> Option<(Vec<f32>, Vec<f32>)> {
        if !self.can_mix() {
            return None;
        }

        // Extract mic window with zero-padding for incomplete buffers
        // Zero-padding (silence) is preferred over last-sample-hold to prevent artifacts

        // Extract mic window (or pad with zeros if insufficient data)
        let mic_window = if self.mic_buffer.len() >= self.window_size_samples {
            // Enough mic data - drain window
            self.mic_buffer.drain(0..self.window_size_samples).collect()
        } else if !self.mic_buffer.is_empty() {
            // Some mic data but not enough - consume all + pad with zeros
            let available: Vec<f32> = self.mic_buffer.drain(..).collect();
            let mut padded = Vec::with_capacity(self.window_size_samples);
            padded.extend_from_slice(&available);

            // Use zero-padding (silence) to prevent repetition artifacts
            // Zero-padding is inaudible at 48kHz sample rate
            padded.resize(self.window_size_samples, 0.0);

            padded
        } else {
            // No mic data - return silence
            vec![0.0; self.window_size_samples]
        };

        // Extract system window (or pad with zeros if insufficient data)
        let sys_window = if self.system_buffer.len() >= self.window_size_samples {
            // Enough system data - drain window
            self.system_buffer
                .drain(0..self.window_size_samples)
                .collect()
        } else if !self.system_buffer.is_empty() {
            // Some system data but not enough - consume all + pad with zeros
            let available: Vec<f32> = self.system_buffer.drain(..).collect();
            let mut padded = Vec::with_capacity(self.window_size_samples);
            padded.extend_from_slice(&available);

            // Use zero-padding (silence) to prevent repetition artifacts
            // Zero-padding is inaudible at 48kHz sample rate
            padded.resize(self.window_size_samples, 0.0);

            padded
        } else {
            // No system data - return silence
            vec![0.0; self.window_size_samples]
        };

        Some((mic_window, sys_window))
    }
}

/// Simple audio mixer without aggressive ducking
/// Combines mic + system audio with basic clipping prevention
struct ProfessionalAudioMixer;

impl ProfessionalAudioMixer {
    fn new(_sample_rate: u32) -> Self {
        Self
    }

    fn mix_window(&mut self, mic_window: &[f32], sys_window: &[f32]) -> Vec<f32> {
        // Handle different lengths (already padded by extract_window, but defensive)
        let max_len = mic_window.len().max(sys_window.len());
        let mut mixed = Vec::with_capacity(max_len);

        // Professional mixing with soft scaling to prevent distortion
        // Uses proportional scaling instead of hard clamping to avoid artifacts
        for i in 0..max_len {
            let mic = mic_window.get(i).copied().unwrap_or(0.0);
            let sys = sys_window.get(i).copied().unwrap_or(0.0);

            // Pre-scale system audio to 70% to leave headroom
            // This prevents constant soft scaling which can cause pumping artifacts
            // Mic is normalized to -23 LUFS (already optimal), system needs reduction
            let sys_scaled = sys * 1.0;
            let _mic_scaled = mic * 0.8; // Reserved for future mic scaling

            // Sum without ducking - mic stays at full volume, system slightly reduced
            let sum = mic + sys_scaled;

            // CRITICAL FIX: Soft scaling prevents distortion artifacts
            // If the sum would exceed ±1.0, scale down PROPORTIONALLY
            // This avoids hard clipping distortion that sounds like "radio breaks"
            let sum_abs = sum.abs();
            let mixed_sample = if sum_abs > 1.0 {
                // Scale down to fit within ±1.0
                sum / sum_abs
            } else {
                sum
            };

            mixed.push(mixed_sample);
        }

        mixed
    }
}

/// Simplified audio capture without broadcast channels
#[derive(Clone)]
pub struct AudioCapture {
    device: Arc<AudioDevice>,
    state: Arc<RecordingState>,
    sample_rate: u32, // Original device sample rate
    channels: u16,
    chunk_counter: Arc<std::sync::atomic::AtomicU64>,
    device_type: DeviceType,
    // Retained (not read): holds a clone of the capture→recording sender so the
    // channel stays open for as long as this capture lives; see specs/0028.
    #[allow(dead_code)]
    recording_sender: Option<mpsc::UnboundedSender<AudioChunk>>,
    needs_resampling: bool, // Flag if resampling is required
    // CRITICAL FIX: Persistent resampler to preserve energy across chunks
    resampler: Arc<std::sync::Mutex<Option<SincFixedIn<f32>>>>,
    // Buffering for variable-size chunks → fixed-size resampler input
    resampler_input_buffer: Arc<std::sync::Mutex<Vec<f32>>>,
    resampler_chunk_size: usize, // Fixed chunk size for resampler (512 samples)
    // Audio enhancement processors (microphone only)
    noise_suppressor: Arc<std::sync::Mutex<Option<NoiseSuppressionProcessor>>>,
    high_pass_filter: Arc<std::sync::Mutex<Option<HighPassFilter>>>,
    // EBU R128 normalizer for microphone audio (per-device, stateful)
    normalizer: Arc<std::sync::Mutex<Option<LoudnessNormalizer>>>,
    // Note: Using global recording timestamp for synchronization
}

impl AudioCapture {
    pub fn new(
        device: Arc<AudioDevice>,
        state: Arc<RecordingState>,
        sample_rate: u32,
        channels: u16,
        device_type: DeviceType,
        recording_sender: Option<mpsc::UnboundedSender<AudioChunk>>,
    ) -> Self {
        // CRITICAL FIX: Detect if resampling is needed
        // Pipeline expects 48kHz, but Bluetooth devices often report 8kHz, 16kHz, or 44.1kHz
        const TARGET_SAMPLE_RATE: u32 = 48000;
        let needs_resampling = sample_rate != TARGET_SAMPLE_RATE;

        // Detect device kind (Bluetooth vs Wired) for adaptive processing
        // Use reasonable defaults for buffer size (512 samples is typical)
        let device_kind =
            super::device_detection::InputDeviceKind::detect(&device.name, 512, sample_rate);

        if needs_resampling {
            warn!("⚠️ SAMPLE RATE MISMATCH DETECTED ⚠️");
            warn!(
                "🔄 [{:?}] Audio device '{}' ({:?}) reports {} Hz (pipeline expects {} Hz)",
                device_type, device.name, device_kind, sample_rate, TARGET_SAMPLE_RATE
            );
            warn!(
                "🔄 Automatic resampling will be applied: {} Hz → {} Hz",
                sample_rate, TARGET_SAMPLE_RATE
            );

            // Log which resampling strategy will be used
            let ratio = TARGET_SAMPLE_RATE as f64 / sample_rate as f64;
            let strategy = if ratio >= 2.0 {
                "High-quality upsampling (sinc_len=512, Cubic interpolation)"
            } else if ratio >= 1.5 {
                "Moderate upsampling (sinc_len=384, Cubic)"
            } else if ratio > 1.0 {
                "Small upsampling (sinc_len=256, Linear)"
            } else if ratio <= 0.5 {
                "Anti-aliased downsampling (sinc_len=512, Cubic)"
            } else {
                "Moderate downsampling (sinc_len=384, Linear)"
            };
            info!("   Resampling strategy: {}", strategy);
        } else {
            info!(
                "✅ [{:?}] Audio device '{}' ({:?}) uses {} Hz (matches pipeline)",
                device_type, device.name, device_kind, sample_rate
            );
        }

        // Initialize audio enhancement processors for MICROPHONE ONLY
        // System audio doesn't need enhancement (already clean)
        let (noise_suppressor, high_pass_filter, normalizer) = if matches!(
            device_type,
            DeviceType::Microphone
        ) {
            // Initialize noise suppression (RNNoise) at 48kHz - CONDITIONAL based on flag
            let ns = if super::ffmpeg_mixer::RNNOISE_APPLY_ENABLED {
                match NoiseSuppressionProcessor::new(TARGET_SAMPLE_RATE) {
                    Ok(processor) => {
                        info!("✅ RNNoise noise suppression ENABLED for microphone '{}' (10-15 dB reduction)", device.name);
                        Some(processor)
                    }
                    Err(e) => {
                        warn!("⚠️ Failed to create noise suppressor: {}, continuing without noise suppression", e);
                        None
                    }
                }
            } else {
                info!("ℹ️ RNNoise noise suppression DISABLED for microphone '{}' (flag: RNNOISE_APPLY_ENABLED=false)", device.name);
                info!("   Whisper handles noise well internally - RNNoise is optional");
                None
            };

            // Initialize high-pass filter (removes rumble below 80 Hz)
            let hpf = {
                let filter = HighPassFilter::new(TARGET_SAMPLE_RATE, 80.0);
                info!(
                    "✅ High-pass filter initialized for microphone '{}' (cutoff: 80 Hz)",
                    device.name
                );
                Some(filter)
            };

            // Initialize EBU R128 normalizer (professional loudness standard)
            let norm = match LoudnessNormalizer::new(1, TARGET_SAMPLE_RATE) {
                Ok(normalizer) => {
                    info!(
                        "✅ EBU R128 normalizer initialized for microphone '{}' (target: -23 LUFS)",
                        device.name
                    );
                    Some(normalizer)
                }
                Err(e) => {
                    warn!(
                        "⚠️ Failed to create normalizer for microphone: {}, normalization disabled",
                        e
                    );
                    None
                }
            };

            (ns, hpf, norm)
        } else {
            // System audio: no enhancement needed
            info!(
                "ℹ️ System audio '{}' captured raw (no enhancement)",
                device.name
            );
            (None, None, None)
        };

        // CRITICAL FIX: Initialize persistent resampler to preserve energy across chunks
        // Creating a new resampler per chunk causes energy amplification and incorrect output sizes
        // Use fixed chunk size of 512 samples with buffering for variable-size input
        const RESAMPLER_CHUNK_SIZE: usize = 512;

        let resampler = if needs_resampling {
            let ratio = TARGET_SAMPLE_RATE as f64 / sample_rate as f64;

            // Adaptive parameters based on sample rate ratio (same logic as resample_audio)
            let (sinc_len, interpolation_type, oversampling) = if ratio >= 2.0 {
                (512, SincInterpolationType::Cubic, 512)
            } else if ratio >= 1.5 {
                (384, SincInterpolationType::Cubic, 384)
            } else if ratio > 1.0 {
                (256, SincInterpolationType::Linear, 256)
            } else if ratio <= 0.5 {
                (512, SincInterpolationType::Cubic, 512)
            } else {
                (384, SincInterpolationType::Linear, 384)
            };

            let params = SincInterpolationParameters {
                sinc_len,
                f_cutoff: 0.95,
                interpolation: interpolation_type,
                oversampling_factor: oversampling,
                window: WindowFunction::BlackmanHarris2,
            };

            match SincFixedIn::<f32>::new(
                ratio,
                2.0, // Maximum relative deviation
                params,
                RESAMPLER_CHUNK_SIZE,
                1, // Mono
            ) {
                Ok(resampler) => {
                    info!(
                        "✅ Persistent resampler initialized for '{}' ({}Hz → {}Hz, chunk_size={})",
                        device.name, sample_rate, TARGET_SAMPLE_RATE, RESAMPLER_CHUNK_SIZE
                    );
                    info!("   Buffering enabled for variable-size chunks (e.g., 320, 512, 1024, etc.)");
                    Some(resampler)
                }
                Err(e) => {
                    warn!(
                        "⚠️ Failed to create persistent resampler: {}, will use fallback",
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        Self {
            device,
            state,
            sample_rate,
            channels,
            chunk_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            device_type,
            recording_sender,
            needs_resampling,
            resampler: Arc::new(std::sync::Mutex::new(resampler)),
            resampler_input_buffer: Arc::new(std::sync::Mutex::new(Vec::with_capacity(
                RESAMPLER_CHUNK_SIZE * 2,
            ))),
            resampler_chunk_size: RESAMPLER_CHUNK_SIZE,
            noise_suppressor: Arc::new(std::sync::Mutex::new(noise_suppressor)),
            high_pass_filter: Arc::new(std::sync::Mutex::new(high_pass_filter)),
            normalizer: Arc::new(std::sync::Mutex::new(normalizer)),
            // Using global recording time for sync
        }
    }

    /// Process audio data directly from callback
    pub fn process_audio_data(&self, data: &[f32]) {
        // Check if still recording
        if !self.state.is_recording() {
            return;
        }

        // Convert to mono if needed
        let mut mono_data = if self.channels > 1 {
            audio_to_mono(data, self.channels)
        } else {
            data.to_vec()
        };

        // CRITICAL FIX: Resample to 48kHz if device uses different sample rate
        // This fixes Bluetooth devices (like Sony WH-1000XM4) that report 16kHz or 44.1kHz
        // Without this, audio is sped up 3x and VAD fails
        //
        // IMPORTANT: Uses PERSISTENT resampler with BUFFERING to preserve energy across chunks
        // Creating a new resampler per chunk causes energy amplification (173.5% RMS)
        // Buffering handles variable chunk sizes (320, 512, 1024, etc.) by accumulating to fixed 512-sample chunks
        const TARGET_SAMPLE_RATE: u32 = 48000;
        if self.needs_resampling {
            let before_len = mono_data.len();
            let before_rms = if !mono_data.is_empty() {
                (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt()
            } else {
                0.0
            };

            // Use persistent resampler with buffering to handle variable chunk sizes
            let mut resampled_output = Vec::new();
            let mut used_persistent_resampler = false;

            if let Ok(mut buffer_lock) = self.resampler_input_buffer.lock() {
                // Add new samples to buffer
                buffer_lock.extend_from_slice(&mono_data);

                // Process complete chunks through the resampler
                if let Ok(mut resampler_lock) = self.resampler.lock() {
                    if let Some(ref mut resampler) = *resampler_lock {
                        used_persistent_resampler = true;

                        // Process as many complete chunks as we have
                        while buffer_lock.len() >= self.resampler_chunk_size {
                            // Extract exactly chunk_size samples
                            let chunk: Vec<f32> =
                                buffer_lock.drain(0..self.resampler_chunk_size).collect();

                            // Rubato expects input as Vec<Vec<f32>> (one Vec per channel)
                            let waves_in = vec![chunk];

                            match resampler.process(&waves_in, None) {
                                Ok(mut waves_out) => {
                                    if let Some(output) = waves_out.pop() {
                                        resampled_output.extend_from_slice(&output);
                                    }
                                }
                                Err(e) => {
                                    warn!("⚠️ Persistent resampler processing failed: {}", e);
                                    used_persistent_resampler = false;
                                    break;
                                }
                            }
                        }
                        // Remaining samples in buffer will be processed in next iteration
                    }
                }
            }

            // CRITICAL: Only update mono_data if we got output from persistent resampler
            // If buffer is accumulating (< 512 samples), skip this chunk - data is safely buffered
            // and will be processed in next iteration with proper resampling
            let has_resampled_output = !resampled_output.is_empty();

            if has_resampled_output {
                mono_data = resampled_output;
            } else if !used_persistent_resampler {
                // Only fallback if persistent resampler is not available at all
                mono_data = super::audio_processing::resample_audio(
                    &mono_data,
                    self.sample_rate,
                    TARGET_SAMPLE_RATE,
                );
            } else {
                // Buffering: samples are accumulating in buffer, waiting for 512-sample chunk
                // Don't send partial/unprocessed data - return early
                // Audio is NOT lost - it's in the buffer and will be processed next iteration
                return;
            }

            // Log resampling only occasionally to avoid spam
            let chunk_id = self.chunk_counter.load(std::sync::atomic::Ordering::SeqCst);
            if chunk_id.is_multiple_of(100) && has_resampled_output {
                let after_len = mono_data.len();
                let after_rms = if !mono_data.is_empty() {
                    (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt()
                } else {
                    0.0
                };
                let ratio = TARGET_SAMPLE_RATE as f64 / self.sample_rate as f64;
                let rms_preservation = if before_rms > 0.0 {
                    (after_rms / before_rms) * 100.0
                } else {
                    100.0
                };

                let buffer_size = if let Ok(buf) = self.resampler_input_buffer.lock() {
                    buf.len()
                } else {
                    0
                };

                info!(
                    "🔄 [{:?}] Persistent buffered resampler: {}Hz → {}Hz (ratio: {:.2}x)",
                    self.device_type, self.sample_rate, TARGET_SAMPLE_RATE, ratio
                );
                info!(
                    "   Chunk {}: {} → {} samples, RMS preservation: {:.1}%, buffer: {}",
                    chunk_id, before_len, after_len, rms_preservation, buffer_size
                );
            }
        }

        // AUDIO ENHANCEMENT PIPELINE (Microphone Only)
        // Processing order is critical: high-pass → noise suppression → normalization
        // This ensures noise is removed before being amplified by the normalizer
        if matches!(self.device_type, DeviceType::Microphone) {
            // STEP 1: Apply high-pass filter to remove low-frequency rumble (< 80 Hz)
            if let Ok(mut hpf_lock) = self.high_pass_filter.lock() {
                if let Some(ref mut filter) = *hpf_lock {
                    mono_data = filter.process(&mono_data);
                }
            }

            // STEP 2: Apply RNNoise noise suppression (10-15 dB reduction) - CONDITIONAL
            if super::ffmpeg_mixer::RNNOISE_APPLY_ENABLED {
                if let Ok(mut ns_lock) = self.noise_suppressor.lock() {
                    if let Some(ref mut suppressor) = *ns_lock {
                        let before_len = mono_data.len();
                        mono_data = suppressor.process(&mono_data);
                        let after_len = mono_data.len();

                        // CRITICAL MONITORING: Track buffer health
                        let chunk_id = self.chunk_counter.load(std::sync::atomic::Ordering::SeqCst);
                        if chunk_id.is_multiple_of(100) {
                            let buffered = suppressor.buffered_samples();
                            let length_delta = (before_len as i32 - after_len as i32).abs();

                            debug!("🔇 Noise suppression health: in={}, out={}, delta={}, buffered={}, RMS={:.4}",
                                   before_len, after_len, length_delta, buffered,
                                   if !mono_data.is_empty() {
                                       (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt()
                                   } else { 0.0 });

                            // WARN if accumulating samples (potential latency buildup)
                            if buffered > 1000 {
                                warn!("⚠️ RNNoise accumulating samples: {} buffered (potential latency issue!)",
                                      buffered);
                            }

                            // WARN if significant length mismatch
                            if length_delta > 50 {
                                warn!(
                                    "⚠️ RNNoise length mismatch: input={} output={} (delta={})",
                                    before_len, after_len, length_delta
                                );
                            }
                        }
                    }
                }
            }

            // STEP 3: Apply EBU R128 normalization (professional loudness standard)
            if let Ok(mut normalizer_lock) = self.normalizer.lock() {
                if let Some(ref mut normalizer) = *normalizer_lock {
                    mono_data = normalizer.normalize_loudness(&mono_data);

                    // Log normalization occasionally for debugging
                    let chunk_id = self.chunk_counter.load(std::sync::atomic::Ordering::SeqCst);
                    if chunk_id.is_multiple_of(200) && !mono_data.is_empty() {
                        let rms = (mono_data.iter().map(|&x| x * x).sum::<f32>()
                            / mono_data.len() as f32)
                            .sqrt();
                        let peak = mono_data.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
                        debug!(
                            "🎤 After normalization chunk {}: RMS={:.4}, Peak={:.4}",
                            chunk_id, rms, peak
                        );
                    }
                }
            }
        }

        // Create audio chunk with stream-specific timestamp (get ID first for logging)
        let chunk_id = self
            .chunk_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // RAW AUDIO: No gain applied here - will be applied AFTER mixing
        // This prevents amplifying system audio bleed-through in the microphone

        // DIAGNOSTIC: Log audio levels for debugging (especially mic issues)
        // if chunk_id % 100 == 0 && !mono_data.is_empty() {
        //     let raw_rms = (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt();
        //     let raw_peak = mono_data.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);

        //         info!("🎙️ [{:?}] Chunk {} - Raw: RMS={:.6}, Peak={:.6}",
        //               self.device_type, chunk_id, raw_rms, raw_peak);

        //     // Warn if microphone is completely silent
        //     if matches!(self.device_type, DeviceType::Microphone) && raw_rms == 0.0 && raw_peak == 0.0 {
        //         warn!("⚠️ Microphone producing ZERO audio - check permissions or hardware!");
        //     }
        // }
        // else if chunk_id % 100 == 0 && matches!(self.device_type, DeviceType::System) {
        //     let raw_rms = (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt();
        //     let raw_peak = mono_data.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
        //     info!("🔊 [{:?}] Chunk {} - Raw: RMS={:.6}, Peak={:.6}",
        //       self.device_type, chunk_id, raw_rms, raw_peak);

        //     // Warn if system audio is completely silent
        //     if raw_rms == 0.0 && raw_peak == 0.0 {
        //         warn!("⚠️ System audio producing ZERO audio - check permissions or hardware!");
        //     }
        // }

        // Use global recording timestamp for proper synchronization
        let timestamp = self.state.get_recording_duration().unwrap_or(0.0);

        // RAW AUDIO CHUNK: No gain applied - will be mixed and gained downstream
        // Use 48kHz if we resampled, otherwise use original rate
        let audio_chunk = AudioChunk {
            data: mono_data, // Raw audio (resampled if needed), no gain yet
            sample_rate: if self.needs_resampling {
                48000
            } else {
                self.sample_rate
            },
            timestamp,
            chunk_id,
            device_type: self.device_type.clone(),
        };

        // NOTE: Raw audio is NOT sent to recording saver to prevent echo
        // Only the mixed audio (from AudioPipeline) is saved to file (see pipeline.rs:726-736)
        // This ensures we only record once: mic + system properly mixed
        // Individual raw streams go only to the transcription pipeline below

        // Send to processing pipeline for transcription
        if let Err(e) = self.state.send_audio_chunk(audio_chunk) {
            // Check if this is the "pipeline not ready" error
            if e.to_string().contains("Audio pipeline not ready") {
                // This is expected during initialization, just log it as debug
                debug!("Audio pipeline not ready yet, skipping chunk {}", chunk_id);
                return;
            }

            warn!("Failed to send audio chunk: {}", e);
            // More specific error handling based on failure reason
            let error = if e.to_string().contains("channel closed") {
                AudioError::ChannelClosed
            } else if e.to_string().contains("full") {
                AudioError::BufferOverflow
            } else {
                AudioError::ProcessingFailed
            };
            self.state.report_error(error);
        } else {
            debug!("Sent audio chunk {} ({} samples)", chunk_id, data.len());
        }
    }

    /// Handle stream errors with enhanced disconnect detection
    pub fn handle_stream_error(&self, error: cpal::StreamError) {
        error!("Audio stream error for {}: {}", self.device.name, error);

        let error_str = error.to_string().to_lowercase();

        // Enhanced error detection for device disconnection
        let audio_error = if error_str.contains("device is no longer available")
            || error_str.contains("device not found")
            || error_str.contains("device disconnected")
            || error_str.contains("no such device")
            || error_str.contains("device unavailable")
            || error_str.contains("device removed")
        {
            warn!("🔌 Device disconnect detected for: {}", self.device.name);
            AudioError::DeviceDisconnected
        } else if error_str.contains("permission") || error_str.contains("access denied") {
            AudioError::PermissionDenied
        } else if error_str.contains("channel closed") {
            AudioError::ChannelClosed
        } else if error_str.contains("stream") && error_str.contains("failed") {
            AudioError::StreamFailed
        } else {
            warn!("Unknown audio error: {}", error);
            AudioError::StreamFailed
        };

        self.state.report_error(audio_error);
    }
}

/// VAD-driven audio processing pipeline
/// Uses Voice Activity Detection to segment speech in real-time and send only speech to Whisper
pub struct AudioPipeline {
    // specs/0028: BOUNDED capture→pipeline and pipeline→transcription channels (were
    // unbounded). On saturation both apply an explicit drop-newest policy (see the
    // send/recv sites) so a slow consumer can't blow up memory or hang stop.
    receiver: mpsc::Receiver<AudioChunk>,
    transcription_sender: mpsc::Sender<TranscriptionChunk>,
    // Retained (not read): keeps an Arc clone of the shared recording state alive
    // for the lifetime of the pipeline.
    #[allow(dead_code)]
    state: Arc<RecordingState>,
    // specs/0029 WS7.2 + low-power-mode spec §3: the VAD/STT gating stage. Owns the
    // optional VAD and the session's shared live-STT flag; `sync()` attaches/detaches
    // it mid-recording. When detached (record-only mode) the whole VAD → 16 kHz
    // resample → transcription-send stage is skipped — that work exists solely to feed
    // live STT — so the CPU cost drops. Mixing, recording/save, the per-channel
    // diarization WAVs, and the live level/spectrum emits are unaffected.
    stt_stage: SttStage,
    sample_rate: u32,
    chunk_id_counter: u64,
    // specs/0028: count of transcription segments shed when the bounded transcription
    // queue is saturated (for a throttled warning). The authoritative user-facing
    // `transcription-falling-behind` event is emitted worker-side (which holds the AppHandle).
    transcription_dropped: u64,
    // Performance optimization: reduce logging frequency
    last_summary_time: std::time::Instant,
    processed_chunks: u64,
    // Smart batching for audio metrics
    metrics_batcher: Option<AudioMetricsBatcher>,
    // PROFESSIONAL AUDIO MIXING: Ring buffer + RMS-based mixer
    ring_buffer: AudioMixerRingBuffer,
    mixer: ProfessionalAudioMixer,
    // Recording sender for pre-mixed audio
    recording_sender_for_mixed: Option<mpsc::UnboundedSender<AudioChunk>>,
    // specs/0010 P1-B1: optional per-channel WAV capture for offline diarization.
    // The meeting folder where `system.wav` / `mic.wav` are written. `None` disables
    // the feature (e.g. auto-save off → no folder). Set by the pipeline manager.
    diarization_meeting_folder: Option<PathBuf>,
    // Lazily-created per-channel writers. `Some(writer)` once successfully opened;
    // `None` means "not yet started" or "failed → permanently disabled" (gated by
    // `diarization_channels_failed` so we don't retry on every window).
    mic_channel_writer: Option<super::channel_writer::ChannelWavWriter>,
    system_channel_writer: Option<super::channel_writer::ChannelWavWriter>,
    diarization_channels_failed: bool,
    // specs/0011 P3-B: optional live-diarization fan-out. When `Some`, each pre-mix
    // SYSTEM window (48 kHz) is also pushed to the live diarizer's growing buffer
    // (best-effort; mic windows are never forwarded — mic = `You`). `None` (the
    // default / live diarization disabled) makes this path inert / zero-cost.
    live_diarizer: Option<std::sync::Arc<crate::diarization::live::LiveDiarizer>>,
    // specs/0029 WS4.2 / 0057 §3.2: live level/spectrum feed for the recording meters,
    // driven by the RAW pre-mix + mixed windows (NOT the VAD-gated transcription path,
    // whose starvation froze the old spectrometer). See `audio::live_meter`.
    live_meter: super::live_meter::LiveMeterStage,
    // specs/0029 WS3.4: per-window channel-dominance history. Each entry is one
    // classified (non-silent) 600 ms mix window: (start_ms, end_ms, class) in the
    // same time base as the VAD segment timestamps (the session audio clock,
    // `recorded_ms`). Bounded by `CHANNEL_WINDOW_HISTORY_MAX`.
    channel_windows: VecDeque<(f64, f64, ChannelTag)>,
    // spec 0051 WS1: the session's audio clock (ms). Advances for EVERY mixed window,
    // including windows processed while live transcription is off, so it stays aligned
    // with the saved WAV across a deferred span. `SttStage` re-bases its segment offset
    // onto this on each attach. A paused recording feeds no chunks at all
    // (`RecordingState::send_audio_chunk` discards them while paused), so this stalls
    // with the WAV and needs no pause gate of its own.
    recorded_ms: f64,
}

impl AudioPipeline {
    #[allow(clippy::too_many_arguments)] // pipeline wiring; cohesive construction params
    pub fn new(
        receiver: mpsc::Receiver<AudioChunk>,
        transcription_sender: mpsc::Sender<TranscriptionChunk>,
        state: Arc<RecordingState>,
        target_chunk_duration_ms: u32,
        sample_rate: u32,
        mic_device_name: String,
        mic_device_kind: super::device_detection::InputDeviceKind,
        system_device_name: String,
        system_device_kind: super::device_detection::InputDeviceKind,
        // low-power-mode spec §3: the session's shared live-STT flag. Starts false in
        // record-only mode (the VAD is never constructed; recording/save and the live
        // level/spectrum emits keep working) and can be flipped mid-recording by the
        // toggle command — the `SttStage` attaches/detaches its VAD on the next window.
        live_stt: Arc<AtomicBool>,
    ) -> Result<Self> {
        // Log device characteristics for adaptive buffering
        info!("🎛️ AudioPipeline initializing with device characteristics:");
        info!(
            "   Mic: '{}' ({:?}) - Buffer: {:?}",
            mic_device_name,
            mic_device_kind,
            mic_device_kind.buffer_timeout()
        );
        info!(
            "   System: '{}' ({:?}) - Buffer: {:?}",
            system_device_name,
            system_device_kind,
            system_device_kind.buffer_timeout()
        );

        // Device kind information can be used for adaptive buffering in the future
        // For now, we log it for monitoring and potential optimization
        let _ = (
            mic_device_name,
            mic_device_kind,
            system_device_name,
            system_device_kind,
        );

        // low-power-mode spec §3: the VAD/STT gating stage. Constructs the VAD up-front
        // when the flag starts true (a start-time init failure still aborts the recording
        // start, as before — specs/0028); skips it in record-only mode. Mid-recording
        // attach/detach happens in `SttStage::sync()`, driven by the shared flag.
        let stt_stage = SttStage::new(sample_rate, live_stt)?;

        // Initialize professional audio mixing components
        let ring_buffer = AudioMixerRingBuffer::new(sample_rate);
        let mixer = ProfessionalAudioMixer::new(sample_rate);

        // Note: target_chunk_duration_ms is ignored - VAD controls segmentation now
        let _ = target_chunk_duration_ms;

        Ok(Self {
            receiver,
            transcription_sender,
            state,
            stt_stage,
            sample_rate,
            chunk_id_counter: 0,
            transcription_dropped: 0,
            // Performance optimization: reduce logging frequency
            last_summary_time: std::time::Instant::now(),
            processed_chunks: 0,
            // Initialize metrics batcher for smart batching
            metrics_batcher: Some(AudioMetricsBatcher::new()),
            // Initialize professional audio mixing
            ring_buffer,
            mixer,
            recording_sender_for_mixed: None, // Will be set by manager
            diarization_meeting_folder: None, // Will be set by manager (specs/0010)
            mic_channel_writer: None,
            system_channel_writer: None,
            diarization_channels_failed: false,
            live_diarizer: None, // Set by the manager when live diarization is enabled (specs/0011).
            live_meter: super::live_meter::LiveMeterStage::new(sample_rate),
            // specs/0029 WS3.4: channel-attribution state.
            channel_windows: VecDeque::new(),
            recorded_ms: 0.0,
        })
    }

    /// Attach (or detach) the sink for the pre-mixed recording audio. Public so
    /// the manager AND the hardware-less integration tests can wire a collector
    /// (specs/0029 WS7.2: record-only mode must still deliver mixed audio here).
    pub fn set_recording_sender(&mut self, sender: Option<mpsc::UnboundedSender<AudioChunk>>) {
        self.recording_sender_for_mixed = sender;
    }

    /// specs/0029 WS3.4: classify one mix window's channel dominance and record its
    /// span. spec 0051 WS1: the caller owns the clock now (it must advance whether or
    /// not the STT stage is attached), so the span bounds are passed in.
    fn record_channel_window(
        &mut self,
        mic_window: &[f32],
        sys_window: &[f32],
        start_ms: f64,
        window_ms: f64,
    ) {
        if let Some(class) = classify_window_channel(mic_window, sys_window) {
            self.channel_windows
                .push_back((start_ms, start_ms + window_ms, class));
            while self.channel_windows.len() > CHANNEL_WINDOW_HISTORY_MAX {
                self.channel_windows.pop_front();
            }
        }
    }

    /// Send a batch of VAD segments to the transcriber. spec 0051 WS1: one
    /// implementation shared by the main mix loop, the mid-recording detach flush, and
    /// the stop-time flush — all three previously carried their own copy of this block.
    fn dispatch_segments(&mut self, segments: Vec<SpeechSegment>) {
        for segment in segments {
            let duration_ms = segment.end_timestamp_ms - segment.start_timestamp_ms;

            // Minimum 50ms at 16kHz - matches Parakeet capability
            let samples = segment.samples.len();
            if samples < 800 {
                debug!("⏭️ Dropping short VAD segment: {duration_ms:.1}ms ({samples} < 800)");
                continue;
            }

            // specs/0029 WS3.4: aggregate the dominant capture channel across the
            // windows this segment spans.
            let channel = dominant_channel_for_span(
                &self.channel_windows,
                segment.start_timestamp_ms,
                segment.end_timestamp_ms,
            );
            info!("📤 Sending VAD segment: {duration_ms:.1}ms, {samples} samples, {channel:?}");

            // specs/0055: the same windows at window resolution, so a row that
            // straddles a handoff can be split instead of taking one label.
            let channel_runs = channel_runs_for_span(
                &self.channel_windows,
                segment.start_timestamp_ms,
                segment.end_timestamp_ms,
            );

            let transcription_chunk = TranscriptionChunk {
                chunk: AudioChunk {
                    data: segment.samples,
                    sample_rate: 16000,
                    timestamp: segment.start_timestamp_ms / 1000.0,
                    chunk_id: self.chunk_id_counter,
                    device_type: DeviceType::Microphone, // Mixed audio
                },
                channel,
                channel_runs,
            };

            // specs/0028: BOUNDED, non-blocking send with an explicit drop-newest
            // policy on saturation so a slow transcriber can't grow this queue without
            // bound. Under normal load there is free space and nothing is dropped.
            match self.transcription_sender.try_send(transcription_chunk) {
                Ok(()) => {
                    self.chunk_id_counter += 1;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    self.transcription_dropped += 1;
                    if self.transcription_dropped == 1
                        || self.transcription_dropped.is_multiple_of(50)
                    {
                        warn!("⚠️ Transcription queue saturated; dropped {} segment(s) (falling behind)", self.transcription_dropped);
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    warn!("Transcription channel closed; stopping segment dispatch");
                }
            }
        }
    }

    /// specs/0029 WS4.2 / 0057 §3.2: pace the live meter feed (throttled inside the
    /// stage; no-op when nothing is staged or no emitter is registered).
    fn maybe_emit_live_meter(&mut self) {
        if let Some(emit) = live_event_emitter() {
            self.live_meter.maybe_emit(emit.as_ref());
        }
    }

    /// Lazily open the per-channel WAV writers on the first window. Best-effort:
    /// any failure logs and permanently disables the feature for this recording so
    /// it never competes with or breaks the primary mixed recording / transcription.
    fn ensure_channel_writers(&mut self) {
        if self.diarization_channels_failed
            || self.mic_channel_writer.is_some()
            || self.system_channel_writer.is_some()
        {
            return;
        }
        let Some(folder) = self.diarization_meeting_folder.clone() else {
            return;
        };

        let sys_path = super::channel_writer::system_channel_wav(&folder);
        let mic_path = super::channel_writer::mic_channel_wav(&folder);

        match (
            super::channel_writer::ChannelWavWriter::new(sys_path),
            super::channel_writer::ChannelWavWriter::new(mic_path),
        ) {
            (Ok(sys_w), Ok(mic_w)) => {
                info!(
                    "🎚️ Per-channel diarization capture enabled → {} / {}",
                    super::channel_writer::SYSTEM_CHANNEL_FILENAME,
                    super::channel_writer::MIC_CHANNEL_FILENAME
                );
                self.system_channel_writer = Some(sys_w);
                self.mic_channel_writer = Some(mic_w);
            }
            (sys_res, mic_res) => {
                if let Err(e) = sys_res {
                    warn!("⚠️ Could not open system.wav for diarization: {} (continuing without per-channel capture)", e);
                }
                if let Err(e) = mic_res {
                    warn!("⚠️ Could not open mic.wav for diarization: {} (continuing without per-channel capture)", e);
                }
                self.system_channel_writer = None;
                self.mic_channel_writer = None;
                self.diarization_channels_failed = true;
            }
        }
    }

    /// Write one pre-mix (mic, system) window to the per-channel WAVs. Best-effort:
    /// on the first write error the feature is disabled for the rest of the recording.
    fn write_channel_windows(&mut self, mic_window: &[f32], sys_window: &[f32]) {
        // specs/0011 P3-B: fan out the same pre-mix SYSTEM window (48 kHz) to the live
        // diarizer, independent of the per-channel WAV capture above. Best-effort and
        // gated by `Some`: when live diarization is off this is inert. Mic windows are
        // NOT forwarded (mic = `You`). `feed_48k` itself never panics / never errors
        // out — it logs-and-disables on failure — so this can't break recording/STT.
        if let Some(live) = self.live_diarizer.as_ref() {
            live.feed_48k(sys_window);
        }

        if self.diarization_meeting_folder.is_none() || self.diarization_channels_failed {
            return;
        }
        self.ensure_channel_writers();

        let mut failed = false;
        if let Some(writer) = self.system_channel_writer.as_mut() {
            if let Err(e) = writer.write_window(sys_window) {
                warn!(
                    "⚠️ system.wav write failed: {} (disabling per-channel capture)",
                    e
                );
                failed = true;
            }
        }
        if let Some(writer) = self.mic_channel_writer.as_mut() {
            if let Err(e) = writer.write_window(mic_window) {
                warn!(
                    "⚠️ mic.wav write failed: {} (disabling per-channel capture)",
                    e
                );
                failed = true;
            }
        }
        if failed {
            self.system_channel_writer = None;
            self.mic_channel_writer = None;
            self.diarization_channels_failed = true;
        }
    }

    /// Finalize per-channel WAVs (flush + patch headers). Best-effort; called at
    /// pipeline shutdown.
    fn finalize_channel_writers(&mut self) {
        if let Some(writer) = self.system_channel_writer.take() {
            if let Err(e) = writer.finalize() {
                warn!("⚠️ Failed to finalize system.wav: {}", e);
            }
        }
        if let Some(writer) = self.mic_channel_writer.take() {
            if let Err(e) = writer.finalize() {
                warn!("⚠️ Failed to finalize mic.wav: {}", e);
            }
        }
    }

    /// Run the VAD-driven audio processing pipeline
    pub async fn run(mut self) -> Result<()> {
        info!("VAD-driven audio pipeline started - segments sent in real-time based on speech detection");

        // CRITICAL FIX: Continue processing until channel is closed, not based on recording state
        // This ensures ALL chunks are processed during shutdown, fixing premature meeting completion
        // Previous bug: Loop checked `while self.state.is_recording()` which caused early exit when
        // stop_recording() was called, losing flush signals and remaining chunks in the pipeline
        loop {
            // Receive audio chunks with timeout
            match tokio::time::timeout(
                std::time::Duration::from_millis(50), // Shorter timeout for responsiveness
                self.receiver.recv(),
            )
            .await
            {
                Ok(Some(chunk)) => {
                    // PERFORMANCE: Check for flush signal (special chunk with ID >= u64::MAX - 10)
                    // Multiple flush signals may be sent to ensure processing
                    if chunk.chunk_id >= u64::MAX - 10 {
                        info!(
                            "📥 Received FLUSH signal #{} - flushing VAD processor",
                            u64::MAX - chunk.chunk_id
                        );
                        self.flush_remaining_audio()?;
                        // Continue processing to handle any remaining chunks
                        continue;
                    }

                    // PERFORMANCE OPTIMIZATION: Eliminate per-chunk logging overhead
                    // Logging in hot paths causes severe performance degradation
                    self.processed_chunks += 1;

                    // Smart batching: collect metrics instead of logging every chunk
                    if let Some(ref batcher) = self.metrics_batcher {
                        let avg_level = chunk.data.iter().map(|&x| x.abs()).sum::<f32>()
                            / chunk.data.len() as f32;
                        let duration_ms =
                            chunk.data.len() as f64 / chunk.sample_rate as f64 * 1000.0;

                        batch_audio_metric!(
                            Some(batcher),
                            chunk.chunk_id,
                            chunk.data.len(),
                            duration_ms,
                            avg_level
                        );
                    }

                    // CRITICAL: Log summary only every 200 chunks OR every 60 seconds (99.5% reduction)
                    // This eliminates I/O overhead in the audio processing hot path
                    // Use performance-optimized debug macro that compiles to nothing in release builds
                    if self.processed_chunks.is_multiple_of(200)
                        || self.last_summary_time.elapsed().as_secs() >= 60
                    {
                        perf_debug!(
                            "Pipeline processed {} chunks, current chunk: {} ({} samples)",
                            self.processed_chunks,
                            chunk.chunk_id,
                            chunk.data.len()
                        );
                        self.last_summary_time = std::time::Instant::now();
                    }

                    // STEP 1: Add raw audio to ring buffer for mixing
                    // Microphone audio is already normalized at capture level (AudioCapture)
                    // System audio remains raw
                    self.ring_buffer
                        .add_samples(chunk.device_type.clone(), chunk.data);

                    // STEP 2: Mix audio in fixed windows when both streams have sufficient data
                    while self.ring_buffer.can_mix() {
                        if let Some((mic_window, sys_window)) = self.ring_buffer.extract_window() {
                            // specs/0010 P1-B1: tap the CLEAN, SEPARATED channels BEFORE the
                            // RMS-duck mix so offline diarization gets unmixed audio. Additive
                            // and best-effort — never affects the mix/transcription below.
                            self.write_channel_windows(&mic_window, &sys_window);

                            // Simple mixing without aggressive ducking
                            let mixed_clean = self.mixer.mix_window(&mic_window, &sys_window);

                            // NO POST-GAIN NEEDED: Microphone already normalized by EBU R128 to -23 LUFS
                            // This is broadcast-standard loudness (Netflix/YouTube/Spotify level)
                            // System audio at natural levels
                            // Previous 2x gain was causing excessive limiting/distortion
                            let mixed_with_gain = mixed_clean;

                            // specs/0029 WS4.2 / 0057 §3.2: stage the RAW windows (clean mic,
                            // clean system, mixed) for the live meters BEFORE VAD — the
                            // needles must move whenever audio flows, independent of speech
                            // gating and transcription backpressure. Emission is paced below
                            // via maybe_emit_live_meter. Headless (no emitter): don't buffer.
                            if live_event_emitter().is_some() {
                                self.live_meter
                                    .stage(&mic_window, &sys_window, &mixed_with_gain);
                            }

                            // spec 0051 WS1: advance the session audio clock for EVERY
                            // mixed window — including deferred ones — then reconcile
                            // the VAD/STT stage against it, BEFORE the channel-window
                            // check and process(), so a mid-recording attach starts on
                            // the right timeline the same window it comes online.
                            let window_ms =
                                mixed_with_gain.len() as f64 / self.sample_rate as f64 * 1000.0;
                            let window_start_ms = self.recorded_ms;
                            self.recorded_ms += window_ms;

                            match self.stt_stage.sync(window_start_ms) {
                                SyncOutcome::Attached => {
                                    // Spans recorded before the gap are in the same time
                                    // base, but nothing between them and the resumed
                                    // segments was classified — drop them so a resumed
                                    // segment can't inherit pre-gap channel dominance.
                                    self.channel_windows.clear();
                                }
                                SyncOutcome::Detached(tail) => {
                                    // Dispatch BEFORE any clear: the tail resolves its
                                    // channel against the spans it actually spanned.
                                    self.dispatch_segments(tail);
                                }
                                SyncOutcome::Unchanged => {}
                            }

                            // specs/0029 WS3.4: classify this window's channel dominance
                            // from the CLEAN pre-mix tracks (the mix destroys channel
                            // identity). specs/0029 WS7.2: the tag exists solely to ride
                            // the transcription chunks, so it is skipped with the rest of
                            // the STT stage in record-only mode.
                            if self.stt_stage.is_active() {
                                self.record_channel_window(
                                    &mic_window,
                                    &sys_window,
                                    window_start_ms,
                                    window_ms,
                                );
                            }

                            // STEP 3: Send mixed audio for transcription (VAD + Whisper).
                            // specs/0029 WS7.2: `None` in record-only mode — the entire
                            // stage (VAD, resample, channel aggregation, sends) is skipped.
                            match self.stt_stage.process(&mixed_with_gain) {
                                None => {}
                                Some(Ok(speech_segments)) => {
                                    self.dispatch_segments(speech_segments)
                                }
                                Some(Err(e)) => {
                                    warn!("⚠️ VAD error: {}", e);
                                }
                            }

                            // STEP 4: Send mixed audio for recording (WAV file)
                            if let Some(ref sender) = self.recording_sender_for_mixed {
                                let recording_chunk = AudioChunk {
                                    data: mixed_with_gain.clone(),
                                    sample_rate: self.sample_rate,
                                    timestamp: chunk.timestamp,
                                    chunk_id: self.chunk_id_counter,
                                    device_type: DeviceType::Microphone, // Mixed audio
                                };
                                let _ = sender.send(recording_chunk);
                            }
                        }
                    }

                    // specs/0029 WS4.2: pace the live level/spectrum feed (throttled
                    // internally; no-op when nothing is staged).
                    self.maybe_emit_live_meter();
                }
                Ok(None) => {
                    info!(
                        "Audio pipeline: sender closed after processing {} chunks",
                        self.processed_chunks
                    );
                    break;
                }
                Err(_) => {
                    // Timeout - VAD handles all segmentation; keep draining the staged
                    // live-spectrum window so the bars stay honest during capture gaps
                    // (specs/0029 WS4.2).
                    self.maybe_emit_live_meter();
                    continue;
                }
            }
        }

        // Flush any remaining VAD segments. Capture (don't `?`-propagate) the result so
        // we ALWAYS finalize the per-channel WAVs below even if the flush errors —
        // otherwise the headers stay at the placeholder (data-size 0) and symphonia
        // can't decode them. (Drop on ChannelWavWriter is the belt; this is the
        // suspenders for the normal path.)
        let flush_result = self.flush_remaining_audio();

        // specs/0010 P1-B1: finalize per-channel WAVs (flush tail + patch headers).
        self.finalize_channel_writers();

        flush_result?;

        info!("VAD-driven audio pipeline ended");
        Ok(())
    }

    fn flush_remaining_audio(&mut self) -> Result<()> {
        info!(
            "Flushing remaining audio from pipeline (processed {} chunks)",
            self.processed_chunks
        );

        // specs/0029 WS7.2: record-only mode has no VAD/STT stage — nothing to flush.
        let Some(flush_result) = self.stt_stage.flush() else {
            return Ok(());
        };

        match flush_result {
            Ok(final_segments) => {
                info!("📤 Flushing {} final VAD segment(s)", final_segments.len());
                // spec 0051 final review (Finding 5): `dispatch_segments` shares the main
                // loop's THROTTLED saturation warning, so a dropped FINAL segment could
                // log nothing — permanently lost words, silently, on a plain live meeting.
                let dropped_before = self.transcription_dropped;
                self.dispatch_segments(final_segments);
                let lost = self.transcription_dropped - dropped_before;
                if lost > 0 {
                    warn!("⚠️ Dropped {lost} FINAL VAD segment(s) at stop (transcription queue saturated) — those words are permanently lost");
                }
            }
            Err(e) => {
                warn!("⚠️ VAD flush error at stop: {}", e);
            }
        }
        Ok(())
    }
}

/// Bounded capacity of the capture→pipeline channel (specs/0028). Was unbounded. Generously
/// sized because the pipeline (mix + VAD) is a fast consumer that should essentially never
/// back up under normal load; this only caps memory in a catastrophic stall.
const CAPTURE_QUEUE_CAPACITY: usize = 2048;

/// Simple audio pipeline manager
pub struct AudioPipelineManager {
    pipeline_handle: Option<JoinHandle<Result<()>>>,
    // specs/0028: BOUNDED capture→pipeline channel (was unbounded).
    audio_sender: Option<mpsc::Sender<AudioChunk>>,
}

impl AudioPipelineManager {
    pub fn new() -> Self {
        Self {
            pipeline_handle: None,
            audio_sender: None,
        }
    }

    /// Start the audio pipeline with device information for adaptive buffering
    #[allow(clippy::too_many_arguments)] // pipeline wiring; cohesive start params
    pub fn start(
        &mut self,
        state: Arc<RecordingState>,
        transcription_sender: mpsc::Sender<TranscriptionChunk>,
        target_chunk_duration_ms: u32,
        sample_rate: u32,
        recording_sender: Option<mpsc::UnboundedSender<AudioChunk>>,
        mic_device_name: String,
        mic_device_kind: super::device_detection::InputDeviceKind,
        system_device_name: String,
        system_device_kind: super::device_detection::InputDeviceKind,
        // specs/0010 P1-B1: meeting folder for per-channel diarization WAVs. `None`
        // (e.g. auto-save disabled / no folder) disables per-channel capture.
        diarization_meeting_folder: Option<PathBuf>,
        // specs/0011 P3-B: live diarizer to fan the pre-mix SYSTEM window into. `None`
        // (live diarization disabled — the default) makes the tap inert / zero-cost.
        // The next stage (IPC) constructs this from settings + the meeting id and
        // passes it here via `recording_manager`.
        live_diarizer: Option<std::sync::Arc<crate::diarization::live::LiveDiarizer>>,
        // low-power-mode spec §3: the session's shared live-STT flag. Starts false in
        // record-only mode (no VAD/STT stage; audio is still mixed, recorded, and drives
        // the live level/spectrum feed) and can be flipped mid-recording by the toggle
        // command.
        live_stt: Arc<AtomicBool>,
    ) -> Result<()> {
        // Log device information for adaptive buffering
        info!("🎙️ Starting pipeline with device info:");
        info!(
            "   Microphone: '{}' ({:?})",
            mic_device_name, mic_device_kind
        );
        info!(
            "   System Audio: '{}' ({:?})",
            system_device_name, system_device_kind
        );

        // Create audio processing channel.
        // specs/0028: BOUNDED (was unbounded). Capture callbacks push via `try_send` with an
        // explicit drop policy (see RecordingState::send_audio_chunk), bounding memory if the
        // pipeline consumer ever stalls. Generously sized — this fast-consumer channel should
        // essentially never drop under normal load.
        let (audio_sender, audio_receiver) = mpsc::channel::<AudioChunk>(CAPTURE_QUEUE_CAPACITY);

        // Set sender in state for audio captures to use
        state.set_audio_sender(audio_sender.clone());

        // Create and start pipeline with device information for adaptive mixing.
        // specs/0028: VAD init can now fail — propagate instead of panicking on the hot path.
        let mut pipeline = AudioPipeline::new(
            audio_receiver,
            transcription_sender,
            state.clone(),
            target_chunk_duration_ms,
            sample_rate,
            mic_device_name,
            mic_device_kind,
            system_device_name,
            system_device_kind,
            live_stt,
        )?;

        // CRITICAL FIX: Connect recording sender to receive pre-mixed audio
        // This ensures both mic AND system audio are captured in recordings
        pipeline.set_recording_sender(recording_sender);

        // specs/0010 P1-B1: enable per-channel WAV capture for offline diarization
        // when a meeting folder is available (additive — does not affect the mix).
        if let Some(ref folder) = diarization_meeting_folder {
            info!(
                "📂 Per-channel diarization capture target: {}",
                folder.display()
            );
        }
        pipeline.diarization_meeting_folder = diarization_meeting_folder;

        // specs/0011 P3-B: attach the live diarizer (if any) so the system window
        // fans out to it. Inert when `None`.
        if live_diarizer.is_some() {
            info!("🗣️ Live diarization fan-out enabled for this recording");
        }
        pipeline.live_diarizer = live_diarizer;

        let handle = tokio::spawn(async move { pipeline.run().await });

        self.pipeline_handle = Some(handle);
        self.audio_sender = Some(audio_sender);

        info!("Audio pipeline manager started with mixed audio recording");
        Ok(())
    }

    /// Stop the audio pipeline
    pub async fn stop(&mut self) -> Result<()> {
        // Drop the sender to close the pipeline
        self.audio_sender = None;

        // Wait for pipeline to finish
        if let Some(handle) = self.pipeline_handle.take() {
            match handle.await {
                Ok(result) => result,
                Err(e) => {
                    error!("Pipeline task failed: {}", e);
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }

    /// Force immediate flush of accumulated audio and stop pipeline
    /// PERFORMANCE CRITICAL: Eliminates 30+ second shutdown delays
    pub async fn force_flush_and_stop(&mut self) -> Result<()> {
        info!("🚀 Force flushing pipeline - processing ALL accumulated audio immediately");

        // If we have a sender, send a special flush signal first
        if let Some(sender) = &self.audio_sender {
            // Create a special flush chunk to trigger immediate processing
            let flush_chunk = AudioChunk {
                data: vec![], // Empty data signals flush
                sample_rate: 16000,
                timestamp: 0.0,
                chunk_id: u64::MAX, // Special ID to indicate flush
                device_type: super::recording_state::DeviceType::Microphone,
            };

            // specs/0028: the channel is now BOUNDED. Await the primary flush signal so it is
            // guaranteed delivered even if the queue is momentarily full (this is the fast-stop
            // path — we must not silently lose the flush trigger). Redundant signals below stay
            // best-effort via try_send.
            if let Err(e) = sender.send(flush_chunk).await {
                warn!("Failed to send flush signal: {}", e);
            } else {
                info!("📤 Sent flush signal to pipeline");

                // PERFORMANCE OPTIMIZATION: Reduced wait time from 50ms to 20ms
                // Pipeline should process flush signal very quickly
                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

                // Send multiple flush signals to ensure the pipeline catches it
                // This aggressive approach eliminates shutdown delay issues
                for i in 0..3 {
                    let additional_flush = AudioChunk {
                        data: vec![],
                        sample_rate: 16000,
                        timestamp: 0.0,
                        chunk_id: u64::MAX - (i as u64),
                        device_type: super::recording_state::DeviceType::Microphone,
                    };
                    let _ = sender.try_send(additional_flush);
                }

                info!("📤 Sent additional flush signals for reliability");
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        }

        // Now stop normally
        self.stop().await
    }
}

impl Default for AudioPipelineManager {
    fn default() -> Self {
        Self::new()
    }
}
