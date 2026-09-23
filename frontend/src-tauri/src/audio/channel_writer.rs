//! Per-channel 16 kHz mono WAV writer for offline speaker diarization (specs/0010, P1-B1).
//!
//! During recording the pipeline mixes the microphone and system streams into a
//! single signal before transcription (see `pipeline.rs`), which destroys channel
//! identity. Diarization (specs/0010) needs the streams *unmixed*: the mic channel
//! is the local user (`You`) and diarization clustering runs only on the system
//! channel. To enable a post-meeting diarization pass we additionally persist the
//! two channels here, tapped BEFORE the RMS-duck mixing so they stay clean.
//!
//! Output format is exactly what sherpa-onnx diarization consumes (see the spec's
//! "Spike findings"): **16 kHz, mono, 16-bit PCM WAV**. The pipeline runs at 48 kHz,
//! so we downsample 48 kHz → 16 kHz with the same `rubato` sinc resampler the STT
//! path uses (a persistent `SincFixedIn` fed fixed-size chunks, to preserve energy
//! across windows — a fresh per-window resampler would amplify RMS, see `pipeline.rs`).
//!
//! This writer is **additive and best-effort**: every public method returns
//! `anyhow::Result`, and the caller (the pipeline) logs-and-continues on any error so
//! that per-channel capture can NEVER break the primary mixed recording or live
//! transcription.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use log::info;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

/// Capture sample rate of the recording pipeline (mic + system are normalized to this).
const CAPTURE_SAMPLE_RATE: u32 = 48_000;
/// Target sample rate for diarization input.
const DIARIZATION_SAMPLE_RATE: u32 = 16_000;
/// Fixed input chunk size fed to the persistent resampler. Matches the capture-path
/// convention (`pipeline.rs` uses 512); the resampler is `SincFixedIn`, so input must
/// arrive in fixed-size blocks.
const RESAMPLER_CHUNK_SIZE: usize = 512;
/// Bytes in a canonical 44-byte PCM WAV header (RIFF + fmt + data headers).
const WAV_HEADER_BYTES: u64 = 44;

/// Filename of the system-audio channel inside a meeting folder (the REQUIRED file:
/// diarization clustering runs on this stream).
pub const SYSTEM_CHANNEL_FILENAME: &str = "system.wav";
/// Filename of the microphone channel inside a meeting folder (the local user / `You`).
pub const MIC_CHANNEL_FILENAME: &str = "mic.wav";

/// A persistent 48 kHz → 16 kHz mono downsampler that processes audio in fixed-size
/// blocks through one `SincFixedIn` resampler.
///
/// Factored out of [`ChannelWavWriter`] so live diarization (`diarization::live`) can
/// build its growing 16 kHz buffer with **byte-for-byte the same** downsampling the
/// `system.wav` writer uses — same sinc params, same fixed block size, same energy
/// preservation across windows (a fresh per-window resampler would amplify RMS, see
/// the module docs). This guarantees the live diarizer and the offline pass see the
/// same audio.
pub struct WindowDownsampler {
    resampler: SincFixedIn<f32>,
    /// Accumulates incoming source samples until a full block exists.
    input_buffer: Vec<f32>,
}

