//! Where a meeting's per-channel audio lives, in either format (specs/0072 W2).
//!
//! Capture writes `mic.wav` / `system.wav` (16 kHz mono PCM, `channel_writer`). Once a meeting
//! is processed and its audio is kept, the lifecycle compresses them to `mic.opus` /
//! `system.opus` (and resume segments to `mic_segNN.opus`, …). Every reader resolves through
//! here, WAV first and then Opus: a WAV beside its Opus holds the same audio (a compression
//! whose WAV removal failed) or newer audio (a resume re-concatenated the channel). Writers
//! keep `channel_writer::*_channel_wav`.
//!
//! Joining channel files decodes (the concat *filter*), because the concat demuxer's stream
//! copy can't join Opus with WAV. Output is always 16 kHz mono PCM WAV, the capture format.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use log::{info, warn};

use super::ffmpeg::find_ffmpeg_path;
use super::recording_saver::RecordingSegment;

/// The channel file formats, in the order readers prefer them.
const CHANNEL_EXTENSIONS: [&str; 2] = ["wav", "opus"];

fn channel_path(folder: &Path, stem: &str) -> Option<PathBuf> {
    CHANNEL_EXTENSIONS
        .iter()
        .map(|ext| folder.join(format!("{stem}.{ext}")))
        .find(|p| p.is_file())
}

/// The meeting's system channel (`system.wav`, else `system.opus`): what speaker
/// identification clusters. `None` when neither exists.
pub fn system_channel_path(folder: &Path) -> Option<PathBuf> {
    channel_path(folder, "system")
}

/// The meeting's microphone channel (`mic.wav`, else `mic.opus`).
pub fn mic_channel_path(folder: &Path) -> Option<PathBuf> {
    channel_path(folder, "mic")
}

/// A segment's channel file as recorded (`system_seg00.wav`), or its compressed sibling
/// (`system_seg00.opus`) when the compressor has run since. Only that one file: never
/// another segment's or the canonical channel.
fn recorded_or_compressed(folder: &Path, name: &str) -> Option<PathBuf> {
    let path = folder.join(name);
    if path.is_file() {
        return Some(path);
    }
    let other = match path.extension().and_then(|e| e.to_str()) {
        Some("wav") => path.with_extension("opus"),
        Some("opus") => path.with_extension("wav"),
        _ => return None,
    };
    other.is_file().then_some(other)
}

/// Resume: the name the first session's plain channel file has on disk right now —
/// `{stem}.wav`, or `{stem}.opus` once the lifecycle compressed it. `{stem}.wav` when
/// neither exists (a purged meeting: nothing to rename).
pub fn plain_channel_name(folder: &Path, stem: &str) -> String {
    let ext = if folder.join(format!("{stem}.wav")).exists() {
        "wav"
    } else if folder.join(format!("{stem}.opus")).exists() {
        "opus"
    } else {
        "wav"
    };
    format!("{stem}.{ext}")
}

/// `system.opus` + segment 0 → `system_seg00.opus` (the extension is kept).
pub fn scoped_channel_name(plain: &str, index: u32) -> String {
    match plain.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}_seg{index:02}.{ext}"),
        None => format!("{plain}_seg{index:02}"),
    }
}

/// At the final stop of a resumed meeting: join every segment's channel files, in segment
/// order, into the canonical `system.wav` / `mic.wav`. Best-effort, like before: a missing
/// segment file (diarization off for that session, or audio purged by the retention setting)
/// is skipped, and a failure is logged, never fatal to the stop.
pub fn concat_segment_channels(folder: &Path, segments: &[RecordingSegment]) {
    match find_ffmpeg_path() {
        Some(ffmpeg) => concat_segment_channels_with(&ffmpeg, folder, segments),
        None => warn!("FFmpeg not found; the channel segments were not joined"),
    }
}

/// [`concat_segment_channels`] with an explicit ffmpeg (tests use the bundled sidecar).
pub fn concat_segment_channels_with(ffmpeg: &Path, folder: &Path, segments: &[RecordingSegment]) {
    let names = |pick: fn(&RecordingSegment) -> &String| -> Vec<PathBuf> {
        segments
            .iter()
            .filter_map(|s| recorded_or_compressed(folder, pick(s)))
            .collect()
    };
    for (stem, inputs) in [
        ("system", names(|s| &s.system_wav)),
        ("mic", names(|s| &s.mic_wav)),
    ] {
        if let Err(e) = concat_channel(ffmpeg, folder, stem, &inputs) {
            warn!("Failed to concat the {stem} channel segments: {e:#}");
        }
    }
}

