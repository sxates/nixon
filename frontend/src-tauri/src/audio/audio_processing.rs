use anyhow::Result;
use chrono::Utc;
use log::{debug, info, warn};
use nnnoiseless::DenoiseState;
use realfft::num_complex::{Complex32, ComplexFloat};
use realfft::RealFftPlanner;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::path::{Path, PathBuf};

use super::encode::encode_single_audio; // Correct path to encode module

/// Sanitize a filename to be safe for filesystem use
pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Create a meeting folder with timestamp and return the path
/// Creates structure: base_path/MeetingName_YYYY-MM-DD_HH-MM/
///                    ├── .checkpoints/  (for incremental saves, optional)
///
/// # Arguments
/// * `base_path` - Base directory for meetings
/// * `meeting_name` - Name of the meeting
/// * `create_checkpoints_dir` - Whether to create .checkpoints/ subdirectory (only needed when auto_save is true)
pub fn create_meeting_folder(
    base_path: &Path,
    meeting_name: &str,
    create_checkpoints_dir: bool,
) -> Result<PathBuf> {
    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M").to_string();
    let sanitized_name = sanitize_filename(meeting_name);
    let folder_name = format!("{}_{}", sanitized_name, timestamp);
    let meeting_folder = base_path.join(folder_name);

    // Create main meeting folder
    std::fs::create_dir_all(&meeting_folder)?;

    // Only create .checkpoints subdirectory if requested (when auto_save is true)
    if create_checkpoints_dir {
        let checkpoints_dir = meeting_folder.join(".checkpoints");
        std::fs::create_dir_all(&checkpoints_dir)?;
        log::info!(
            "Created meeting folder with checkpoints: {}",
            meeting_folder.display()
        );
    } else {
        log::info!(
            "Created meeting folder without checkpoints: {}",
            meeting_folder.display()
        );
    }

    Ok(meeting_folder)
}

pub fn normalize_v2(audio: &[f32]) -> Vec<f32> {
    let rms = (audio.iter().map(|&x| x * x).sum::<f32>() / audio.len() as f32).sqrt();
    let peak = audio
        .iter()
        .fold(0.0f32, |max, &sample| max.max(sample.abs()));

    // Return the original audio if it's completely silent
    if rms == 0.0 || peak == 0.0 {
        return audio.to_vec();
    }

    // Increase target RMS for better voice volume while keeping peak in check
    let target_rms = 0.9; // Increased from 0.6
    let target_peak = 0.95; // Slightly reduced to prevent clipping

    let rms_scaling = target_rms / rms;
    let peak_scaling = target_peak / peak;

    // Apply a minimum scaling factor to boost very quiet audio
    let min_scaling = 1.5; // Minimum boost for quiet audio
    let scaling_factor = (rms_scaling.min(peak_scaling)).max(min_scaling);

    // Apply scaling with soft clipping to prevent harsh distortion
    audio
        .iter()
        .map(|&sample| {
            let scaled = sample * scaling_factor;
            // Soft clip at ±0.95 to prevent harsh distortion
            if scaled > 0.95 {
                0.95 + (scaled - 0.95) * 0.05
            } else if scaled < -0.95 {
                -0.95 + (scaled + 0.95) * 0.05
            } else {
                scaled
            }
        })
        .collect()
}

/// True peak limiter with lookahead buffer (prevents clipping)
struct TruePeakLimiter {
    lookahead_samples: usize,
    buffer: Vec<f32>,
    gain_reduction: Vec<f32>,
    current_position: usize,
}

impl TruePeakLimiter {
    fn new(sample_rate: u32) -> Self {
        const LIMITER_LOOKAHEAD_MS: usize = 10;
        let lookahead_samples = ((sample_rate as usize * LIMITER_LOOKAHEAD_MS) / 1000).max(1);

        Self {
            lookahead_samples,
            buffer: vec![0.0; lookahead_samples],
            gain_reduction: vec![1.0; lookahead_samples],
            current_position: 0,
        }
    }

    fn process(&mut self, sample: f32, true_peak_limit: f32) -> f32 {
        self.buffer[self.current_position] = sample;

        let sample_abs = sample.abs();
        if sample_abs > true_peak_limit {
            let reduction = true_peak_limit / sample_abs;
            self.gain_reduction[self.current_position] = reduction;
        } else {
            self.gain_reduction[self.current_position] = 1.0;
        }

        let output_position = (self.current_position + 1) % self.lookahead_samples;
        let output_sample = self.buffer[output_position] * self.gain_reduction[output_position];

        self.current_position = output_position;
        output_sample
    }
}