impl WindowDownsampler {
    /// Create a downsampler from `from_rate` to `to_rate` (mono). Only the
    /// 48 kHz → 16 kHz case is used in this app, but the ratio is computed generally.
    pub fn new(from_rate: u32, to_rate: u32) -> Result<Self> {
        let ratio = to_rate as f64 / from_rate as f64;
        // Anti-aliased downsampling params — identical to `ChannelWavWriter::new`.
        let params = SincInterpolationParameters {
            sinc_len: 512,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Cubic,
            oversampling_factor: 512,
            window: WindowFunction::BlackmanHarris2,
        };
        let resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, RESAMPLER_CHUNK_SIZE, 1)
            .context("failed to create window downsampler")?;
        Ok(Self {
            resampler,
            input_buffer: Vec::with_capacity(RESAMPLER_CHUNK_SIZE * 2),
        })
    }

    /// Push one source-rate mono window; append the resulting target-rate samples to
    /// `out`. Residual (< one block) is retained for the next call.
    pub fn push(&mut self, window: &[f32], out: &mut Vec<f32>) -> Result<()> {
        self.input_buffer.extend_from_slice(window);
        while self.input_buffer.len() >= RESAMPLER_CHUNK_SIZE {
            let chunk: Vec<f32> = self.input_buffer.drain(0..RESAMPLER_CHUNK_SIZE).collect();
            let waves_out = self
                .resampler
                .process(&[chunk], None)
                .context("window downsampler process failed")?;
            if let Some(o) = waves_out.into_iter().next() {
                out.extend_from_slice(&o);
            }
        }
        Ok(())
    }

    /// Flush any buffered residual (< one block) by zero-padding to a full block and
    /// processing it, appending the result to `out`. Called once at stream end (a few
    /// ms of trailing silence is harmless). After this the downsampler is spent.
    pub fn flush_tail(&mut self, out: &mut Vec<f32>) -> Result<()> {
        if self.input_buffer.is_empty() {
            return Ok(());
        }
        let mut tail = std::mem::take(&mut self.input_buffer);
        tail.resize(RESAMPLER_CHUNK_SIZE, 0.0);
        if let Ok(waves_out) = self.resampler.process(&[tail], None) {
            if let Some(o) = waves_out.into_iter().next() {
                out.extend_from_slice(&o);
            }
        }
        Ok(())
    }
}

/// Readers use these (`.wav` or the compressed `.opus`, specs/0072); writers use `*_channel_wav`.
pub use super::channel_files::{mic_channel_path, system_channel_path};

/// Resolve the system-channel WAV for a finished meeting's folder.
///
/// This is the file the diarization pipeline (the next slice) consumes. The meeting
/// folder is the per-meeting directory under the recordings root (e.g.
/// `~/Movies/nixon-recordings/<meeting>/`), tracked at runtime via
/// `RecordingSaver::get_meeting_folder()`.
pub fn system_channel_wav(meeting_folder: &Path) -> PathBuf {
    meeting_folder.join(SYSTEM_CHANNEL_FILENAME)
}

/// Resolve the microphone-channel WAV for a finished meeting's folder. Cheap and
/// useful later (e.g. confirming the mic/local short-circuit); not required for P1.
pub fn mic_channel_wav(meeting_folder: &Path) -> PathBuf {
    meeting_folder.join(MIC_CHANNEL_FILENAME)
}

/// Streams one channel of 48 kHz mono f32 audio to a 16 kHz mono 16-bit PCM WAV file.
///
/// Feed pre-mix windows via [`ChannelWavWriter::write_window`]; call
/// [`ChannelWavWriter::finalize`] at stop to flush and patch the WAV size fields.
pub struct ChannelWavWriter {
    path: PathBuf,
    /// `Option` so [`finalize`](ChannelWavWriter::finalize) can take the writer out
    /// and the [`Drop`] impl can tell whether finalize already ran (see the flag).
    writer: Option<BufWriter<File>>,
    /// Shared 48 kHz → 16 kHz downsampler (also used by live diarization for parity).
    downsampler: WindowDownsampler,
    /// Number of 16-bit samples written to the data chunk (for header patching).
    samples_written: u64,
    /// Set once the header has been patched (via `finalize` or `Drop`); prevents the
    /// `Drop` impl from re-patching after an explicit `finalize` and guards against a
    /// double patch corrupting the file.
    finalized: bool,
}

impl ChannelWavWriter {
    /// Create a writer that downsamples 48 kHz → 16 kHz and writes a 16-bit PCM mono WAV
    /// at `path`. Writes a placeholder header immediately (patched in `finalize`).
    pub fn new(path: PathBuf) -> Result<Self> {
        // 48 kHz → 16 kHz downsampler shared with live diarization (see
        // [`WindowDownsampler`]) so `system.wav` and the live buffer stay in lockstep.
        let downsampler = WindowDownsampler::new(CAPTURE_SAMPLE_RATE, DIARIZATION_SAMPLE_RATE)
            .context("failed to create per-channel resampler")?;

        let file = File::create(&path)
            .with_context(|| format!("failed to create channel WAV {}", path.display()))?;
        let mut writer = BufWriter::new(file);

        // Placeholder header with zero data length; patched in `finalize`.
        write_wav_header(&mut writer, DIARIZATION_SAMPLE_RATE, 0)
            .with_context(|| format!("failed to write WAV header {}", path.display()))?;

        Ok(Self {
            path,
            writer: Some(writer),
            downsampler,
            samples_written: 0,
            finalized: false,
        })
    }

