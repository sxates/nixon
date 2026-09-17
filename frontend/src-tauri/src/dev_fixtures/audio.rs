//! Renders fixture speech onto mic/system timelines and muxes `audio.mp4` (specs/0059).
use super::dataset::FixtureMeeting;
use super::wav::{place, read_pcm16_mono, say_to_wav, write_pcm16_mono, SAMPLE_RATE};
use crate::audio::channel_writer::{MIC_CHANNEL_FILENAME, SYSTEM_CHANNEL_FILENAME};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

const HASH_FILE: &str = ".fixture-hash";
const VOICES: [&str; 6] = ["Daniel", "Moira", "Rishi", "Karen", "Tessa", "Fred"];

/// Owner is always Samantha; remote keys cycle through VOICES by their numeric suffix.
pub fn voice_for(key: &str) -> &'static str {
    if key == "local" {
        return "Samantha";
    }
    let n: usize = key.trim_start_matches("spk_").parse().unwrap_or(0);
    VOICES[n % VOICES.len()]
}

fn fingerprint(m: &FixtureMeeting) -> String {
    let mut h = Sha256::new();
    h.update(m.id.as_bytes());
    h.update(m.duration_seconds.to_le_bytes());
    for s in &m.segments {
        h.update(s.text.as_bytes());
        h.update(s.start.to_le_bytes());
        h.update(s.speaker.as_deref().unwrap_or("").as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// Ok(true) = rendered or cache hit; Ok(false) = `say` unavailable (folder left without audio).
pub fn render_meeting_audio(m: &FixtureMeeting, folder: &Path) -> Result<bool> {
    let fp = fingerprint(m);
    if std::fs::read_to_string(folder.join(HASH_FILE))
        .map(|s| s.trim() == fp)
        .unwrap_or(false)
        && folder.join("audio.mp4").exists()
    {
        return Ok(true);
    }
    let total = (m.duration_seconds as usize + 1) * SAMPLE_RATE as usize;
    let mut mic = vec![0i16; total];
    let mut sys = vec![0i16; total];
    let tmp = tempfile::tempdir()?;
    for (i, seg) in m.segments.iter().enumerate() {
        let clip_path = tmp.path().join(format!("{i}.wav"));
        let key = seg.speaker.as_deref().unwrap_or("spk_0");
        if !say_to_wav(&seg.text, Some(voice_for(key)), &clip_path) {
            return Ok(false);
        }
        let mut clip = read_pcm16_mono(&clip_path)?;
        let max = ((seg.end - seg.start) * SAMPLE_RATE as f64) as usize;
        clip.truncate(max);
        if seg.channel == "microphone" {
            place(&mut mic, &clip, SAMPLE_RATE, seg.start);
        } else {
            place(&mut sys, &clip, SAMPLE_RATE, seg.start);
        }
    }
    write_pcm16_mono(&folder.join(MIC_CHANNEL_FILENAME), &mic, SAMPLE_RATE)?;
    write_pcm16_mono(&folder.join(SYSTEM_CHANNEL_FILENAME), &sys, SAMPLE_RATE)?;
    mux_mp4(folder)?;
    std::fs::write(folder.join(HASH_FILE), fp)?;
    Ok(true)
}

fn mux_mp4(folder: &Path) -> Result<()> {
    let ffmpeg = crate::audio::ffmpeg::find_ffmpeg_path().context("ffmpeg not found")?;
    let status = std::process::Command::new(ffmpeg)
        .args(["-y", "-loglevel", "error"])
        .arg("-i")
        .arg(folder.join(MIC_CHANNEL_FILENAME))
        .arg("-i")
        .arg(folder.join(SYSTEM_CHANNEL_FILENAME))
        .args([
            "-filter_complex",
            "[0:a][1:a]amix=inputs=2:normalize=0",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
        ])
        .arg(folder.join("audio.mp4"))
        .status()
        .context("run ffmpeg")?;
    anyhow::ensure!(status.success(), "ffmpeg amix failed: {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_fixtures::dataset::load_embedded;
    #[test]
    fn renders_wavs_and_mp4_then_caches() {
        if crate::audio::ffmpeg::find_ffmpeg_path().is_none() {
            eprintln!("ffmpeg missing; skipped");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ds = load_embedded().unwrap();
        let mut m = ds.meetings[3].clone(); // standup, shortest
        m.segments.truncate(6);
        m.duration_seconds = 40;
        if !render_meeting_audio(&m, dir.path()).unwrap() {
            eprintln!("say unavailable; skipped");
            return;
        }
        for f in ["mic.wav", "system.wav", "audio.mp4", ".fixture-hash"] {
            assert!(dir.path().join(f).exists(), "{f}");
        }
        let mtime = std::fs::metadata(dir.path().join("audio.mp4"))
            .unwrap()
            .modified()
            .unwrap();
        assert!(
            !render_meeting_audio(&m, dir.path()).unwrap_or(true)
                || std::fs::metadata(dir.path().join("audio.mp4"))
                    .unwrap()
                    .modified()
                    .unwrap()
                    == mtime,
            "second run must be a cache hit"
        );
    }
}