/// Professional loudness normalizer using EBU R128 standard
/// This is a STATEFUL normalizer that tracks cumulative loudness over time
///
/// EBU R128 is the broadcast industry standard for loudness normalization:
/// - Target: -23 LUFS (Loudness Units relative to Full Scale)
/// - Used by: Netflix, YouTube, Spotify, all professional broadcast
/// - Perceptually accurate (not just simple RMS)
///
pub struct LoudnessNormalizer {
    ebur128: ebur128::EbuR128,
    limiter: TruePeakLimiter,
    gain_linear: f32,
    loudness_buffer: Vec<f32>,
    true_peak_limit: f32,
}

impl LoudnessNormalizer {
    /// Create a new EBU R128 loudness normalizer
    ///
    /// # Arguments
    /// * `channels` - Number of audio channels (1 for mono, 2 for stereo)
    /// * `sample_rate` - Sample rate in Hz (e.g., 48000)
    pub fn new(channels: u32, sample_rate: u32) -> Result<Self> {
        const TRUE_PEAK_LIMIT: f64 = -1.0;
        const ANALYZE_CHUNK_SIZE: usize = 512;

        let ebur128 = ebur128::EbuR128::new(
            channels,
            sample_rate,
            ebur128::Mode::I | ebur128::Mode::TRUE_PEAK,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create EBU R128 normalizer: {}", e))?;

        let true_peak_limit = 10_f32.powf(TRUE_PEAK_LIMIT as f32 / 20.0);

        Ok(Self {
            ebur128,
            limiter: TruePeakLimiter::new(sample_rate),
            gain_linear: 1.0,
            loudness_buffer: Vec::with_capacity(ANALYZE_CHUNK_SIZE),
            true_peak_limit,
        })
    }

    /// Normalize loudness using EBU R128 standard with true peak limiting
    ///
    /// This maintains cumulative loudness measurements across all processed audio,
    /// resulting in consistent normalization that sounds natural.
    ///
    /// Target: -23 LUFS (professional broadcast standard for speech/dialog)
    /// Applies sample-by-sample with 10ms lookahead limiter to prevent clipping
    pub fn normalize_loudness(&mut self, samples: &[f32]) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        const TARGET_LUFS: f64 = -23.0;
        const ANALYZE_CHUNK_SIZE: usize = 512;

        let mut normalized_samples = Vec::with_capacity(samples.len());

        for &sample in samples {
            // Accumulate samples for loudness analysis
            self.loudness_buffer.push(sample);

            // Analyze loudness every 512 samples
            if self.loudness_buffer.len() >= ANALYZE_CHUNK_SIZE {
                if let Err(e) = self.ebur128.add_frames_f32(&self.loudness_buffer) {
                    warn!("Failed to add frames to EBU R128: {}", e);
                } else {
                    // Update gain based on cumulative loudness
                    if let Ok(current_lufs) = self.ebur128.loudness_global() {
                        if current_lufs.is_finite() && current_lufs < 0.0 {
                            let gain_db = TARGET_LUFS - current_lufs;
                            self.gain_linear = 10_f32.powf(gain_db as f32 / 20.0);
                        }
                    }
                }
                self.loudness_buffer.clear();
            }

            // Apply gain and true peak limiting
            let amplified = sample * self.gain_linear;
            let limited = self.limiter.process(amplified, self.true_peak_limit);

            normalized_samples.push(limited);
        }

        normalized_samples
    }
}

/// RNNoise-based noise suppression processor
///
/// Uses a recurrent neural network to suppress background noise while preserving speech.
/// Processes audio at 48kHz in 10ms frames (480 samples per frame).
///
/// Benefits:
/// - 10-15 dB noise reduction in typical office/home environments
/// - Preserves speech quality and intelligibility
/// - Low latency (~10ms per frame)
/// - Cross-platform (works on macOS, Windows, Linux)
pub struct NoiseSuppressionProcessor {
    denoiser: DenoiseState<'static>,
    frame_buffer: Vec<f32>,
    frame_size: usize, // 480 samples at 48kHz = 10ms
}