    /// Append one 48 kHz mono window for this channel. Buffers, resamples in fixed-size
    /// blocks, and streams the 16 kHz 16-bit samples to disk. Residual (< one block) is
    /// retained for the next call and flushed by `finalize`.
    pub fn write_window(&mut self, window_48k: &[f32]) -> Result<()> {
        let mut out = Vec::new();
        self.downsampler
            .push(window_48k, &mut out)
            .context("per-channel resampler process failed")?;
        if !out.is_empty() {
            self.write_samples_16k(&out)?;
        }
        Ok(())
    }

    /// Flush any buffered tail (zero-padded to a full resampler block), patch the WAV
    /// header with the real data length, and return the file path.
    pub fn finalize(mut self) -> Result<PathBuf> {
        self.finish()?;
        Ok(self.path.clone())
    }

    /// Shared finalize body used by both [`finalize`](Self::finalize) and [`Drop`]:
    /// flush the residual tail, then patch the RIFF + `data` size fields. Idempotent
    /// via the `finalized` flag so an explicit `finalize` followed by `Drop` never
    /// double-patches (which would corrupt the header).
    fn finish(&mut self) -> Result<()> {
        if self.finalized {
            return Ok(());
        }
        self.finalized = true;

        // Flush the residual tail (zero-padded internally); a few ms of trailing
        // silence is harmless for diarization.
        let mut tail_out = Vec::new();
        self.downsampler.flush_tail(&mut tail_out).ok();
        if !tail_out.is_empty() {
            self.write_samples_16k(&tail_out)?;
        }

        let mut writer = self
            .writer
            .take()
            .ok_or_else(|| anyhow::anyhow!("channel WAV writer already taken"))?;
        writer.flush().context("flush channel WAV")?;

        // Patch RIFF chunk size and data sub-chunk size now that we know the length.
        let data_bytes = self.samples_written * 2; // 16-bit = 2 bytes/sample
                                                   // RIFF chunk size = file size - 8 = (36 + data_bytes).
        writer
            .seek(SeekFrom::Start(4))
            .context("seek to patch RIFF size")?;
        writer.write_all(&((36 + data_bytes) as u32).to_le_bytes())?;
        // data sub-chunk size at byte offset 40.
        writer
            .seek(SeekFrom::Start(40))
            .context("seek to patch data size")?;
        writer.write_all(&(data_bytes as u32).to_le_bytes())?;
        writer.flush().context("flush patched WAV header")?;

        info!(
            "✅ Per-channel WAV finalized: {} ({} samples @ {} Hz mono 16-bit)",
            self.path.display(),
            self.samples_written,
            DIARIZATION_SAMPLE_RATE
        );
        Ok(())
    }

    /// Convert 16 kHz f32 samples to 16-bit PCM and write them to the data chunk.
    fn write_samples_16k(&mut self, samples: &[f32]) -> Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(anyhow::anyhow!("channel WAV writer already finalized"));
        };
        for &s in samples {
            let clamped = s.clamp(-1.0, 1.0);
            let v = (clamped * i16::MAX as f32) as i16;
            writer.write_all(&v.to_le_bytes())?;
        }
        self.samples_written += samples.len() as u64;
        Ok(())
    }
}

impl Drop for ChannelWavWriter {
    /// Belt-and-suspenders finalize for the abort/early-exit path: if the pipeline
    /// task is dropped (e.g. aborted, or `run()` early-returns before
    /// `finalize_channel_writers()`), the writer's header would otherwise keep the
    /// placeholder data-size 0 / RIFF 36 values and the file would be undecodable by
    /// symphonia (the exact bug salvaged by [`repair_wav_header_if_needed`]). Drop
    /// runs the same patch the explicit `finalize` does. Best-effort: never panics,
    /// logs on error. The `finalized` flag makes this a no-op after `finalize`.
    fn drop(&mut self) {
        if self.finalized {
            return;
        }
        if let Err(e) = self.finish() {
            log::warn!(
                "⚠️ Per-channel WAV {} not finalized cleanly on drop: {e:#} \
                 (header may need repair on next decode)",
                self.path.display()
            );
        }
    }
}