/// Decode `inputs` (WAV and/or Opus) into `folder/{stem}.wav` through a hidden temp, then
/// drop a `{stem}.opus` it supersedes (the previous canonical channel, compressed).
fn concat_channel(ffmpeg: &Path, folder: &Path, stem: &str, inputs: &[PathBuf]) -> Result<()> {
    if inputs.is_empty() {
        info!("No {stem} channel segments to join in {}", folder.display());
        return Ok(());
    }
    let out = folder.join(format!("{stem}.wav"));
    let tmp = folder.join(format!(".{stem}.concat.wav"));
    let labels: String = (0..inputs.len()).map(|i| format!("[n{i}]")).collect();
    let graph = format!(
        "{}{labels}concat=n={}:v=0:a=1[out]",
        normalize_inputs(inputs.len()),
        inputs.len()
    );
    if let Err(e) = run_filter(ffmpeg, inputs, &graph, &tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, &out).with_context(|| format!("move the joined {stem} channel"))?;
    let stale = folder.join(format!("{stem}.opus"));
    if !inputs.contains(&stale) && stale.is_file() {
        let _ = std::fs::remove_file(&stale);
    }
    info!(
        "Joined {} {stem} channel segment(s) into {}",
        inputs.len(),
        out.display()
    );
    Ok(())
}

/// Mix the microphone and system channels into one 16 kHz mono WAV for transcription, as a
/// hidden temp in `folder` that is deleted when the returned path is dropped. For a folder
/// that holds only channels (no mix): transcribing one side would lose half the meeting.
pub fn mix_channels_to_temp(ffmpeg: &Path, folder: &Path) -> Result<tempfile::TempPath> {
    let (Some(mic), Some(system)) = (mic_channel_path(folder), system_channel_path(folder)) else {
        bail!(
            "{} has no mixed recording and only one side of the conversation; \
             it can't be transcribed without losing the other side",
            folder.display()
        );
    };
    let tmp = tempfile::Builder::new()
        .prefix(".nixon_decode_mix_")
        .suffix(".wav")
        .tempfile_in(folder)
        .context("create the temporary mix")?
        .into_temp_path();
    let graph = format!(
        "{}[n0][n1]amix=inputs=2:duration=longest[out]",
        normalize_inputs(2)
    );
    run_filter(ffmpeg, &[mic, system], &graph, &tmp)?;
    Ok(tmp)
}

/// `[i:a]` → `[ni]` at 16 kHz mono for every input (Opus decodes at 48 kHz).
fn normalize_inputs(n: usize) -> String {
    (0..n)
        .map(|i| format!("[{i}:a]aresample=16000,aformat=channel_layouts=mono[n{i}];"))
        .collect()
}

fn run_filter(ffmpeg: &Path, inputs: &[PathBuf], graph: &str, out: &Path) -> Result<()> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    for input in inputs {
        cmd.arg("-i").arg(input);
    }
    let output = cmd
        .args(["-filter_complex", graph, "-map", "[out]"])
        .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", "-f", "wav"])
        .arg(out)
        .stdin(Stdio::null())
        .output()
        .context("could not run ffmpeg")?;
    if !output.status.success() {
        return Err(anyhow!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(folder: &Path, name: &str) {
        std::fs::write(folder.join(name), b"x").unwrap();
    }

    #[test]
    fn readers_prefer_the_wav_and_fall_back_to_the_opus() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(system_channel_path(d), None);
        touch(d, "system.opus");
        assert_eq!(system_channel_path(d), Some(d.join("system.opus")));
        touch(d, "system.wav");
        assert_eq!(system_channel_path(d), Some(d.join("system.wav")));
        assert_eq!(mic_channel_path(d), None, "a system file is not a mic file");
        touch(d, "mic_seg00.opus");
        assert_eq!(mic_channel_path(d), None, "a segment is not the channel");
    }

    #[test]
    fn segment_files_resolve_only_to_themselves_or_their_compressed_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        touch(d, "system.opus");
        assert_eq!(recorded_or_compressed(d, "system_seg00.wav"), None);
        touch(d, "system_seg00.opus");
        assert_eq!(
            recorded_or_compressed(d, "system_seg00.wav"),
            Some(d.join("system_seg00.opus"))
        );
        touch(d, "system_seg00.wav");
        assert_eq!(
            recorded_or_compressed(d, "system_seg00.wav"),
            Some(d.join("system_seg00.wav"))
        );
    }

    #[test]
    fn plain_names_follow_the_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(plain_channel_name(d, "mic"), "mic.wav");
        touch(d, "mic.opus");
        assert_eq!(plain_channel_name(d, "mic"), "mic.opus");
        assert_eq!(scoped_channel_name("mic.opus", 0), "mic_seg00.opus");
        assert_eq!(scoped_channel_name("system.wav", 12), "system_seg12.wav");
    }
}