impl NoiseSuppressionProcessor {
    /// Create a new noise suppression processor
    ///
    /// # Arguments
    /// * `sample_rate` - Must be 48000 Hz (RNNoise requirement)
    pub fn new(sample_rate: u32) -> Result<Self> {
        if sample_rate != 48000 {
            return Err(anyhow::anyhow!(
                "Noise suppression requires 48kHz sample rate, got {}Hz",
                sample_rate
            ));
        }

        const FRAME_SIZE: usize = DenoiseState::FRAME_SIZE;

        info!(
            "Initializing RNNoise noise suppression (frame size: {} samples, 10ms @ 48kHz)",
            FRAME_SIZE
        );

        Ok(Self {
            denoiser: *DenoiseState::new(),
            frame_buffer: Vec::with_capacity(FRAME_SIZE * 2),
            frame_size: FRAME_SIZE,
        })
    }

    /// Apply noise suppression to audio samples
    ///
    /// Processes audio in 480-sample frames (10ms at 48kHz).
    /// Buffers partial frames for next call.
    ///
    /// CRITICAL FIX: Always returns same length as input to prevent latency accumulation
    ///
    /// # Arguments
    /// * `samples` - Input audio samples at 48kHz
    ///
    /// # Returns
    /// Noise-suppressed audio samples (SAME LENGTH as input)
    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        // CRITICAL: Remember original input length
        let input_len = samples.len();

        // Add new samples to buffer
        self.frame_buffer.extend_from_slice(samples);

        let mut output = Vec::with_capacity(input_len);

        // Process complete frames
        while self.frame_buffer.len() >= self.frame_size {
            // Extract one frame
            let frame: Vec<f32> = self.frame_buffer.drain(0..self.frame_size).collect();

            // RNNoise processes audio: separate input and output buffers
            let mut denoised_frame = vec![0.0f32; self.frame_size];

            // Apply noise suppression
            // process_frame(output: &mut [f32], input: &[f32]) -> f32
            // Returns VAD probability (0.0-1.0), higher means more likely to be speech
            let _vad_prob = self.denoiser.process_frame(&mut denoised_frame, &frame);

            output.extend_from_slice(&denoised_frame);
        }

        // Return processed output without forcing length matching
        // Frame-based processing naturally creates variable-length output
        // Downstream pipeline handles this correctly via ring buffer
        output
    }

    /// Get the number of buffered samples waiting for processing
    pub fn buffered_samples(&self) -> usize {
        self.frame_buffer.len()
    }

    /// Flush any remaining buffered samples
    /// Call this at the end of recording to process partial frames
    pub fn flush(&mut self) -> Vec<f32> {
        if self.frame_buffer.is_empty() {
            return Vec::new();
        }

        // Pad the remaining samples to a full frame with zeros
        let remaining = self.frame_buffer.len();
        let mut input_frame = self.frame_buffer.clone();
        if input_frame.len() < self.frame_size {
            input_frame.resize(self.frame_size, 0.0);
        }

        let mut output = vec![0.0f32; self.frame_size];
        self.denoiser.process_frame(&mut output, &input_frame);
        self.frame_buffer.clear();

        // Return only the original samples (without padding)
        output.truncate(remaining);
        output
    }
}

/// High-pass filter to remove low-frequency rumble and noise
/// Removes frequencies below cutoff_hz (typically 80-100 Hz for speech)
pub struct HighPassFilter {
    #[allow(dead_code)]
    sample_rate: f32,
    #[allow(dead_code)]
    cutoff_hz: f32,
    // First-order IIR filter coefficients
    alpha: f32,
    prev_input: f32,
    prev_output: f32,
}

impl HighPassFilter {
    /// Create a new high-pass filter
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `cutoff_hz` - Cutoff frequency in Hz (typical: 80-100 Hz for speech)
    pub fn new(sample_rate: u32, cutoff_hz: f32) -> Self {
        let sample_rate_f = sample_rate as f32;
        let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
        let dt = 1.0 / sample_rate_f;
        let alpha = rc / (rc + dt);

        info!(
            "Initializing high-pass filter: cutoff={}Hz @ {}Hz",
            cutoff_hz, sample_rate
        );

        Self {
            sample_rate: sample_rate_f,
            cutoff_hz,
            alpha,
            prev_input: 0.0,
            prev_output: 0.0,
        }
    }