/// Write a canonical 44-byte PCM WAV header (mono, 16-bit) for `sample_rate` with a
/// data length of `data_samples` 16-bit samples. With `data_samples == 0` this is a
/// placeholder, patched later in `finalize`.
fn write_wav_header<W: Write>(w: &mut W, sample_rate: u32, data_samples: u64) -> Result<()> {
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * channels as u32 * (bits_per_sample as u32 / 8);
    let block_align = channels * (bits_per_sample / 8);
    let data_bytes = (data_samples * 2) as u32;

    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_bytes).to_le_bytes())?; // RIFF chunk size
    w.write_all(b"WAVE")?;

    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // fmt chunk size (PCM)
    w.write_all(&1u16.to_le_bytes())?; // audio format = PCM
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&bits_per_sample.to_le_bytes())?;

    w.write_all(b"data")?;
    w.write_all(&data_bytes.to_le_bytes())?; // data sub-chunk size

    debug_assert_eq!(WAV_HEADER_BYTES, 44);
    Ok(())
}

/// Write a full **already-16 kHz** mono f32 buffer to `path` as a finalized 16 kHz
/// mono 16-bit PCM WAV — the diarization input format.
///
/// This is for callers (e.g. audio *import*, `audio/import.rs`) that already hold a
/// 16 kHz mono buffer (the same one handed to Whisper/Parakeet) and therefore must
/// **not** go through [`ChannelWavWriter`], which unconditionally downsamples
/// 48 kHz → 16 kHz and would corrupt an already-16 kHz signal. Unlike the streaming
/// writer, the full length is known up front, so the header is written with the real
/// size fields immediately — no placeholder/patch step.
///
/// Best-effort by contract like the rest of this module: returns `anyhow::Result` and
/// callers log-and-continue (a missing `system.wav` only means diarization is
/// unavailable, it must never fail the import).
pub(crate) fn write_16k_mono_wav(path: &Path, samples_16k: &[f32]) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("failed to create WAV {}", path.display()))?;
    let mut writer = BufWriter::new(file);

    // Real data length known up front → write the final header directly.
    write_wav_header(
        &mut writer,
        DIARIZATION_SAMPLE_RATE,
        samples_16k.len() as u64,
    )
    .with_context(|| format!("failed to write WAV header {}", path.display()))?;

    for &s in samples_16k {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * i16::MAX as f32) as i16;
        writer.write_all(&v.to_le_bytes())?;
    }
    writer.flush().context("flush 16 kHz mono WAV")?;

    info!(
        "✅ Wrote 16 kHz mono WAV: {} ({} samples)",
        path.display(),
        samples_16k.len()
    );
    Ok(())
}

