//! Renders fixture speech onto mic/system timelines and muxes `audio.mp4` (specs/0059).
use super::dataset::FixtureMeeting;
use super::wav::{place, read_pcm16_mono, say_to_wav, write_pcm16_mono, SAMPLE_RATE};
use crate::audio::channel_writer::{MIC_CHANNEL_FILENAME, SYSTEM_CHANNEL_FILENAME};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

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

/// Synthesizes every segment's clip in parallel onto `<tmp>/<i>.wav` using a bounded
/// worker pool (segment indices are handed out from a shared atomic counter so workers
/// never contend over the same segment). Returns `None` (after logging which segment(s)
/// failed) when any `say` call fails, so the caller can preserve the existing
/// `Ok(false)` "say unavailable" contract; otherwise returns each clip's path indexed
/// by segment position, ready for the same sequential timeline placement as before.
fn synth_segments_parallel(m: &FixtureMeeting, tmp: &Path) -> Option<Vec<PathBuf>> {
    let n = m.segments.len();
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);
    let next = AtomicUsize::new(0);
    let ok = std::sync::atomic::AtomicBool::new(true);
    let results: Mutex<Vec<Option<PathBuf>>> = Mutex::new(vec![None; n]);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= n {
                    break;
                }
                let seg = &m.segments[i];
                let clip_path = tmp.join(format!("{i}.wav"));
                let key = seg.speaker.as_deref().unwrap_or("spk_0");
                if say_to_wav(&seg.text, Some(voice_for(key)), &clip_path) {
                    results.lock().unwrap()[i] = Some(clip_path);
                } else {
                    log::warn!("[dev] say failed for segment {i} ({}) of {}", seg.id, m.id);
                    ok.store(false, Ordering::SeqCst);
                }
            });
        }
    });
    if !ok.load(Ordering::SeqCst) {
        return None;
    }
    results.into_inner().unwrap().into_iter().collect()
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
    let clip_paths = match synth_segments_parallel(m, tmp.path()) {
        Some(paths) => paths,
        None => return Ok(false),
    };
    for (i, seg) in m.segments.iter().enumerate() {
        let mut clip = read_pcm16_mono(&clip_paths[i])?;
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

    #[test]
    fn parallel_synthesis_produces_correct_length_and_places_owner_audio() {
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
        let mic = read_pcm16_mono(&dir.path().join("mic.wav")).unwrap();
        assert_eq!(
            mic.len(),
            (m.duration_seconds as usize + 1) * SAMPLE_RATE as usize
        );
        // seg_1 ("Morning, everyone.") is the owner's first, microphone-channel segment,
        // starting at t=0 — the parallel path must place it at the same offset the
        // sequential path always did.
        let owner_seg = m
            .segments
            .iter()
            .find(|s| s.channel == "microphone")
            .expect("standup meeting has an owner/microphone segment");
        let off = (owner_seg.start * SAMPLE_RATE as f64) as usize;
        assert!(
            mic[off..off + 100].iter().any(|&s| s != 0),
            "expected non-silent audio at the owner segment's offset"
        );
    }
}