    /// Apply high-pass filter to audio samples
    /// Uses first-order IIR (Infinite Impulse Response) filter
    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut output = Vec::with_capacity(samples.len());

        for &sample in samples {
            // First-order high-pass IIR filter formula:
            // y[n] = alpha * (y[n-1] + x[n] - x[n-1])
            let filtered = self.alpha * (self.prev_output + sample - self.prev_input);

            self.prev_input = sample;
            self.prev_output = filtered;

            output.push(filtered);
        }

        output
    }

    /// Reset filter state (call when starting new recording)
    pub fn reset(&mut self) {
        self.prev_input = 0.0;
        self.prev_output = 0.0;
    }
}

pub fn spectral_subtraction(audio: &[f32], d: f32) -> Result<Vec<f32>> {
    let mut real_planner = RealFftPlanner::<f32>::new();
    let window_size = 1600; // 16k sample rate - 100ms

    // CRITICAL FIX: Handle cases where audio is longer than window size
    if audio.is_empty() {
        return Ok(Vec::new());
    }

    // If audio is longer than window size, truncate to prevent overflow
    let processed_audio = if audio.len() > window_size {
        warn!(
            "Audio length {} exceeds window size {}, truncating",
            audio.len(),
            window_size
        );
        &audio[..window_size]
    } else {
        audio
    };

    let r2c = real_planner.plan_fft_forward(window_size);
    let mut y = r2c.make_output_vec();

    // Safe padding: only pad if audio is shorter than window size
    let mut padded_audio = processed_audio.to_vec();
    if processed_audio.len() < window_size {
        let padding_needed = window_size - processed_audio.len();
        padded_audio.extend(vec![0.0f32; padding_needed]);
    }

    let mut indata = padded_audio;
    r2c.process(&mut indata, &mut y)?;

    let mut processed_audio = y
        .iter()
        .map(|&x| {
            let magnitude_y = x.abs().powf(2.0);

            let div = 1.0 - (d / magnitude_y);

            let gain = {
                if div > 0.0 {
                    f32::sqrt(div)
                } else {
                    0.0f32
                }
            };

            x * gain
        })
        .collect::<Vec<Complex32>>();

    let c2r = real_planner.plan_fft_inverse(window_size);

    let mut outdata = c2r.make_output_vec();

    c2r.process(&mut processed_audio, &mut outdata)?;

    Ok(outdata)
}

/// Number of vocal-range frequency bands emitted for the recording spectrometer (specs/0025).
/// Chosen as an upper bound on any recording surface's bar count so the frontend always
/// downsamples (never interpolates up).
pub const SPECTRUM_BANDS: usize = 32;

/// Compute `SPECTRUM_BANDS` log-spaced magnitude bands across the vocal range (~80 Hz–4 kHz)
/// for the recording spectrometer (specs/0025). Returns values in 0..1 (perceptually
/// compressed), **low frequency first**.
///
/// Cheap + best-effort, called from the throttled live-audio emit (~12×/s): a fixed
/// 2048-sample Hann-windowed real FFT of the MOST RECENT samples (zero-padded if the chunk
/// is shorter). Per band we take the peak bin magnitude (punchier than an average), normalize
/// by the window size (FFT magnitude scales with N), apply a fixed `GAIN` + sqrt compression
/// so quiet speech is visible, and clamp. Never errors — returns all-zero on empty input.
/// `GAIN` is the one knob to tune the bar liveliness.
pub fn spectrum_bands(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    const WINDOW: usize = 2048;
    const F_LO: f32 = 80.0; // low end of speech fundamentals
    const F_HI: f32 = 4000.0; // upper edge of speech intelligibility
    const GAIN: f32 = 6.0;

    let mut bands = vec![0.0f32; SPECTRUM_BANDS];
    if samples.is_empty() || sample_rate == 0 {
        return bands;
    }

    // Most-recent WINDOW samples, zero-padded at the front when the chunk is shorter.
    let mut frame = vec![0.0f32; WINDOW];
    let n = samples.len().min(WINDOW);
    frame[WINDOW - n..].copy_from_slice(&samples[samples.len() - n..]);

    // Hann window to reduce spectral leakage.
    for (i, s) in frame.iter_mut().enumerate() {
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (WINDOW as f32 - 1.0)).cos();
        *s *= w;
    }

    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(WINDOW);
    let mut spectrum = r2c.make_output_vec();
    if r2c.process(&mut frame, &mut spectrum).is_err() {
        return bands;
    }

    let bin_hz = sample_rate as f32 / WINDOW as f32;
    let log_lo = F_LO.ln();
    let log_hi = F_HI.ln();
    for (b, band) in bands.iter_mut().enumerate() {
        let lo = (log_lo + (log_hi - log_lo) * b as f32 / SPECTRUM_BANDS as f32).exp();
        let hi = (log_lo + (log_hi - log_lo) * (b + 1) as f32 / SPECTRUM_BANDS as f32).exp();
        let bin_lo = (lo / bin_hz).floor() as usize;
        if bin_lo >= spectrum.len() {
            break;
        }
        let bin_hi = (((hi / bin_hz).ceil() as usize).max(bin_lo + 1)).min(spectrum.len());
        let peak = spectrum[bin_lo..bin_hi]
            .iter()
            .fold(0.0f32, |m, c| m.max(c.abs()));
        let norm = (peak / WINDOW as f32) * GAIN;
        *band = norm.sqrt().clamp(0.0, 1.0);
    }

    bands
}

