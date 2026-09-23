//! specs/0072 W0 — codec round-trips for the eval harnesses.
//!
//! Kept audio is re-encoded after processing (channels to Opus, the mix to low-bitrate
//! AAC). Before choosing a bitrate we measure what that lossy pass costs: the DER harness
//! (`diarization_tuning.rs`, `NIXON_EVAL_CODEC`) and the WER harness
//! (`transcription_wer.rs`, `NIXON_WER_CODEC`) route their input audio through the codec
//! with the bundled ffmpeg and score the decoded result instead of the original.
//!
//! Codec specs: `opus<kbps>` (Ogg/Opus, libopus `-application audio`, the production
//! channel encoder), `aac<kbps>` (AAC-LC in `.m4a`, 48 kHz mono like the recorded mix) and
//! `flac` (lossless control). Every round-trip decodes back to 16 kHz mono s16 WAV, which
//! is what the models read.

#![allow(dead_code)] // each harness uses a different half of this module

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A parsed codec spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Opus { kbps: u32 },
    Aac { kbps: u32 },
    Flac,
}

impl Codec {
    /// Parse `opus24`, `aac64`, `flac`, … (case-insensitive). `None` for anything else.
    pub fn parse(spec: &str) -> Option<Codec> {
        let spec = spec.trim().to_ascii_lowercase();
        if spec == "flac" {
            return Some(Codec::Flac);
        }
        let kbps = |rest: &str| rest.parse::<u32>().ok().filter(|k| (6..=512).contains(k));
        if let Some(rest) = spec.strip_prefix("opus") {
            return kbps(rest).map(|kbps| Codec::Opus { kbps });
        }
        if let Some(rest) = spec.strip_prefix("aac") {
            return kbps(rest).map(|kbps| Codec::Aac { kbps });
        }
        None
    }

    /// Read a codec spec from `var`. Unset/empty = `None` (baseline, no round-trip); an
    /// unparseable value panics so a typo can't silently produce a baseline run.
    pub fn from_env(var: &str) -> Option<Codec> {
        let raw = std::env::var(var).ok().filter(|v| !v.trim().is_empty())?;
        Some(
            Codec::parse(&raw)
                .unwrap_or_else(|| panic!("{var}={raw:?}: expected opus<kbps>, aac<kbps> or flac")),
        )
    }

    /// Stable cache tag, e.g. `opus24`.
    pub fn tag(self) -> String {
        match self {
            Codec::Opus { kbps } => format!("opus{kbps}"),
            Codec::Aac { kbps } => format!("aac{kbps}"),
            Codec::Flac => "flac".to_string(),
        }
    }

    /// Container extension of the encoded intermediate.
    pub fn extension(self) -> &'static str {
        match self {
            Codec::Opus { .. } => "opus",
            Codec::Aac { .. } => "m4a",
            Codec::Flac => "flac",
        }
    }

    /// ffmpeg output arguments for the encode (after `-i <input>`).
    pub fn encode_args(self) -> Vec<String> {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        match self {
            // Same flags as the production channel compressor (specs/0072 Design).
            Codec::Opus { kbps } => {
                let mut a = s(&["-vn", "-ac", "1", "-c:a", "libopus", "-b:a"]);
                a.push(format!("{kbps}k"));
                a.extend(s(&["-application", "audio"]));
                a
            }
            // The recorded mix is 48 kHz mono AAC-LC (audio/encode.rs).
            Codec::Aac { kbps } => {
                let mut a = s(&["-vn", "-ac", "1", "-ar", "48000", "-c:a", "aac", "-b:a"]);
                a.push(format!("{kbps}k"));
                a
            }
            Codec::Flac => s(&["-vn", "-ac", "1", "-c:a", "flac"]),
        }
    }
}