/// Repair a WAV file whose RIFF/`data` size fields are still the placeholders our
/// writer emits at creation (`data` size 0, RIFF size 36) even though the file holds
/// valid PCM out to EOF — the unfinalized-header bug (specs/0010): a pipeline abort or
/// early-exit could drop a [`ChannelWavWriter`] before its header was patched. ffmpeg
/// reads such files (it ignores the size fields and reads to EOF) but symphonia trusts
/// the header and decodes 0 samples ("No audio samples decoded from file"), so
/// diarization fails.
///
/// This patches the header **in place** to the byte-correct values
/// [`ChannelWavWriter::finalize`] would have written: RIFF size = filesize − 8, and
/// `data` size = (filesize − data_payload_start). The PCM itself is untouched, so the
/// repaired file is permanently, correctly decodable.
///
/// Idempotent + defensive:
/// - Returns `Ok(false)` (a no-op, file unchanged) when the header is already valid,
///   i.e. the `data` size already matches the bytes available to EOF.
/// - Parses the RIFF chunk list to locate `data` rather than assuming offset 36/40,
///   but our writer always emits the canonical 44-byte layout, so that's the fast path.
/// - Only patches when the stored `data` size is smaller than the actual payload to
///   EOF (the placeholder case); never shrinks a file or touches non-WAV / truncated
///   files (returns `Ok(false)` for anything it can't confidently repair).
///
/// Returns whether it patched. Best-effort by contract; callers log + continue.
pub fn repair_wav_header_if_needed(path: &Path) -> Result<bool> {
    use std::io::Read;

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("open WAV for header repair {}", path.display()))?;

    let file_len = file
        .metadata()
        .with_context(|| format!("stat WAV {}", path.display()))?
        .len();

    // Must at least hold a canonical header to be a WAV we can reason about.
    if file_len < WAV_HEADER_BYTES {
        return Ok(false);
    }

    // Read the 12-byte RIFF/WAVE prelude.
    let mut prelude = [0u8; 12];
    file.seek(SeekFrom::Start(0)).context("seek WAV start")?;
    file.read_exact(&mut prelude).context("read RIFF prelude")?;
    if &prelude[0..4] != b"RIFF" || &prelude[8..12] != b"WAVE" {
        return Ok(false); // not a RIFF/WAVE container; leave untouched.
    }

    // Walk the chunk list (after the 12-byte prelude) to find `data`'s header offset.
    // Each chunk: 4-byte id + 4-byte LE size + payload (+ pad byte if size is odd).
    let mut pos: u64 = 12;
    let mut data_size_field_offset: Option<u64> = None;
    let mut data_payload_start: Option<u64> = None;
    let mut stored_data_size: u32 = 0;
    while pos + 8 <= file_len {
        let mut chunk_hdr = [0u8; 8];
        file.seek(SeekFrom::Start(pos))
            .context("seek chunk header")?;
        file.read_exact(&mut chunk_hdr)
            .context("read chunk header")?;
        let id = &chunk_hdr[0..4];
        let size = u32::from_le_bytes([chunk_hdr[4], chunk_hdr[5], chunk_hdr[6], chunk_hdr[7]]);
        let payload_start = pos + 8;
        if id == b"data" {
            data_size_field_offset = Some(pos + 4);
            data_payload_start = Some(payload_start);
            stored_data_size = size;
            break;
        }
        // Advance past this chunk's payload (+ pad byte if odd length).
        let advance = size as u64 + (size as u64 & 1);
        // Defensive: if the (non-data) chunk's declared size overruns EOF, bail.
        if payload_start + advance > file_len || advance == 0 {
            return Ok(false);
        }
        pos = payload_start + advance;
    }

    let (Some(size_offset), Some(payload_start)) = (data_size_field_offset, data_payload_start)
    else {
        return Ok(false); // no `data` chunk found.
    };

    // Bytes actually present from the data payload to EOF — the true PCM length.
    let actual_data_bytes = file_len - payload_start;

    // No-op if the header already reports the real length (already finalized) or the
    // file genuinely has no payload (a correct zero-length WAV).
    if stored_data_size as u64 == actual_data_bytes {
        return Ok(false);
    }
    // Only repair the placeholder/underflow case (stored smaller than what's on disk).
    // Never grow the stored size beyond the file or shrink a larger declared size.
    if (stored_data_size as u64) >= actual_data_bytes {
        return Ok(false);
    }

    // Patch the two size fields, mirroring `ChannelWavWriter::finalize` exactly:
    //   data sub-chunk size = actual_data_bytes
    //   RIFF chunk size     = file size - 8
    let data_size = actual_data_bytes as u32;
    let riff_size = (file_len - 8) as u32;

    file.seek(SeekFrom::Start(4)).context("seek RIFF size")?;
    file.write_all(&riff_size.to_le_bytes())
        .context("write RIFF size")?;
    file.seek(SeekFrom::Start(size_offset))
        .context("seek data size")?;
    file.write_all(&data_size.to_le_bytes())
        .context("write data size")?;
    file.flush().context("flush repaired WAV header")?;

    info!(
        "🛠️  Repaired unfinalized WAV header: {} (data {} → {} bytes, RIFF → {})",
        path.display(),
        stored_data_size,
        data_size,
        riff_size
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read the 16-bit little-endian sample count from a finished WAV's data chunk size.
    fn read_wav(path: &Path) -> (u16, u32, u16, u32) {
        let bytes = std::fs::read(path).expect("read wav");
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
        let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
        assert_eq!(&bytes[36..40], b"data");
        let data_size = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        (channels, sample_rate, bits, data_size)
    }

    #[test]
    fn writes_16k_mono_16bit_wav_with_expected_length() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("system.wav");

        // Feed 1 second of 48 kHz audio (a quiet sine so values are in range).
        let n = CAPTURE_SAMPLE_RATE as usize; // 48000 samples = 1.0s
        let input: Vec<f32> = (0..n)
            .map(|i| {
                (i as f32 / CAPTURE_SAMPLE_RATE as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5
            })
            .collect();

        let mut writer = ChannelWavWriter::new(path.clone()).expect("create writer");
        // Feed in irregular window sizes to exercise the buffering path.
        for w in input.chunks(600) {
            writer.write_window(w).expect("write window");
        }
        let out = writer.finalize().expect("finalize");
        assert_eq!(out, path);

        let (channels, sample_rate, bits, data_size) = read_wav(&path);
        assert_eq!(channels, 1, "must be mono");
        assert_eq!(sample_rate, DIARIZATION_SAMPLE_RATE, "must be 16 kHz");
        assert_eq!(bits, 16, "must be 16-bit PCM");

        // ~1s @ 16 kHz mono 16-bit ≈ 16000 samples * 2 bytes. Resampler edge effects +
        // tail-padding make this approximate; assert it's within a small tolerance.
        let samples = data_size / 2;
        let expected = DIARIZATION_SAMPLE_RATE; // 16000
        let diff = (samples as i64 - expected as i64).unsigned_abs();
        assert!(
            diff < 2_000,
            "expected ~{expected} samples (16 kHz * 1s), got {samples} (data_size={data_size})"
        );
        assert!(samples > 0, "no audio written");

        // The WAV must decode cleanly via the app's decoder (symphonia) as 16 kHz mono.
        let decoded = crate::audio::decoder::decode_audio_file(&path).expect("decode written wav");
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.sample_rate, DIARIZATION_SAMPLE_RATE);
    }

    #[test]
    fn write_16k_mono_wav_writes_decodable_16k_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path();
        // Use the real folder-resolution helper so the file lands where diarization looks.
        let path = system_channel_wav(folder);

        // 1 second of already-16 kHz mono audio (a quiet sine, in range).
        let n = DIARIZATION_SAMPLE_RATE as usize; // 16000 samples = 1.0s
        let input: Vec<f32> = (0..n)
            .map(|i| {
                (i as f32 / DIARIZATION_SAMPLE_RATE as f32 * 220.0 * std::f32::consts::TAU).sin()
                    * 0.5
            })
            .collect();

        write_16k_mono_wav(&path, &input).expect("write 16k mono wav");

        // (a) The file exists exactly at the path diarization resolves.
        assert!(path.exists(), "system.wav must exist at {}", path.display());

        // Header is correct: mono / 16 kHz / 16-bit, and the data length is exact
        // (no downsampling, no resampler edge effects — samples pass through 1:1).
        let (channels, sample_rate, bits, data_size) = read_wav(&path);
        assert_eq!(channels, 1, "must be mono");
        assert_eq!(sample_rate, DIARIZATION_SAMPLE_RATE, "must be 16 kHz");
        assert_eq!(bits, 16, "must be 16-bit PCM");
        assert_eq!(data_size as usize, n * 2, "16-bit samples pass through 1:1");

        // (b) It decodes back to ~16 kHz mono with the expected sample count.
        let decoded = crate::audio::decoder::decode_audio_file(&path).expect("decode written wav");
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.sample_rate, DIARIZATION_SAMPLE_RATE);
        assert_eq!(decoded.samples.len(), n, "all samples decode 1:1");
    }

    #[test]
    fn empty_recording_produces_valid_zero_length_wav() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mic.wav");
        let writer = ChannelWavWriter::new(path.clone()).expect("create writer");
        writer.finalize().expect("finalize empty");

        let (channels, sample_rate, bits, data_size) = read_wav(&path);
        assert_eq!(channels, 1);
        assert_eq!(sample_rate, DIARIZATION_SAMPLE_RATE);
        assert_eq!(bits, 16);
        assert_eq!(data_size, 0, "no windows written → zero-length data chunk");
    }

    /// Build a WAV with our canonical 44-byte header but the placeholder size fields
    /// (RIFF=36, data=0), followed by `n_samples` of 16-bit PCM — exactly the on-disk
    /// shape of the known-broken `system.wav`/`mic.wav` files. Returns the bytes.
    fn unfinalized_wav(sample_rate: u32, n_samples: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        // Placeholder header: data_samples = 0 → RIFF size 36, data size 0.
        write_wav_header(&mut bytes, sample_rate, 0).expect("write placeholder header");
        assert_eq!(bytes.len(), WAV_HEADER_BYTES as usize);
        // A quiet ramp of PCM so there's clearly real data past the header.
        for i in 0..n_samples {
            let v = ((i % 2000) as i16) - 1000;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn repair_patches_placeholder_header_and_decodes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("system.wav");

        let n_samples = DIARIZATION_SAMPLE_RATE as usize; // 1s @ 16 kHz
        std::fs::write(&path, unfinalized_wav(DIARIZATION_SAMPLE_RATE, n_samples))
            .expect("write broken wav");

        // Header starts as the placeholder: data size 0, RIFF 36.
        let (_, _, _, data_size_before) = read_wav(&path);
        assert_eq!(data_size_before, 0, "precondition: unfinalized header");

        // Repair patches it and reports it did.
        let patched = repair_wav_header_if_needed(&path).expect("repair");
        assert!(patched, "should report it patched the placeholder header");

        // Header now reports the true on-disk length.
        let (channels, sample_rate, bits, data_size_after) = read_wav(&path);
        assert_eq!(channels, 1);
        assert_eq!(sample_rate, DIARIZATION_SAMPLE_RATE);
        assert_eq!(bits, 16);
        assert_eq!(
            data_size_after as usize,
            n_samples * 2,
            "data size must equal samples * 2 bytes"
        );

        // And symphonia now decodes exactly the N samples it previously saw as 0.
        let decoded = crate::audio::decoder::decode_audio_file(&path).expect("decode repaired");
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.sample_rate, DIARIZATION_SAMPLE_RATE);
        assert_eq!(
            decoded.samples.len(),
            n_samples,
            "all PCM samples must decode after repair"
        );
    }

    #[test]
    fn repair_is_noop_on_finalized_wav() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("system.wav");

        // Produce a correctly finalized WAV via the real writer.
        let n = CAPTURE_SAMPLE_RATE as usize;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                (i as f32 / CAPTURE_SAMPLE_RATE as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5
            })
            .collect();
        let mut writer = ChannelWavWriter::new(path.clone()).expect("create writer");
        for w in input.chunks(600) {
            writer.write_window(w).expect("write window");
        }
        writer.finalize().expect("finalize");

        let before = std::fs::read(&path).expect("read finalized");

        // Repair must be a no-op: returns false and leaves the bytes untouched.
        let patched = repair_wav_header_if_needed(&path).expect("repair noop");
        assert!(!patched, "already-finalized WAV must not be patched");

        let after = std::fs::read(&path).expect("read after repair");
        assert_eq!(before, after, "no-op repair must not modify the file");
    }

    #[test]
    fn repair_is_noop_on_correct_zero_length_wav() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mic.wav");
        // A header-only WAV: data size 0 IS correct (no payload to EOF).
        let writer = ChannelWavWriter::new(path.clone()).expect("create writer");
        writer.finalize().expect("finalize empty");

        let before = std::fs::read(&path).expect("read empty");
        let patched = repair_wav_header_if_needed(&path).expect("repair noop");
        assert!(!patched, "correct zero-length WAV must not be patched");
        let after = std::fs::read(&path).expect("read after");
        assert_eq!(before, after);
    }

    #[test]
    fn drop_finalizes_unfinalized_writer() {
        // Dropping a writer WITHOUT calling finalize() (the abort path) must still
        // patch the header so the file decodes — the root-cause fix.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("system.wav");

        let n = CAPTURE_SAMPLE_RATE as usize;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                (i as f32 / CAPTURE_SAMPLE_RATE as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5
            })
            .collect();
        {
            let mut writer = ChannelWavWriter::new(path.clone()).expect("create writer");
            for w in input.chunks(600) {
                writer.write_window(w).expect("write window");
            }
            // Intentionally NO finalize(): writer goes out of scope here → Drop.
        }

        // Header must be patched by Drop: nonzero data size and decodable.
        let (_, sample_rate, _, data_size) = read_wav(&path);
        assert_eq!(sample_rate, DIARIZATION_SAMPLE_RATE);
        assert!(data_size > 0, "Drop must patch the data size field");

        let decoded =
            crate::audio::decoder::decode_audio_file(&path).expect("decode drop-finalized wav");
        assert_eq!(decoded.channels, 1);
        assert!(
            !decoded.samples.is_empty(),
            "Drop-finalized WAV has samples"
        );
    }
}