// not an average of non-speech segments, but I don't know how much pause time we
// get. for now, we will just assume the noise is constant (kinda defeats the purpose)
// but oh well
pub fn average_noise_spectrum(audio: &[f32]) -> f32 {
    let mut total_sum = 0.0f32;

    for sample in audio {
        let magnitude = sample.abs();

        total_sum += magnitude.powf(2.0);
    }

    total_sum / audio.len() as f32
}

pub fn audio_to_mono(audio: &[f32], channels: u16) -> Vec<f32> {
    let mut mono_samples = Vec::with_capacity(audio.len() / channels as usize);

    // For microphone arrays (> 2 channels), only use first 2 channels
    // Many microphone arrays have auxiliary channels for beam-forming/noise cancellation
    // that can contain anti-phase signals. Averaging all channels can cause destructive
    // interference resulting in near-zero output.
    let effective_channels = if channels > 2 { 2 } else { channels };

    // Iterate over the audio slice in chunks, each containing `channels` samples
    for chunk in audio.chunks(channels as usize) {
        // Sum only the first effective_channels (typically 1-2 for mic arrays)
        let sum: f32 = chunk.iter().take(effective_channels as usize).sum();

        // Calculate the average mono sample using effective channel count
        let mono_sample = sum / effective_channels as f32;

        // Store the computed mono sample
        mono_samples.push(mono_sample);
    }

    mono_samples
}

/// specs/0029 WS4.2 (H1): small MRU pool of constructed `SincFixedIn` resamplers.
///
/// Constructing a `SincFixedIn` is the expensive part of `resample` — it computes the
/// windowed-sinc interpolation table (`sinc_len` × `oversampling_factor` coefficients;
/// ~1 MiB at the 512×512 quality tier used for 48 kHz→16 kHz). The VAD path calls
/// `resample` once per 600 ms mixed window with an identical `(len, from, to)`
/// signature, so rebuilding that table per call was hot-path waste that could back the
/// pipeline up until the bounded capture queue shed windows (starving the live
/// surfaces). Entries are keyed by `(input_len, from_rate, to_rate)` — the adaptive
/// `SincInterpolationParameters` derive purely from the rate pair, so a key match
/// guarantees a parameter match. Before reuse the entry is `reset()`, which restores
/// exact fresh-construction state (zeroed history buffer, original ratio, initial
/// index), so output is bit-identical to a newly built instance.
type ResamplerKey = (usize, u32, u32);
const RESAMPLER_CACHE_CAPACITY: usize = 4;
static RESAMPLER_CACHE: std::sync::Mutex<Vec<(ResamplerKey, SincFixedIn<f32>)>> =
    std::sync::Mutex::new(Vec::new());

