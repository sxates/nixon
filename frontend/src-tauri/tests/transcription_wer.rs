//! Word-error-rate comparison: Parakeet vs Whisper on real meeting audio (specs/0067).
//!
//! Settings offers two engines and thirteen models between them, which is not a choice a
//! normal user can make. Before collapsing that to one default we need evidence, and the
//! repo had none: `diarization_tuning.rs` scores DER (who spoke when), never WER (what was
//! said). This is the missing half.
//!
//! **What the reference actually is.** The corpus is the same `zoom-samples/` set the DER
//! harness uses, and the reference is Zoom's own `.transcript.vtt`. Zoom's transcript is
//! itself ASR output, not a human gold transcript, so a number here is *agreement with
//! Zoom*, not accuracy against truth. That is still the right shape for this decision —
//! we are ranking two engines against a common third opinion, and a large gap between them
//! is meaningful — but no single figure should be quoted as "Nixon's WER".
//!
//! Run:
//!   cargo test --features metal --test transcription_wer -- --nocapture --ignored
//!
//! Env:
//!   NIXON_DIARIZATION_EVAL_DIR  corpus dir (default: `zoom-samples/` beside the repo)
//!   NIXON_WER_SECONDS           audio per sample, seconds (default 300; 0 = whole file)
//!   NIXON_WER_WHISPER_MODEL     catalogue name (default: whatever is installed)
//!   NIXON_WER_CODEC             specs/0072 W0: `aac64`, `aac96`, … round-trips each
//!                               sample through that codec first (the recorded mix's
//!                               bitrate); unset = baseline. See `tests/eval_codec/`.

use std::path::{Path, PathBuf};

use app_lib::audio::decoder::decode_audio_file;
use app_lib::parakeet_engine::ParakeetEngine;
use app_lib::whisper_engine::WhisperEngine;

mod eval_codec;

const SAMPLE_RATE: usize = 16_000;

/// Audio handed to an engine in one call, in seconds.
///
/// **Not cosmetic.** The first run of this harness fed each engine one 5-minute buffer and
/// reported Whisper at ~100% WER, which is not a model failing — it is a harness feeding an
/// engine something the app never feeds it. Nixon transcribes *VAD segments*: the live path
/// sends a few seconds at a time and import segments the file first. Whisper's params here
/// set `no_timestamps(true)` precisely because the app owns the segmentation, so a long
/// blob loses most of its content. Both engines get the same 30-second windows, which is
/// the closest fair analogue of production.
const CHUNK_SECS: usize = 30;

// ---------------------------------------------------------------------------
// Corpus discovery (mirrors diarization_tuning.rs)
// ---------------------------------------------------------------------------

