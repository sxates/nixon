//! The audio a meeting is (re)transcribed from: its mix (specs/0072 W2, moved out of
//! `retranscription.rs`).
//!
//! The mix is `audio.mp4` (or an imported file). The per-channel files (`mic.*`,
//! `system.*`, their resume segments) are never picked on their own, since each holds only
//! one side of the conversation. A folder with no mix but both channels (a meeting recorded
//! with the old "delete immediately" setting and processed later) is mixed from them.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use super::channel_files::{mix_channels_to_temp, system_channel_path};
use super::constants::AUDIO_EXTENSIONS;
use super::lifecycle::state::is_channel_stem;

/// The file to decode, and the temporary mix behind it (deleted on drop) if there is one.
pub struct MeetingAudio {
    pub path: PathBuf,
    /// The file name to record in `metadata.json` (the system channel's for a temp mix).
    pub name: String,
    _mixed: Option<tempfile::TempPath>,
}

/// Find the meeting's mix in `folder`: common names first, then any other audio file that
/// isn't a channel file or a hidden temp, then a mix of the two channels.
pub fn find_audio_file(folder: &Path) -> Result<MeetingAudio> {
    find_audio_file_with(folder, super::ffmpeg::find_ffmpeg_path)
}

/// [`find_audio_file`] with the ffmpeg lookup injected (it's only needed to mix channels).
pub fn find_audio_file_with(
    folder: &Path,
    ffmpeg: impl FnOnce() -> Option<PathBuf>,
) -> Result<MeetingAudio> {
    let candidates = [
        "audio.mp4",
        "audio.m4a",
        "audio.wav",
        "audio.mp3",
        "audio.flac",
        "audio.ogg",
        "recording.mp4",
        "audio.mkv",
        "audio.webm",
        "audio.wma",
    ];
    let found = candidates
        .iter()
        .map(|name| folder.join(name))
        .find(|p| p.is_file())
        .or_else(|| other_mix(folder));
    if let Some(path) = found {
        let name = file_name(&path);
        return Ok(MeetingAudio {
            path,
            name,
            _mixed: None,
        });
    }

    if let Some(system) = system_channel_path(folder) {
        let ffmpeg = ffmpeg().ok_or_else(|| anyhow!("FFmpeg not found; can't mix the channels"))?;
        let mixed = mix_channels_to_temp(&ffmpeg, folder)?;
        return Ok(MeetingAudio {
            path: mixed.to_path_buf(),
            name: file_name(&system),
            _mixed: Some(mixed),
        });
    }
    Err(anyhow!("No audio file found in: {}", folder.display()))
}

/// Any other audio file (sorted, so the choice is stable), excluding channel files and
/// hidden temps.
fn other_mix(folder: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            AUDIO_EXTENSIONS.contains(&ext.as_str())
                && !stem.starts_with('.')
                && !is_channel_stem(&stem)
        })
        .collect();
    files.sort();
    files.into_iter().next()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(folder: &Path) -> Result<MeetingAudio> {
        find_audio_file_with(folder, || None)
    }

    #[test]
    fn common_names_come_first_in_order() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find(dir.path()).is_err());
        std::fs::write(dir.path().join("audio.m4a"), b"fake").unwrap();
        assert_eq!(find(dir.path()).unwrap().name, "audio.m4a");
        std::fs::write(dir.path().join("audio.mp4"), b"fake").unwrap();
        assert_eq!(find(dir.path()).unwrap().name, "audio.mp4");
    }

    #[test]
    fn the_fallback_scan_finds_an_imported_file_but_never_a_channel() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for f in [
            "mic.wav",
            "system.opus",
            "mic_seg00.wav",
            ".nixon_decode_x.wav",
        ] {
            std::fs::write(d.join(f), b"fake").unwrap();
        }
        std::fs::write(d.join("notes.txt"), b"text").unwrap();
        std::fs::write(d.join("my_recording.flac"), b"fake").unwrap();
        assert_eq!(find(d).unwrap().name, "my_recording.flac");
    }

    #[test]
    fn a_channels_only_folder_is_never_transcribed_from_one_side() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("mic.wav"), b"fake").unwrap();
        let err = find(d)
            .err()
            .expect("a lone mic channel is not the meeting");
        assert!(format!("{err:#}").contains("No audio file"), "{err:#}");
        std::fs::write(d.join("system.wav"), b"fake").unwrap();
        let err = find(d).err().expect("both channels need ffmpeg to mix");
        assert!(format!("{err:#}").contains("FFmpeg"), "{err:#}");
    }

    #[test]
    fn every_supported_format_is_listed() {
        let listed = [
            "mp4", "m4a", "wav", "mp3", "flac", "ogg", "aac", "mkv", "webm", "wma",
        ];
        for ext in listed.iter().chain(&["opus"]) {
            assert!(AUDIO_EXTENSIONS.contains(ext), "{ext}");
        }
        assert!(!AUDIO_EXTENSIONS.contains(&"txt") && !AUDIO_EXTENSIONS.contains(&"pdf"));
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        assert!(find(Path::new("/nonexistent/path/12345")).is_err());
    }
}