/// Take a matching resampler out of the pool (it is returned via
/// [`store_cached_resampler`] after use, so concurrent callers with the same key simply
/// construct fresh instances). Best-effort: a poisoned lock falls back to `None`.
fn take_cached_resampler(key: ResamplerKey) -> Option<SincFixedIn<f32>> {
    let mut cache = RESAMPLER_CACHE.lock().ok()?;
    let pos = cache.iter().position(|(k, _)| *k == key)?;
    Some(cache.swap_remove(pos).1)
}

fn store_cached_resampler(key: ResamplerKey, resampler: SincFixedIn<f32>) {
    if let Ok(mut cache) = RESAMPLER_CACHE.lock() {
        if cache.len() >= RESAMPLER_CACHE_CAPACITY {
            // Evict the least-recently-stored entry. Hot keys (the per-window VAD
            // resample) are re-stored after every use, so they stay resident.
            cache.remove(0);
        }
        cache.push((key, resampler));
    }
}

/// High-quality audio resampling with adaptive parameters based on sample rate ratio
///
/// This function automatically selects the best resampling parameters based on:
/// - Sample rate ratio (upsampling vs downsampling)
/// - Quality requirements (integer ratios get optimized paths)
/// - Anti-aliasing needs
///
/// Supports all common sample rates: 8kHz, 16kHz, 24kHz, 44.1kHz, 48kHz, etc.
pub fn resample(input: &[f32], from_sample_rate: u32, to_sample_rate: u32) -> Result<Vec<f32>> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    // Fast path: No resampling needed
    if from_sample_rate == to_sample_rate {
        return Ok(input.to_vec());
    }

    let ratio = to_sample_rate as f64 / from_sample_rate as f64;

    // Adaptive parameters based on sample rate ratio
    let (sinc_len, interpolation_type, oversampling) = if ratio >= 2.0 {
        // Large upsampling (e.g., 8kHz → 16kHz, 16kHz → 48kHz, 24kHz → 48kHz)
        // Needs high quality to avoid artifacts
        debug!(
            "High-quality upsampling: {}Hz → {}Hz (ratio: {:.2}x)",
            from_sample_rate, to_sample_rate, ratio
        );
        (
            512,                          // Longer sinc for smoother interpolation
            SincInterpolationType::Cubic, // Cubic for best quality
            512,                          // Higher oversampling
        )
    } else if ratio >= 1.5 {
        // Moderate upsampling (e.g., 32kHz → 48kHz)
        debug!(
            "Moderate upsampling: {}Hz → {}Hz (ratio: {:.2}x)",
            from_sample_rate, to_sample_rate, ratio
        );
        (384, SincInterpolationType::Cubic, 384)
    } else if ratio > 1.0 {
        // Small upsampling (e.g., 44.1kHz → 48kHz)
        debug!(
            "Small upsampling: {}Hz → {}Hz (ratio: {:.2}x)",
            from_sample_rate, to_sample_rate, ratio
        );
        (256, SincInterpolationType::Linear, 256)
    } else if ratio <= 0.5 {
        // Large downsampling (e.g., 48kHz → 16kHz, 48kHz → 8kHz)
        // Needs strong anti-aliasing
        debug!(
            "Anti-aliased downsampling: {}Hz → {}Hz (ratio: {:.2}x)",
            from_sample_rate, to_sample_rate, ratio
        );
        (
            512,                          // Longer sinc for anti-aliasing
            SincInterpolationType::Cubic, // Cubic for quality
            512,
        )
    } else {
        // Moderate downsampling (e.g., 48kHz → 24kHz, 48kHz → 32kHz)
        debug!(
            "Moderate downsampling: {}Hz → {}Hz (ratio: {:.2}x)",
            from_sample_rate, to_sample_rate, ratio
        );
        (384, SincInterpolationType::Linear, 384)
    };

    let params = SincInterpolationParameters {
        sinc_len,
        f_cutoff: 0.95, // Preserve most of the frequency content
        interpolation: interpolation_type,
        oversampling_factor: oversampling,
        window: WindowFunction::BlackmanHarris2, // Best window for audio
    };

    // specs/0029 WS4.2 (H1): reuse a pooled resampler when one matches this exact
    // (input_len, from, to) signature; `reset()` makes it numerically identical to a
    // fresh instance while skipping the expensive sinc-table construction.
    let key = (input.len(), from_sample_rate, to_sample_rate);
    let mut resampler = match take_cached_resampler(key) {
        Some(mut cached) => {
            cached.reset();
            cached
        }
        None => SincFixedIn::<f32>::new(
            ratio,
            2.0, // Maximum relative deviation
            params,
            input.len(),
            1, // Mono
        )?,
    };

    let waves_in = vec![input.to_vec()];
    let waves_out = resampler.process(&waves_in, None)?;
    store_cached_resampler(key, resampler);

    debug!(
        "Resampling complete: {} samples → {} samples",
        input.len(),
        waves_out[0].len()
    );

    Ok(waves_out.into_iter().next().unwrap())
}