fn eval_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("NIXON_DIARIZATION_EVAL_DIR") {
        let p = PathBuf::from(dir);
        return p.is_dir().then_some(p);
    }
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    for candidate in [
        repo_root.join("..").join("zoom-samples"),
        repo_root.join("..").join("..").join("zoom-samples"),
    ] {
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

/// One sample: the media file and its Zoom VTT.
struct Sample {
    name: String,
    media: PathBuf,
    vtt: PathBuf,
}

fn samples(dir: &Path) -> Vec<Sample> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let files: Vec<PathBuf> = std::fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        let media = files
            .iter()
            .find(|p| p.extension().is_some_and(|e| e == "mp4"))
            .cloned();
        let vtt = files
            .iter()
            .find(|p| p.extension().is_some_and(|e| e == "vtt"))
            .cloned();
        if let (Some(media), Some(vtt)) = (media, vtt) {
            found.push(Sample {
                name: path.file_name().unwrap().to_string_lossy().to_string(),
                media,
                vtt,
            });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

// ---------------------------------------------------------------------------
// Reference text
// ---------------------------------------------------------------------------

fn parse_vtt_ts(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.replace(',', ".").parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Reference words spoken before `limit_secs` (0 = no limit). Zoom cues are
/// `start --> end` followed by `Speaker Name: text`; the speaker prefix is dropped.
fn reference_text(vtt: &Path, limit_secs: f64) -> String {
    let content = std::fs::read_to_string(vtt).expect("read vtt");
    let mut out: Vec<String> = Vec::new();
    let mut take = false;
    for line in content.lines() {
        if let Some((start, _)) = line.split_once("-->") {
            take = match parse_vtt_ts(start) {
                Some(t) => limit_secs <= 0.0 || t < limit_secs,
                None => false,
            };
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "WEBVTT" || trimmed.parse::<u32>().is_ok() {
            continue;
        }
        if take {
            let text = trimmed.split_once(':').map(|(_, t)| t).unwrap_or(trimmed);
            out.push(text.trim().to_string());
        }
    }
    out.join(" ")
}

// ---------------------------------------------------------------------------
// WER
// ---------------------------------------------------------------------------

/// Lowercase, strip everything but letters/digits/apostrophes, collapse whitespace.
/// Both sides get the same treatment, so punctuation and casing — which the engines
/// format differently and which nobody reads a transcript for — cannot skew the result.
fn normalize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Levenshtein over words → (WER, substitutions+deletions+insertions, reference length).
fn wer(reference: &[String], hypothesis: &[String]) -> (f64, usize, usize) {
    let n = reference.len();
    let m = hypothesis.len();
    if n == 0 {
        return (f64::NAN, 0, 0);
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = usize::from(reference[i - 1] != hypothesis[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let dist = prev[m];
    (dist as f64 / n as f64, dist, n)
}

// ---------------------------------------------------------------------------
// Engines
// ---------------------------------------------------------------------------

fn models_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("NIXON_TEST_MODELS_DIR") {
        return Some(PathBuf::from(dir));
    }
    let data = dirs::data_dir()?;
    ["ai.vinyl.app", "ai.vinyl.app.debug"]
        .iter()
        .map(|id| data.join(id).join("models"))
        .find(|p| p.is_dir())
}

/// The installed Whisper model's catalogue name (filename minus `ggml-` and `.bin`).
fn installed_whisper_model(root: &Path) -> Option<String> {
    if let Ok(name) = std::env::var("NIXON_WER_WHISPER_MODEL") {
        return Some(name);
    }
    let mut found: Vec<(u64, String)> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            (name.starts_with("ggml-") && name.ends_with(".bin"))
                .then(|| (e.metadata().ok().map(|m| m.len()).unwrap_or(0), name))
        })
        .collect();
    // Prefer the LARGEST installed model: this comparison should put Whisper's best
    // installed foot forward, or a win for Parakeet proves nothing.
    found.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
    found.into_iter().next().map(|(_, name)| {
        name.trim_start_matches("ggml-")
            .trim_end_matches(".bin")
            .to_string()
    })
}

fn clip(samples: Vec<f32>, limit_secs: f64) -> Vec<f32> {
    if limit_secs <= 0.0 {
        return samples;
    }
    let max = (limit_secs * SAMPLE_RATE as f64) as usize;
    samples.into_iter().take(max).collect()
}

#[tokio::test]
#[ignore = "eval harness: needs the zoom-samples corpus and both engines' models"]
async fn parakeet_vs_whisper_word_error_rate() {
    let Some(dir) = eval_dir() else {
        eprintln!(
            "SKIP: no corpus. Set NIXON_DIARIZATION_EVAL_DIR or put zoom-samples/ beside the repo."
        );
        return;
    };
    let Some(root) = models_root() else {
        eprintln!("SKIP: no models directory found.");
        return;
    };
    let limit_secs: f64 = std::env::var("NIXON_WER_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300.0);

    let whisper_model = installed_whisper_model(&root);
    let samples = samples(&dir);
    eprintln!(
        "corpus: {} ({} samples) · models: {} · window: {}",
        dir.display(),
        samples.len(),
        root.display(),
        if limit_secs <= 0.0 {
            "whole file".to_string()
        } else {
            format!("first {limit_secs:.0}s")
        }
    );

    let parakeet =
        ParakeetEngine::new_with_models_dir(Some(root.clone())).expect("parakeet engine");
    parakeet.discover_models().await.expect("discover parakeet");
    let parakeet_model = app_lib::config::DEFAULT_PARAKEET_MODEL;
    let parakeet_ok = parakeet.load_model(parakeet_model).await.is_ok();
    if !parakeet_ok {
        eprintln!("NOTE: parakeet model '{parakeet_model}' not loadable — skipping that column");
    }

    let whisper = WhisperEngine::new_with_models_dir(Some(root.clone())).expect("whisper engine");
    whisper.discover_models().await.ok();
    let whisper_ok = match &whisper_model {
        Some(name) => whisper.load_model(name).await.is_ok(),
        None => false,
    };
    if !whisper_ok {
        eprintln!("NOTE: no Whisper model loadable — skipping that column");
    }

    println!(
        "\n{:<30} {:>9} {:>7} {:>9} {:>7}",
        "sample", "parakeet", "xRT", "whisper", "xRT"
    );
    println!("{}", "-".repeat(66));

    let mut totals = [(0usize, 0usize); 2]; // (errors, ref words) for parakeet, whisper
                                            // Wall-clock per engine, against audio seconds processed, for a real-time factor.
                                            // For live transcription this matters at least as much as WER: an engine slower than
                                            // real time cannot keep up with a meeting at all.
    let mut spent = [0f64; 2];
    let mut audio_secs_total = 0f64;
    let codec = eval_codec::Codec::from_env("NIXON_WER_CODEC");
    if let Some(codec) = codec {
        eprintln!("codec round-trip: {}", codec.tag());
    }
    for sample in &samples {
        let media = match codec {
            None => sample.media.clone(),
            Some(codec) => {
                let ffmpeg = eval_codec::bundled_ffmpeg().expect("bundled ffmpeg sidecar");
                let stem = sample.media.file_stem().unwrap().to_string_lossy();
                let cache = sample.media.parent().unwrap().join(".eval-cache");
                eval_codec::round_trip_cached(&ffmpeg, codec, &sample.media, &cache, &stem)
                    .unwrap_or_else(|e| panic!("{} round-trip failed: {e}", codec.tag()))
            }
        };
        let decoded = match decode_audio_file(&media) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  {}: decode failed ({e})", sample.name);
                continue;
            }
        };
        let audio = clip(decoded.to_whisper_format(), limit_secs);
        let reference = normalize(&reference_text(&sample.vtt, limit_secs));
        if reference.is_empty() {
            eprintln!("  {}: empty reference window, skipped", sample.name);
            continue;
        }

        // A WER near 100% means "no overlap", which in practice means the harness got
        // nothing usable back — not that the engine cannot transcribe. Print what each
        // engine actually returned so that case is diagnosable instead of believable.
        let debug = std::env::var("NIXON_WER_DEBUG").is_ok();
        let show = |who: &str, text: &str, words: usize| {
            if debug {
                eprintln!(
                    "    [{who}] {} words; head: {:?}",
                    words,
                    text.chars().take(160).collect::<String>()
                );
            }
        };

        let chunks: Vec<Vec<f32>> = audio
            .chunks(CHUNK_SECS * SAMPLE_RATE)
            .map(<[f32]>::to_vec)
            .collect();

        audio_secs_total += audio.len() as f64 / SAMPLE_RATE as f64;
        let mut row = [f64::NAN; 2];
        let mut elapsed = [0f64; 2];
        if parakeet_ok {
            let started = std::time::Instant::now();
            let mut parts = Vec::new();
            for chunk in &chunks {
                match parakeet.transcribe_audio(chunk.clone()).await {
                    Ok(text) => parts.push(text),
                    Err(e) => eprintln!("    [parakeet] chunk FAILED: {e}"),
                }
            }
            elapsed[0] = started.elapsed().as_secs_f64();
            spent[0] += elapsed[0];
            let text = parts.join(" ");
            let hyp = normalize(&text);
            show("parakeet", &text, hyp.len());
            let (rate, errors, refs) = wer(&reference, &hyp);
            row[0] = rate;
            totals[0].0 += errors;
            totals[0].1 += refs;
        }
        if whisper_ok {
            let started = std::time::Instant::now();
            let mut parts = Vec::new();
            for chunk in &chunks {
                match whisper
                    .transcribe_audio(chunk.clone(), Some("en".to_string()))
                    .await
                {
                    Ok(text) => parts.push(text),
                    Err(e) => eprintln!("    [whisper] chunk FAILED: {e}"),
                }
            }
            elapsed[1] = started.elapsed().as_secs_f64();
            spent[1] += elapsed[1];
            let text = parts.join(" ");
            let hyp = normalize(&text);
            show("whisper", &text, hyp.len());
            let (rate, errors, refs) = wer(&reference, &hyp);
            row[1] = rate;
            totals[1].0 += errors;
            totals[1].1 += refs;
        }
        if debug {
            eprintln!("    [reference] {} words", reference.len());
        }
        let audio_secs = audio.len() as f64 / SAMPLE_RATE as f64;
        let xrt = |secs: f64| {
            if secs > 0.0 {
                audio_secs / secs
            } else {
                f64::NAN
            }
        };
        println!(
            "{:<30} {:>8.1}% {:>6.1}x {:>8.1}% {:>6.1}x",
            sample.name,
            row[0] * 100.0,
            xrt(elapsed[0]),
            row[1] * 100.0,
            xrt(elapsed[1])
        );
    }

    println!("{}", "-".repeat(66));
    let pooled = |t: (usize, usize)| {
        if t.1 == 0 {
            f64::NAN
        } else {
            t.0 as f64 / t.1 as f64 * 100.0
        }
    };
    let xrt_total = |secs: f64| {
        if secs > 0.0 {
            audio_secs_total / secs
        } else {
            f64::NAN
        }
    };
    println!(
        "{:<30} {:>8.1}% {:>6.1}x {:>8.1}% {:>6.1}x",
        "POOLED",
        pooled(totals[0]),
        xrt_total(spent[0]),
        pooled(totals[1]),
        xrt_total(spent[1])
    );
    println!(
        "\nxRT = audio seconds per wall-clock second (higher is faster; <1 cannot keep up live)."
    );
    println!(
        "\nwhisper model: {}\nreference: Zoom's own VTT (ASR, not a human transcript) — these are\nagreement scores between two engines and a third opinion, not absolute accuracy.\n",
        whisper_model.as_deref().unwrap_or("none installed")
    );
}
