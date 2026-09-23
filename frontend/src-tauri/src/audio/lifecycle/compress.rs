//! Compress a processed meeting's channel WAVs to Opus (specs/0072 task 9).
//!
//! For each channel file (`mic.wav`, `system.wav`, `mic_segNN.wav`, `system_segNN.wav`):
//! encode to a hidden temp (`.{stem}.opus.tmp`, Ogg Opus, the W0 codec), then **verify by
//! decoding it** and comparing against the WAV decoded the same way: the sample counts must
//! agree to within 1% (floored at one 20 ms Opus frame, capped at 50 ms) and a WAV with
//! signal must not decode to silence. Only when every channel passed are the temps fsynced
//! and renamed to `{stem}.opus`, and only then are the WAVs removed. Both channels or
//! neither: any failure removes every temp and every `.opus` this run renamed, and leaves
//! the WAVs exactly as they were. "Encoder missing" is just another failure.
//!
//! Synchronous and CPU-bound: call it from `spawn_blocking`, under the meeting's
//! `Compression` folder lease (the sweep does both).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

use super::state::is_channel_stem;

/// The W0 channel codec (spec Implementation notes): Opus, 24 kbps, `-application audio`.
pub const CHANNEL_OPUS_ARGS: [&str; 6] =
    ["-c:a", "libopus", "-b:a", "24k", "-application", "audio"];

/// Channel files are 16 kHz mono; both sides are decoded at this rate for the comparison.
const VERIFY_RATE: u64 = 16_000;

/// What a compression changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompressOutcome {
    pub files: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

/// Test hooks, never set in production (`CompressOptions::default()`), in the style of the
/// mover's `ExecOptions`: integration tests link the library without `cfg(test)`.
#[derive(Debug, Clone, Default)]
pub struct CompressOptions {
    /// Stop the encoder after this many seconds (simulates a truncated encode).
    pub truncate_encode_secs: Option<f64>,
    /// Make the encode of this file name (e.g. `"system.wav"`) fail.
    pub fail_encode_of: Option<String>,
}

/// The channel WAVs in `folder`, sorted.
pub fn channel_wavs(folder: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("wav"))
                        && p.file_stem()
                            .is_some_and(|s| is_channel_stem(&s.to_string_lossy()))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Compress every channel WAV in `folder`. `Ok` with `files == 0` when there's nothing to do.
pub fn compress_channels(ffmpeg: &Path, folder: &Path) -> Result<CompressOutcome> {
    compress_channels_with(ffmpeg, folder, &CompressOptions::default())
}

/// [`compress_channels`] with test hooks.
pub fn compress_channels_with(
    ffmpeg: &Path,
    folder: &Path,
    opts: &CompressOptions,
) -> Result<CompressOutcome> {
    let wavs = channel_wavs(folder);
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new(); // (wav, tmp, final)
    for wav in &wavs {
        let stem = wav
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let tmp = folder.join(format!(".{stem}.opus.tmp"));
        let dst = folder.join(format!("{stem}.opus"));
        staged.push((wav.clone(), tmp.clone(), dst));
        if let Err(e) = encode_and_verify(ffmpeg, wav, &tmp, opts) {
            remove_all(staged.iter().map(|(_, t, _)| t));
            return Err(e.context(format!("kept {} uncompressed", wav.display())));
        }
    }
    if staged.is_empty() {
        return Ok(CompressOutcome::default());
    }

    // Swap phase: rename every temp into place. A failure undoes the renames already done.
    let mut renamed: Vec<&PathBuf> = Vec::new();
    for (_, tmp, dst) in &staged {
        if let Err(e) = std::fs::rename(tmp, dst) {
            remove_all(renamed.iter().copied());
            remove_all(staged.iter().map(|(_, t, _)| t));
            return Err(anyhow!(
                "Could not move the compressed {} into place ({e}); kept the WAVs",
                dst.display()
            ));
        }
        renamed.push(dst);
    }
    sync_dir(folder);

    let mut outcome = CompressOutcome {
        files: staged.len(),
        ..CompressOutcome::default()
    };
    for (wav, _, dst) in &staged {
        outcome.bytes_before += std::fs::metadata(wav).map(|m| m.len()).unwrap_or(0);
        outcome.bytes_after += std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        if let Err(e) = std::fs::remove_file(wav) {
            // Harmless: the WAV is preferred by readers and the next sweep re-encodes it.
            log::warn!("Compressed {} but could not remove it: {e}", wav.display());
        }
    }
    sync_dir(folder);
    Ok(outcome)
}

fn encode_and_verify(ffmpeg: &Path, wav: &Path, tmp: &Path, opts: &CompressOptions) -> Result<()> {
    let _ = std::fs::remove_file(tmp);
    let name = wav
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(wav)
        .args(["-vn", "-ac", "1"])
        .args(CHANNEL_OPUS_ARGS);
    if let Some(secs) = opts.truncate_encode_secs {
        cmd.args(["-t", &secs.to_string()]);
    }
    if opts.fail_encode_of.as_deref() == Some(name.as_str()) {
        cmd.args(["-c:a", "nixon-no-such-encoder"]);
    }
    let out = cmd
        .args(["-f", "opus"])
        .arg(tmp)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("could not run ffmpeg to compress {name}"))?;
    if !out.status.success() {
        bail!(
            "ffmpeg could not compress {name}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let (want, want_peak) = decode_stats(ffmpeg, wav)?;
    let (got, got_peak) = decode_stats(ffmpeg, tmp)?;
    let tolerance = (want / 100).clamp(VERIFY_RATE / 50, VERIFY_RATE / 20);
    if want.abs_diff(got) > tolerance {
        bail!(
            "the compressed {name} decodes to {got} samples, the WAV has {want} \
             (allowed difference {tolerance})"
        );
    }
    if want_peak > 64 && got_peak == 0 {
        bail!("the compressed {name} decodes to silence");
    }
    std::fs::File::open(tmp)
        .and_then(|f| f.sync_all())
        .with_context(|| format!("could not flush the compressed {name}"))?;
    Ok(())
}

/// Decode `path` with ffmpeg to 16 kHz mono s16 and return (sample count, peak |sample|),
/// streamed so an hour-long channel never sits in memory.
pub fn decode_stats(ffmpeg: &Path, path: &Path) -> Result<(u64, u16)> {
    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", &VERIFY_RATE.to_string()])
        .args(["-f", "s16le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not run ffmpeg to decode {}", path.display()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("ffmpeg gave no output for {}", path.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let (mut bytes, mut peak, mut carry): (u64, u16, Option<u8>) = (0, 0, None);
    loop {
        let n = stdout.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        let mut chunk = &buf[..n];
        if let Some(lo) = carry.take() {
            peak = peak.max(i16::from_le_bytes([lo, chunk[0]]).unsigned_abs());
            chunk = &chunk[1..];
        }
        let mut pairs = chunk.chunks_exact(2);
        for p in &mut pairs {
            peak = peak.max(i16::from_le_bytes([p[0], p[1]]).unsigned_abs());
        }
        carry = pairs.remainder().first().copied();
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("ffmpeg could not decode {}", path.display());
    }
    Ok((bytes / 2, peak))
}

fn remove_all<'a>(paths: impl Iterator<Item = &'a PathBuf>) {
    for p in paths {
        let _ = std::fs::remove_file(p);
    }
}

fn sync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
}