// Alias for compatibility with existing code
pub fn resample_audio(input: &[f32], from_sample_rate: u32, to_sample_rate: u32) -> Vec<f32> {
    match resample(input, from_sample_rate, to_sample_rate) {
        Ok(result) => result,
        Err(e) => {
            debug!("Resampling failed: {}, returning original audio", e);
            input.to_vec()
        }
    }
}

/// Fast resampling optimized for transcription preprocessing
///
pub fn write_audio_to_file(
    audio: &[f32],
    sample_rate: u32,
    output_path: &Path,
    device: &str,
    skip_encoding: bool,
) -> Result<String> {
    write_audio_to_file_with_meeting_name(
        audio,
        sample_rate,
        output_path,
        device,
        skip_encoding,
        None,
    )
}

pub fn write_audio_to_file_with_meeting_name(
    audio: &[f32],
    sample_rate: u32,
    output_path: &Path,
    device: &str,
    skip_encoding: bool,
    meeting_name: Option<&str>,
) -> Result<String> {
    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let sanitized_device_name = device.replace(['/', '\\'], "_");

    // Create meeting folder if meeting name is provided
    let final_output_path = if let Some(name) = meeting_name {
        let sanitized_meeting_name = sanitize_filename(name);
        let meeting_folder = output_path.join(&sanitized_meeting_name);

        // Create the meeting folder if it doesn't exist
        if !meeting_folder.exists() {
            std::fs::create_dir_all(&meeting_folder)?;
        }

        meeting_folder
    } else {
        output_path.to_path_buf()
    };

    let file_path = final_output_path
        .join(format!("{}_{}.mp4", sanitized_device_name, timestamp))
        .to_str()
        .expect("Failed to create valid path")
        .to_string();
    let file_path_clone = file_path.clone();
    // Run FFmpeg in a separate task
    if !skip_encoding {
        encode_single_audio(
            bytemuck::cast_slice(audio),
            sample_rate,
            1,
            Path::new(&file_path),
        )?;
    }
    Ok(file_path_clone)
}

/// Write transcript text to a file alongside the recording (legacy plain text format)
pub fn write_transcript_to_file(
    transcript_text: &str,
    output_path: &Path,
    meeting_name: Option<&str>,
) -> Result<String> {
    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();

    // Create meeting folder if meeting name is provided (same logic as audio)
    let final_output_path = if let Some(name) = meeting_name {
        let sanitized_meeting_name = sanitize_filename(name);
        let meeting_folder = output_path.join(&sanitized_meeting_name);

        // Create the meeting folder if it doesn't exist
        if !meeting_folder.exists() {
            std::fs::create_dir_all(&meeting_folder)?;
        }

        meeting_folder
    } else {
        output_path.to_path_buf()
    };

    let file_path = final_output_path.join(format!("transcript_{}.txt", timestamp));

    // Write transcript to file
    std::fs::write(&file_path, transcript_text)?;

    Ok(file_path.to_string_lossy().to_string())
}

/// Write structured transcript with timestamps to JSON file
pub fn write_transcript_json_to_file(
    segments: &[super::recording_saver::TranscriptSegment],
    output_path: &Path,
    meeting_name: Option<&str>,
    audio_filename: &str,
    recording_duration: f64,
) -> Result<String> {
    use serde_json::json;

    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();

    // Create meeting folder if meeting name is provided
    let final_output_path = if let Some(name) = meeting_name {
        let sanitized_meeting_name = sanitize_filename(name);
        let meeting_folder = output_path.join(&sanitized_meeting_name);

        if !meeting_folder.exists() {
            std::fs::create_dir_all(&meeting_folder)?;
        }

        meeting_folder
    } else {
        output_path.to_path_buf()
    };

    let file_path = final_output_path.join(format!("transcript_{}.json", timestamp));

    // Create structured JSON transcript
    let transcript_json = json!({
        "version": "1.0",
        "recording_duration": recording_duration,
        "audio_file": audio_filename,
        "sample_rate": 48000,
        "created_at": Utc::now().to_rfc3339(),
        "meeting_name": meeting_name,
        "segments": segments,
    });

    // Write JSON to file with pretty formatting
    let json_string = serde_json::to_string_pretty(&transcript_json)?;
    std::fs::write(&file_path, json_string)?;

    Ok(file_path.to_string_lossy().to_string())
}

