//! Minimal 16-bit PCM WAV I/O for fixture audio (hound is not a dependency by choice).
use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

pub const SAMPLE_RATE: u32 = 16_000;

pub fn write_pcm16_mono(path: &Path, samples: &[i16], sample_rate: u32) -> Result<()> {
    let data_bytes = (samples.len() * 2) as u32;
    let mut f =
        std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_bytes).to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&(sample_rate * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_bytes.to_le_bytes())?;
    let mut buf = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    f.write_all(&buf)?;
    Ok(())
}

/// Reads a canonical 44-byte-header PCM16 mono WAV (what `say` and `write_pcm16_mono` emit).
pub fn read_pcm16_mono(path: &Path) -> Result<Vec<i16>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file: {}", path.display());
    }
    // walk chunks to the data chunk (say may emit extra chunks)
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if id == b"data" {
            let end = (pos + 8 + len).min(bytes.len());
            return Ok(bytes[pos + 8..end]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect());
        }
        pos += 8 + len + (len & 1);
    }
    bail!("no data chunk in {}", path.display())
}

/// Additively place `clip` onto `timeline` starting at `at_seconds` (saturating add).
pub fn place(timeline: &mut [i16], clip: &[i16], sample_rate: u32, at_seconds: f64) {
    let off = (at_seconds * sample_rate as f64) as usize;
    for (i, s) in clip.iter().enumerate() {
        if let Some(t) = timeline.get_mut(off + i) {
            *t = t.saturating_add(*s);
        }
    }
}

/// `say` → 16 kHz mono PCM16. Returns false when `say` is unavailable or fails.
pub fn say_to_wav(text: &str, voice: Option<&str>, out: &Path) -> bool {
    let mut cmd = Command::new("say");
    cmd.arg("-o")
        .arg(out)
        .arg("--data-format=LEI16@16000")
        .arg("--channels=1");
    if let Some(v) = voice {
        cmd.arg("-v").arg(v);
    }
    cmd.arg(text);
    match cmd.status() {
        Ok(s) if s.success() && out.exists() => true,
        Ok(s) => {
            log::warn!("say exited {s}");
            false
        }
        Err(e) => {
            log::warn!("say unavailable: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm16_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        let samples: Vec<i16> = (0..1600).map(|i| ((i % 100) as i16 - 50) * 100).collect();
        write_pcm16_mono(&p, &samples, 16_000).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 44 + 1600 * 2);
        assert_eq!(read_pcm16_mono(&p).unwrap(), samples);
    }
    #[test]
    fn place_on_timeline_mixes_at_offset() {
        let mut timeline = vec![0i16; 16_000 * 2];
        place(&mut timeline, &[1000, 1000, 1000], 16_000, 1.0);
        assert_eq!(timeline[16_000], 1000);
        assert_eq!(timeline[15_999], 0);
    }
    #[test]
    fn say_produces_16k_mono_or_skips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.wav");
        if !say_to_wav("testing one two", None, &p) {
            eprintln!("say unavailable; skipped");
            return;
        }
        let s = read_pcm16_mono(&p).unwrap();
        assert!(s.len() > 8_000, "got {} samples", s.len());
    }
}