fn run_ffmpeg(ffmpeg: &Path, input: &Path, args: &[String], out: &Path) -> Result<(), String> {
    let output = Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args(args)
        .arg(out)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn ffmpeg: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Round-trip `input` through `codec` into `{cache_dir}/{stem}.{codec}.wav` (16 kHz mono
/// s16), keeping the encoded intermediate beside it as `{stem}.{codec}.{ext}` so its size
/// can be inspected. Cached: an existing output is returned as-is. Temp names are renamed
/// into place only on success, so an interrupted run never leaves a truncated cache hit.
pub fn round_trip_cached(
    ffmpeg: &Path,
    codec: Codec,
    input: &Path,
    cache_dir: &Path,
    stem: &str,
) -> Result<PathBuf, String> {
    let tag = codec.tag();
    let wav = cache_dir.join(format!("{stem}.{tag}.wav"));
    if wav.exists() {
        return Ok(wav);
    }
    std::fs::create_dir_all(cache_dir).map_err(|e| format!("create cache dir: {e}"))?;
    let encoded = cache_dir.join(format!("{stem}.{tag}.{}", codec.extension()));
    let enc_tmp = cache_dir.join(format!(".tmp.{stem}.{tag}.{}", codec.extension()));
    let wav_tmp = cache_dir.join(format!(".tmp.{stem}.{tag}.wav"));
    run_ffmpeg(ffmpeg, input, &codec.encode_args(), &enc_tmp)?;
    std::fs::rename(&enc_tmp, &encoded).map_err(|e| format!("rename encoded: {e}"))?;
    let decode = ["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"].map(String::from);
    run_ffmpeg(ffmpeg, &encoded, &decode, &wav_tmp)?;
    // A short decode would be scored against the full reference and look like a codec
    // failure (or, cached, poison every later run), so it never becomes a cache hit.
    let (want, got) = (
        probe_duration(ffmpeg, input),
        probe_duration(ffmpeg, &wav_tmp),
    );
    match (want, got) {
        (Some(want), Some(got)) if (want - got).abs() <= want * 0.01 + 0.1 => {}
        _ => {
            let _ = std::fs::remove_file(&wav_tmp);
            return Err(format!("round-trip duration {got:?}s != input {want:?}s"));
        }
    }
    std::fs::rename(&wav_tmp, &wav).map_err(|e| format!("rename wav: {e}"))?;
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "  codec {tag}: encoded {:.1} MB ({:.1}% of the input), decoded -> {}",
        size(&encoded) as f64 / 1e6,
        size(&encoded) as f64 * 100.0 / size(input).max(1) as f64,
        wav.display()
    );
    Ok(wav)
}

/// Container duration in seconds, parsed from ffmpeg's `Duration: HH:MM:SS.xx` banner.
fn probe_duration(ffmpeg: &Path, path: &Path) -> Option<f64> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let ts = text.split("Duration: ").nth(1)?.split(',').next()?.trim();
    let mut parts = ts.split(':').map(|p| p.parse::<f64>().ok());
    let (h, m, sec) = (parts.next()??, parts.next()??, parts.next()??);
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// The repo's bundled ffmpeg sidecar (what the app ships), else `None`.
pub fn bundled_ffmpeg() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("ffmpeg-"))
        })
}

#[test]
fn codec_spec_parses_ladder_and_rejects_junk() {
    assert_eq!(Codec::parse("opus24"), Some(Codec::Opus { kbps: 24 }));
    assert_eq!(Codec::parse(" OPUS48 "), Some(Codec::Opus { kbps: 48 }));
    assert_eq!(Codec::parse("aac64"), Some(Codec::Aac { kbps: 64 }));
    assert_eq!(Codec::parse("flac"), Some(Codec::Flac));
    for junk in ["opus", "opusX", "aac", "mp3", "opus0", "flac24", ""] {
        assert_eq!(Codec::parse(junk), None, "{junk:?} must not parse");
    }
    assert_eq!(Codec::Opus { kbps: 32 }.tag(), "opus32");
    assert!(Codec::Opus { kbps: 24 }
        .encode_args()
        .windows(2)
        .any(|w| w[0] == "-b:a" && w[1] == "24k"));
}

/// A real encode → decode through the bundled ffmpeg keeps the sample count (the eval
/// would otherwise score time-shifted audio against the VTT). Skips without the sidecar.
#[test]
fn round_trip_preserves_duration() {
    let Some(ffmpeg) = bundled_ffmpeg() else {
        eprintln!("SKIP: no bundled ffmpeg sidecar under binaries/");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("tone.wav");
    // 3 s of 440 Hz at 16 kHz mono s16, synthesized by ffmpeg itself.
    let tone = ["-f", "lavfi"].map(String::from);
    let status = Command::new(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(&tone)
        .args(["-i", "sine=frequency=440:sample_rate=16000:duration=3"])
        .args(["-ac", "1", "-c:a", "pcm_s16le"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success(), "synthesize tone");
    for codec in [
        Codec::Opus { kbps: 24 },
        Codec::Aac { kbps: 64 },
        Codec::Flac,
    ] {
        let out = round_trip_cached(&ffmpeg, codec, &src, dir.path(), "tone").unwrap();
        assert!(out.exists());
        assert!(dir
            .path()
            .join(format!("tone.{}.{}", codec.tag(), codec.extension()))
            .exists());
        let n = app_lib::audio::decoder::decode_audio_file(&out)
            .unwrap()
            .to_whisper_format()
            .len() as i64;
        assert!(
            (n - 48_000).abs() <= 480,
            "{codec:?}: {n} samples, want ~48000"
        );
    }
    let secs = probe_duration(&ffmpeg, &src).expect("probe");
    assert!((secs - 3.0).abs() < 0.05, "probed {secs}s, want 3s");
}