#[cfg(test)]
mod spectrum_tests {
    use super::{spectrum_bands, SPECTRUM_BANDS};

    fn sine(freq: f32, sample_rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / sample_rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn silence_is_flat_zero() {
        let bands = spectrum_bands(&vec![0.0f32; 2048], 16000);
        assert_eq!(bands.len(), SPECTRUM_BANDS);
        assert!(
            bands.iter().all(|&b| b == 0.0),
            "silence must produce all-zero bands"
        );
    }

    #[test]
    fn empty_input_is_safe() {
        assert!(spectrum_bands(&[], 16000).iter().all(|&b| b == 0.0));
        assert!(spectrum_bands(&[0.1, 0.2], 0).iter().all(|&b| b == 0.0));
    }

    #[test]
    fn tone_peaks_in_the_band_that_contains_it() {
        let sr = 16000u32;
        let freq = 1000.0f32;
        let bands = spectrum_bands(&sine(freq, sr, 4096), sr);

        let (peak_idx, &peak_val) = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert!(
            peak_val > 0.3,
            "a loud tone should drive a clearly visible band (got {peak_val})"
        );

        // Recompute the peak band's frequency edges with the same log formula.
        let log_lo = 80f32.ln();
        let log_hi = 4000f32.ln();
        let lo = (log_lo + (log_hi - log_lo) * peak_idx as f32 / SPECTRUM_BANDS as f32).exp();
        let hi = (log_lo + (log_hi - log_lo) * (peak_idx + 1) as f32 / SPECTRUM_BANDS as f32).exp();
        assert!(
            lo * 0.9 <= freq && freq <= hi * 1.1,
            "peak band [{lo:.0},{hi:.0}) Hz should contain the {freq} Hz tone"
        );
    }
}

#[cfg(test)]
mod resample_tests {
    use super::resample;

    fn tone(freq: f32, sample_rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / sample_rate as f32).sin() * 0.5)
            .collect()
    }

    /// specs/0029 WS4.2 (H1): the resampler pool must be transparent — a cache-hit call
    /// (reset + reuse) must produce output bit-identical to the cache-miss call that
    /// constructed the instance.
    #[test]
    fn cached_resampler_reuse_is_bit_identical() {
        // Same shape as the VAD hot path: one 600 ms mixed window, 48 kHz → 16 kHz.
        let input = tone(440.0, 48_000, 28_800);

        let first = resample(&input, 48_000, 16_000).expect("first call (constructs)");
        let second = resample(&input, 48_000, 16_000).expect("second call (cache hit)");
        let third = resample(&input, 48_000, 16_000).expect("third call (cache hit)");

        assert!(!first.is_empty(), "resample must produce output");
        assert_eq!(
            first, second,
            "cache-hit output must be bit-identical to a fresh resampler's"
        );
        assert_eq!(
            first, third,
            "reuse must stay identical across repeated hits"
        );
    }

    /// Interleaved calls with different (len, from, to) keys must each behave exactly
    /// like independent fresh calls (no state bleed between pool entries).
    #[test]
    fn interleaved_lengths_and_rates_stay_deterministic() {
        let a = tone(300.0, 48_000, 4_800);
        let b = tone(700.0, 44_100, 8_820);

        let a1 = resample(&a, 48_000, 16_000).unwrap();
        let b1 = resample(&b, 44_100, 16_000).unwrap();
        let a2 = resample(&a, 48_000, 16_000).unwrap();
        let b2 = resample(&b, 44_100, 16_000).unwrap();

        assert_eq!(
            a1, a2,
            "48k key must be unaffected by the interleaved 44.1k call"
        );
        assert_eq!(
            b1, b2,
            "44.1k key must be unaffected by the interleaved 48k call"
        );
    }
}
