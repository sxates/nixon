//! Real-file diarization tuning harness (specs/0011 P3-0 accuracy gate).
//!
//! This is a **dev/diagnostic** harness, NOT part of the normal test suite. Every
//! test here is `#[ignore]` so a bare `cargo test` skips them — they load a large
//! real recording from the recordings folder (`~/Movies/nixon-recordings/`, or the
//! legacy `~/Movies/meetily-recordings/` on a pre-0057 install) and run the (slow) sherpa
//! offline diarizer, which is unsuitable for CI and unavailable on machines
//! without the recording + downloaded models.
//!
//! ## What it does
//!
//! Loads a real ~90-min, ~10-speaker `system.wav`, optionally truncates it to a
//! representative slice for fast iteration, and runs [`SherpaDiarizer`] with
//! varying `threshold` / `min_duration_*` / cluster-count, printing the resulting
//! **distinct speaker count**. This is how the tuned default in
//! `sherpa.rs::DEFAULT_AUTO_THRESHOLD` (and the `expected_speaker_count` override
//! recommendation) were chosen.
//!
//! ## Run it
//!
//! ```text
//! # Default file resolution is the recordings under ~/Movies/nixon-recordings
//! # (falling back to the legacy ~/Movies/meetily-recordings); override with env vars.
//! export NIXON_TUNE_WAV="$HOME/Movies/nixon-recordings/<folder>/system.wav"
//! # Tune on a fast slice (seconds). 0 / unset = whole file.
//! export NIXON_TUNE_SLICE_SECS=900
//!
//! # The threshold sweep (fast, on the slice):
//! cargo test --features metal --test diarization_tuning sweep_threshold -- --ignored --nocapture
//!
//! # Validate the shipped default + Fixed(10) on the WHOLE file:
//! cargo test --features metal --test diarization_tuning validate_full_file -- --ignored --nocapture
//! ```
//!
//! ## Provider note (important for the multi-config sweeps)
//!
//! Force CPU for the sweeps: `NIXON_DIARIZATION_PROVIDER=cpu`. The sweeps build &
//! tear down many ONNX sessions in a tight loop; the CoreML EP's destructor aborts
//! the process on rapid teardown (a temp-`.mlmodelc` cleanup permission error — a
//! harness artifact, not a product bug; the real app builds one diarizer per pass).
//! CPU is slower (~75 s/pass on a 12-min slice) but stable and gives the same
//! cluster counts (clustering is provider-independent). CoreML (the macOS default)
//! is fine for a *single* pass.

use std::path::{Path, PathBuf};
use std::time::Instant;

use app_lib::audio::decoder::decode_audio_file;
use app_lib::diarization::seed::{estimate_speakers_by_duration, N_AUDIO_MIN_SECS};
use app_lib::diarization::sherpa::{
    consolidate_clusters, merge_clusters_to_at_most, ConsolidateOpts, DiarizeTuning, SherpaDiarizer,
    SpeakerCount, CONSOLIDATE_FLOOR, DEFAULT_AUTO_THRESHOLD,
};
use app_lib::diarization::Diarizer;

const SAMPLE_RATE: u32 = 16_000;

const SEGMENTATION_FILE: &str = "segmentation.onnx";
const EMBEDDING_FILE: &str = "nemo_en_titanet_large.onnx";

/// Resolve the two diarization model paths across dev/prod/upstream identifier
/// dirs (the test binary has no Tauri bundle id, so `app_paths` can't find them).
/// `NIXON_TEST_DIARIZATION_DIR` overrides. Returns `None` (skip) if not found.
fn resolve_model_dir() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("NIXON_TEST_DIARIZATION_DIR") {
        roots.push(PathBuf::from(dir));
    }
    if let Some(data) = dirs::data_dir() {
        for id in [
            "ai.vinyl.app",
            "ai.vinyl.app.debug",
            "com.meetily.ai",
            "Nixon",
        ] {
            roots.push(data.join(id).join("models").join("diarization"));
        }
    }
    roots
        .into_iter()
        .find(|d| d.join(SEGMENTATION_FILE).exists() && d.join(EMBEDDING_FILE).exists())
}

/// The two real recordings the user captured (~10 speakers each). The first that
/// exists is used unless `NIXON_TUNE_WAV` overrides.
///
/// Both recordings roots are searched: `nixon-recordings` (fresh installs since
/// specs/0057) first, then the legacy `meetily-recordings` that a pre-rebrand
/// install — including the owner's — still writes to.
fn default_candidate_wavs() -> Vec<PathBuf> {
    let movies = dirs::home_dir().unwrap_or_default().join("Movies");
    let roots = ["nixon-recordings", "meetily-recordings"];
    let meetings = [
        "Meeting 2026-06-25_10-06-35_2026-06-25_17-06",
        "Meeting 2026-06-25_10-00-47_2026-06-25_17-00",
    ];
    roots
        .iter()
        .flat_map(|root| {
            let root = movies.join(root);
            meetings
                .iter()
                .map(move |f| root.join(f).join("system.wav"))
        })
        .collect()
}

/// Resolve the WAV to tune on, honoring `NIXON_TUNE_WAV`. Returns `None` (skip)
/// when no file is available.
fn resolve_wav() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("NIXON_TUNE_WAV") {
        let p = PathBuf::from(p);
        return p.exists().then_some(p);
    }
    default_candidate_wavs().into_iter().find(|p| p.exists())
}

/// Load + decode the WAV to 16 kHz mono f32, optionally truncated to a slice for
/// fast iteration (`NIXON_TUNE_SLICE_SECS`, 0/unset = whole file).
fn load_samples(wav: &Path) -> Vec<f32> {
    // The real file's header may have been left unfinalized by an aborted run;
    // repair before decode (idempotent, same as the pipeline does).
    let _ = app_lib::audio::channel_writer::repair_wav_header_if_needed(wav);
    let decoded = decode_audio_file(wav).expect("decode system.wav");
    let mut samples = decoded.to_whisper_format();

    let slice_secs: usize = std::env::var("NIXON_TUNE_SLICE_SECS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    if slice_secs > 0 {
        let want = slice_secs * SAMPLE_RATE as usize;
        if samples.len() > want {
            samples.truncate(want);
        }
    }
    eprintln!(
        "loaded {} samples ({:.1} min) from {}",
        samples.len(),
        samples.len() as f32 / SAMPLE_RATE as f32 / 60.0,
        wav.display()
    );
    samples
}

/// Run one diarization pass with the given config and return the distinct speaker
/// count + elapsed seconds.
fn distinct_speakers(
    model_dir: &Path,
    samples: &[f32],
    speakers: SpeakerCount,
    tuning: DiarizeTuning,
) -> (usize, f64) {
    let seg = model_dir.join(SEGMENTATION_FILE);
    let emb = model_dir.join(EMBEDDING_FILE);
    let diarizer =
        SherpaDiarizer::with_config(&seg, &emb, speakers, tuning).expect("init diarizer");
    let t0 = Instant::now();
    let turns = diarizer.diarize(samples, SAMPLE_RATE).expect("diarize");
    let elapsed = t0.elapsed().as_secs_f64();
    let distinct = turns
        .iter()
        .map(|t| t.speaker.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    (distinct, elapsed)
}

/// Sweep `threshold` in Auto mode and print the threshold → speaker-count curve.
/// This is the core diagnostic for the over-clustering bug.
#[test]
#[ignore = "real-file tuning harness; run explicitly with --ignored"]
fn sweep_threshold() {
    let Some(wav) = resolve_wav() else {
        eprintln!("SKIP: no tuning WAV found (set NIXON_TUNE_WAV)");
        return;
    };
    let Some(model_dir) = resolve_model_dir() else {
        eprintln!("SKIP: diarization models not present");
        return;
    };
    let samples = load_samples(&wav);

    eprintln!(
        "\n=== Auto-mode threshold sweep (higher threshold => more merging => fewer speakers) ==="
    );
    eprintln!("threshold | speakers | secs");
    eprintln!("----------|----------|-----");
    for &th in &[0.3f32, 0.4, 0.5, 0.6, 0.7, 0.75, 0.8, 0.85, 0.9, 0.95] {
        let tuning = DiarizeTuning {
            threshold: th,
            ..Default::default()
        };
        let (count, secs) = distinct_speakers(&model_dir, &samples, SpeakerCount::Auto, tuning);
        eprintln!("  {th:>5.2}  |   {count:>4}   | {secs:>5.1}");
    }
    eprintln!("\nShipped default Auto threshold = {DEFAULT_AUTO_THRESHOLD}");
}

/// Confirm the deterministic escape hatch: Fixed(n) forces ~n speakers.
#[test]
#[ignore = "real-file tuning harness; run explicitly with --ignored"]
fn sweep_fixed_count() {
    let Some(wav) = resolve_wav() else {
        eprintln!("SKIP: no tuning WAV found");
        return;
    };
    let Some(model_dir) = resolve_model_dir() else {
        eprintln!("SKIP: models not present");
        return;
    };
    let samples = load_samples(&wav);

    eprintln!("\n=== Fixed-count mode (forces num_clusters = n) ===");
    eprintln!("requested | actual | secs");
    for &n in &[2u32, 5, 8, 10, 12] {
        let (count, secs) = distinct_speakers(
            &model_dir,
            &samples,
            SpeakerCount::Fixed(n),
            DiarizeTuning::default(),
        );
        eprintln!("   {n:>4}   |  {count:>4}  | {secs:>5.1}");
    }
}

/// Validate the shipped Auto default AND Fixed(10) on the WHOLE file (no slice).
/// This is the gate check: Auto should be in a sane range, Fixed(10) ~10.
#[test]
#[ignore = "real-file tuning harness; run explicitly with --ignored"]
fn validate_full_file() {
    // Force whole-file regardless of the slice env used for fast sweeps.
    std::env::remove_var("NIXON_TUNE_SLICE_SECS");
    let Some(wav) = resolve_wav() else {
        eprintln!("SKIP: no tuning WAV found");
        return;
    };
    let Some(model_dir) = resolve_model_dir() else {
        eprintln!("SKIP: models not present");
        return;
    };
    let samples = load_samples(&wav);

    let (auto, auto_s) = distinct_speakers(
        &model_dir,
        &samples,
        SpeakerCount::Auto,
        DiarizeTuning::default(),
    );
    eprintln!(
        "FULL FILE: Auto (threshold={DEFAULT_AUTO_THRESHOLD}) => {auto} speakers in {auto_s:.1}s"
    );

    let (fixed, fixed_s) = distinct_speakers(
        &model_dir,
        &samples,
        SpeakerCount::Fixed(10),
        DiarizeTuning::default(),
    );
    eprintln!("FULL FILE: Fixed(10) => {fixed} speakers in {fixed_s:.1}s");
}

/// specs/0039 WS1: report the same-voice **consolidation** before/after distinct
/// speaker count on a real long `system.wav`, sweeping the floor around the shipped
/// [`CONSOLIDATE_FLOOR`]. Run one (slow) Auto diarization pass with consolidation
/// DISABLED to get the raw clusters + embeddings, then apply [`consolidate_clusters`]
/// purely (fast) for each floor — so the expensive ONNX pass runs once. Use this to
/// tune the floor on real drift-splits and fill the ADR-0005 addendum table.
///
/// ```text
/// export NIXON_TUNE_WAV="$HOME/Movies/nixon-recordings/<folder>/system.wav"
/// export NIXON_DIARIZATION_PROVIDER=cpu   # single pass; CoreML is also fine here
/// cargo test --features metal --test diarization_tuning \
///     sweep_consolidation -- --ignored --nocapture
/// ```
#[test]
#[ignore = "real-file tuning harness; run explicitly with --ignored"]
fn sweep_consolidation() {
    let Some(wav) = resolve_wav() else {
        eprintln!("SKIP: no tuning WAV found (set NIXON_TUNE_WAV)");
        return;
    };
    let Some(model_dir) = resolve_model_dir() else {
        eprintln!("SKIP: diarization models not present");
        return;
    };
    let samples = load_samples(&wav);

    // One Auto pass, consolidation OFF → the raw clustered turns + per-cluster centroids.
    let seg = model_dir.join(SEGMENTATION_FILE);
    let emb = model_dir.join(EMBEDDING_FILE);
    let diarizer =
        SherpaDiarizer::with_config(&seg, &emb, SpeakerCount::Auto, DiarizeTuning::default())
            .expect("init diarizer")
            .with_consolidation(false);

    let t0 = Instant::now();
    let (turns, embeddings) = diarizer
        .diarize_with_embeddings(&samples, SAMPLE_RATE)
        .expect("diarize");
    let elapsed = t0.elapsed().as_secs_f64();

    let distinct = |ts: &[app_lib::diarization::SpeakerTurn]| {
        ts.iter()
            .map(|t| t.speaker.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
    };
    let before = distinct(&turns);
    eprintln!("\n=== Consolidation floor sweep (Auto, one {elapsed:.1}s pass) ===");
    eprintln!("raw distinct speakers (before consolidation) = {before}");
    eprintln!("floor | distinct-after | unknown-bucket?");
    eprintln!("------|----------------|----------------");
    for &floor in &[
        0.35f32,
        0.40,
        0.45,
        0.50,
        0.55,
        0.60,
        CONSOLIDATE_FLOOR,
        0.80,
    ] {
        // Auto-mode bounds (no AtMost seed): the sweep exercises the shipped
        // specs/0041 WS1 cascade budget alongside each candidate floor.
        let opts = ConsolidateOpts {
            floor,
            ..ConsolidateOpts::default()
        };
        let (out_turns, _out_emb) = consolidate_clusters(turns.clone(), embeddings.clone(), opts);
        let after = distinct(&out_turns);
        let has_unknown = out_turns
            .iter()
            .any(|t| t.speaker == app_lib::diarization::UNKNOWN_SPEAKER_KEY);
        let marker = if (floor - CONSOLIDATE_FLOOR).abs() < f32::EPSILON {
            "  (shipped)"
        } else {
            ""
        };
        eprintln!("  {floor:>4.2} |      {after:>4}      | {has_unknown}{marker}");
    }
    eprintln!(
        "\nShipped CONSOLIDATE_FLOOR = {CONSOLIDATE_FLOOR}. Pick the floor that collapses \
         drift-splits without fusing distinct voices, then record before/after in the \
         ADR-0005 addendum."
    );
}

// ===========================================================================
// specs/0041 WS1 — ground-truth DER evaluation harness (2026-07-08)
//
// Scores the app's offline diarization path against Zoom `.transcript.vtt`
// reference labels (cue text is "Speaker Name: text"; timing + name are the
// ground truth, the words are irrelevant). Samples are discovered under
// `NIXON_DIARIZATION_EVAL_DIR` (default: a `zoom-samples/` directory next to
// the repo root); every subdirectory containing one `.mp4` + one `.vtt` is a
// sample. The data is LOCAL ONLY — never copied into the repo — and the tests
// skip cleanly (with a message) when the directory is absent.
//
// Pipeline mirrored here (see `sherpa.rs::diarize_with_embeddings`):
//   raw Auto clustering + per-cluster embeddings  (expensive — ONNX; cached
//   on disk in `.eval-cache/` beside each sample, alongside the ffmpeg-
//   converted 16 kHz mono WAV, so sweeps re-cluster without re-embedding)
//   → consolidate_clusters (WS1 bounds)           (pure, swept here)
//   → merge_clusters_to_at_most (AtMost(n) only)  (pure, swept here)
//
// Scoring: time-weighted DER (miss + false alarm + confusion, over total
// reference speech time) at 10 ms frame resolution with a ±0.25 s collar
// around every reference cue boundary. Cluster→reference mapping is
// GREEDY-BY-OVERLAP (repeatedly take the (cluster, ref) pair with the most
// overlapped scored speech time) — not Hungarian; with ≤ ~30 clusters and
// ≤ 20 reference speakers the greedy assignment is at or near optimal and
// much simpler. Overlapping reference cues are scored with the standard
// multi-speaker DER conventions (a frame needs as many matching mapped
// clusters as reference speakers; overlap regions are INCLUDED, not excluded).
//
// Run (CPU provider recommended: 3 diarizer builds in one process; see the
// provider note at the top of this file):
//
// ```text
// export NIXON_DIARIZATION_PROVIDER=cpu
// cargo test --features metal --test diarization_tuning \
//     eval_ground_truth_sweep -- --ignored --nocapture
// cargo test --features metal --test diarization_tuning \
//     eval_regression_gate -- --ignored --nocapture
// ```
// ===========================================================================

use std::collections::HashMap;

/// Frame resolution for DER scoring (seconds).
const EVAL_FRAME: f64 = 0.010;
/// Collar excluded around every reference cue boundary (seconds, each side).
const EVAL_COLLAR: f64 = 0.25;

/// Resolve the ground-truth samples directory: `NIXON_DIARIZATION_EVAL_DIR`
/// wins; otherwise look for `zoom-samples/` next to the repo root (the repo
/// root is `CARGO_MANIFEST_DIR/../..`), then next to its parent.
fn eval_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("NIXON_DIARIZATION_EVAL_DIR") {
        let p = PathBuf::from(dir);
        return p.is_dir().then_some(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest.join("..").join("..");
    [
        repo_root.join("..").join("zoom-samples"),
        repo_root.join("..").join("..").join("zoom-samples"),
    ]
    .into_iter()
    .filter_map(|p| p.canonicalize().ok())
    .find(|p| p.is_dir())
}

/// One reference speaker turn parsed from the Zoom VTT (times in seconds).
struct RefTurn {
    start: f64,
    end: f64,
    /// Index into [`Reference::names`].
    speaker: usize,
}

/// Parsed ground truth for one sample.
struct Reference {
    names: Vec<String>,
    turns: Vec<RefTurn>,
}

impl Reference {
    fn true_speaker_count(&self) -> usize {
        self.names.len()
    }
}

/// Parse a `HH:MM:SS.mmm` VTT timestamp to seconds.
fn parse_vtt_ts(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec.parse::<f64>().ok()?)
}

/// Parse a Zoom transcript VTT: cues are `start --> end` followed by one text
/// line of the form `Speaker Name: words`. Cues without a `Name: ` prefix are
/// skipped (counted + reported). Gaps between cues are non-speech by
/// definition of the format.
fn parse_zoom_vtt(path: &Path) -> Reference {
    let content = std::fs::read_to_string(path).expect("read vtt");
    let mut names: Vec<String> = Vec::new();
    let mut idx_by_name: HashMap<String, usize> = HashMap::new();
    let mut turns: Vec<RefTurn> = Vec::new();
    let mut skipped = 0usize;

    let mut lines = content.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((start_s, end_s)) = line.split_once("-->") else {
            continue;
        };
        // End token may carry cue settings after whitespace.
        let end_tok = end_s.split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (parse_vtt_ts(start_s), parse_vtt_ts(end_tok)) else {
            continue;
        };
        // First text line of the cue carries the "Speaker Name: " prefix.
        let text = lines.next().unwrap_or("");
        let Some((name, _)) = text.split_once(": ") else {
            skipped += 1;
            continue;
        };
        let name = name.trim().to_string();
        let idx = *idx_by_name.entry(name.clone()).or_insert_with(|| {
            names.push(name);
            names.len() - 1
        });
        turns.push(RefTurn {
            start,
            end,
            speaker: idx,
        });
    }
    if skipped > 0 {
        eprintln!(
            "  vtt: skipped {skipped} cue(s) without a 'Name: ' prefix in {}",
            path.display()
        );
    }
    turns.sort_by(|a, b| a.start.total_cmp(&b.start));
    Reference { names, turns }
}

/// One discovered evaluation sample.
struct EvalSample {
    name: String,
    mp4: PathBuf,
    reference: Reference,
    cache_dir: PathBuf,
}

/// Discover samples: every direct subdirectory of `dir` containing exactly one
/// `.mp4` and one `.vtt`. Sorted by name for stable output.
fn discover_samples(dir: &Path) -> Vec<EvalSample> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let sub = entry.path();
        if !sub.is_dir()
            || sub
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        let mut mp4 = None;
        let mut vtt = None;
        for f in std::fs::read_dir(&sub).into_iter().flatten().flatten() {
            let p = f.path();
            match p.extension().and_then(|e| e.to_str()) {
                Some("mp4") => mp4 = Some(p),
                Some("vtt") => vtt = Some(p),
                _ => {}
            }
        }
        let (Some(mp4), Some(vtt)) = (mp4, vtt) else {
            continue;
        };
        out.push(EvalSample {
            name: entry.file_name().to_string_lossy().to_string(),
            reference: parse_zoom_vtt(&vtt),
            cache_dir: sub.join(".eval-cache"),
            mp4,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Resolve an ffmpeg binary via the app's own discovery (bundled → `$PATH` →
/// auto-install) instead of re-implementing it here.
fn ffmpeg_bin() -> Option<PathBuf> {
    // Prefer the repo's shipped sidecar (what the app actually bundles) so the
    // eval never falls into find_ffmpeg_path's auto-install download from a test.
    let triple_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    if let Ok(entries) = std::fs::read_dir(&triple_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("ffmpeg-") {
                return Some(entry.path());
            }
        }
    }
    app_lib::audio::ffmpeg::find_ffmpeg_path()
}

/// Convert the sample's mp4 to 16 kHz mono s16 WAV via ffmpeg, cached in the
/// sample's `.eval-cache/` so re-runs skip the conversion.
fn ensure_wav(sample: &EvalSample) -> PathBuf {
    let stem = sample.mp4.file_stem().unwrap().to_string_lossy();
    let wav = sample.cache_dir.join(format!("{stem}.16k-mono.wav"));
    if wav.exists() {
        return wav;
    }
    std::fs::create_dir_all(&sample.cache_dir).expect("create .eval-cache");
    let ffmpeg = ffmpeg_bin().expect("ffmpeg binary (bundled, PATH, or auto-install)");
    eprintln!("  converting {} -> {}", sample.mp4.display(), wav.display());
    let status = std::process::Command::new(&ffmpeg)
        .args(["-y", "-i"])
        .arg(&sample.mp4)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg conversion failed for {stem}");
    wav
}

/// Disk-cache shape for one raw (consolidation-OFF, Auto) diarization pass.
#[derive(serde::Serialize, serde::Deserialize)]
struct RawDiarization {
    /// Auto-clustering threshold the pass ran with (cache invalidation key).
    threshold: f32,
    turns: Vec<(f32, f32, String)>,
    embeddings: HashMap<String, Vec<f32>>,
}

/// Raw Auto-clustering threshold for the eval's expensive pass — the shipped
/// [`DEFAULT_AUTO_THRESHOLD`] unless `NIXON_EVAL_RAW_THRESHOLD` overrides it
/// (diagnostic: the eval showed the shipped 0.8 produces an impure dominant
/// cluster on real Zoom audio; this knob lets a raw-threshold sweep reuse the
/// same harness — each value gets its own `.eval-cache` entry).
fn eval_raw_threshold() -> f32 {
    std::env::var("NIXON_EVAL_RAW_THRESHOLD")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_AUTO_THRESHOLD)
}

type TurnsEmb = (
    Vec<app_lib::diarization::SpeakerTurn>,
    HashMap<String, Vec<f32>>,
);

/// specs/0043 W2.1 — alternate embedding model for benchmarking: an absolute
/// path to a speaker-embedding `.onnx` via `NIXON_EVAL_EMBEDDING_MODEL`.
/// Returns the model path plus a cache tag — always the model file stem, since
/// each embedding space needs its own raw passes (untagged pre-W2.1 entries
/// were CAM++ passes and must never be loaded for another model).
fn eval_embedding_model(model_dir: &Path) -> (PathBuf, String) {
    let path = match std::env::var("NIXON_EVAL_EMBEDDING_MODEL") {
        Ok(p) if !p.is_empty() => {
            let path = PathBuf::from(&p);
            assert!(path.exists(), "NIXON_EVAL_EMBEDDING_MODEL not found: {p}");
            path
        }
        _ => model_dir.join(EMBEDDING_FILE),
    };
    let tag = format!(
        "{}.",
        path.file_stem().expect("model file stem").to_string_lossy()
    );
    (path, tag)
}

/// Run (or load from `.eval-cache/`) ONE raw diarization pass per file: Auto
/// mode, tuned default threshold, consolidation disabled — the expensive ONNX
/// part. Every sweep config below is pure post-processing over this result.
/// (Auto and AtMost cluster identically — AtMost caps post-clustering — so a
/// single raw pass serves every swept mode.)
fn raw_diarization_cached(sample: &EvalSample, model_dir: &Path) -> TurnsEmb {
    let threshold = eval_raw_threshold();
    let (embedding_model, model_tag) = eval_embedding_model(model_dir);
    let cache = sample
        .cache_dir
        .join(format!("raw-diar.{model_tag}th{threshold}.v1.json"));
    if let Ok(bytes) = std::fs::read(&cache) {
        if let Ok(raw) = serde_json::from_slice::<RawDiarization>(&bytes) {
            if (raw.threshold - threshold).abs() < f32::EPSILON {
                eprintln!("  raw pass: loaded from cache ({})", cache.display());
                let turns = raw
                    .turns
                    .into_iter()
                    .map(|(start, end, speaker)| app_lib::diarization::SpeakerTurn {
                        start,
                        end,
                        speaker,
                    })
                    .collect();
                return (turns, raw.embeddings);
            }
        }
    }

    let wav = ensure_wav(sample);
    let decoded = decode_audio_file(&wav).expect("decode eval wav");
    let samples = decoded.to_whisper_format();
    eprintln!(
        "  raw pass: diarizing {:.1} min of audio (Auto, threshold {threshold}, \
         consolidation off) — slow, cached after",
        samples.len() as f32 / SAMPLE_RATE as f32 / 60.0
    );
    let diarizer = SherpaDiarizer::with_config(
        &model_dir.join(SEGMENTATION_FILE),
        &embedding_model,
        SpeakerCount::Auto,
        DiarizeTuning {
            threshold,
            ..DiarizeTuning::default()
        },
    )
    .expect("init diarizer")
    .with_consolidation(false);
    let t0 = Instant::now();
    let (turns, embeddings) = diarizer
        .diarize_with_embeddings(&samples, SAMPLE_RATE)
        .expect("diarize");
    eprintln!(
        "  raw pass: {} turns, {} embedded clusters in {:.0}s",
        turns.len(),
        embeddings.len(),
        t0.elapsed().as_secs_f64()
    );

    std::fs::create_dir_all(&sample.cache_dir).expect("create .eval-cache");
    let raw = RawDiarization {
        threshold,
        turns: turns
            .iter()
            .map(|t| (t.start, t.end, t.speaker.clone()))
            .collect(),
        embeddings: embeddings.clone(),
    };
    std::fs::write(&cache, serde_json::to_vec(&raw).unwrap()).expect("write raw cache");
    (turns, embeddings)
}

/// PRE-WS1 (specs/0039 as originally shipped, before commit 02e8d7b)
/// consolidation replica for the "bounds off" arm of the sweep: greedily fuse
/// the most-similar centroid pair while similarity >= `floor` — NO count
/// floor, NO merge budget, NO snowball guard. Merge mechanics (survivor =
/// lexicographically smaller key; duration-weighted centroid combine,
/// re-normalized) mirror the shipped `merge_closest_pairs` so the only
/// difference under test is the WS1 bounding.
fn consolidate_unbounded(turns_emb: TurnsEmb, floor: f32) -> TurnsEmb {
    use app_lib::diarization::embedding::{cosine_similarity, l2_normalize, weighted_mean};
    let (turns, embeddings) = turns_emb;

    struct C {
        key: String,
        centroid: Option<Vec<f32>>,
        duration: f32,
    }
    let mut dur_by_key: HashMap<&str, f32> = HashMap::new();
    for t in &turns {
        *dur_by_key.entry(t.speaker.as_str()).or_insert(0.0) += t.duration();
    }
    let mut clusters: Vec<C> = dur_by_key
        .into_iter()
        .map(|(key, duration)| C {
            key: key.to_string(),
            centroid: embeddings.get(key).cloned(),
            duration,
        })
        .collect();
    clusters.sort_by(|a, b| a.key.cmp(&b.key));

    let mut remap: HashMap<String, String> = HashMap::new();
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for (i, a) in clusters.iter().enumerate() {
            let Some(ci) = a.centroid.as_deref() else {
                continue;
            };
            for (j, b) in clusters.iter().enumerate().skip(i + 1) {
                let Some(cj) = b.centroid.as_deref() else {
                    continue;
                };
                let sim = cosine_similarity(ci, cj);
                if sim >= floor && best.is_none_or(|(_, _, prev)| sim > prev) {
                    best = Some((i, j, sim));
                }
            }
        }
        let Some((i, j, _)) = best else {
            break;
        };
        let (si, li) = if clusters[i].key <= clusters[j].key {
            (i, j)
        } else {
            (j, i)
        };
        let loser = clusters.remove(li);
        let si = if li < si { si - 1 } else { si };
        let survivor = &mut clusters[si];
        if let (Some(sc), Some(lc)) = (survivor.centroid.as_deref(), loser.centroid.as_deref()) {
            let sw = survivor.duration.max(0.0).round().max(1.0) as u32;
            let lw = loser.duration.max(0.0).round().max(1.0) as u32;
            survivor.centroid = Some(l2_normalize(&weighted_mean(sc, sw, lc, lw)));
        }
        survivor.duration += loser.duration;
        let skey = survivor.key.clone();
        for v in remap.values_mut() {
            if *v == loser.key {
                *v = skey.clone();
            }
        }
        remap.insert(loser.key, skey);
    }

    let out_turns = turns
        .into_iter()
        .map(|mut t| {
            if let Some(s) = remap.get(&t.speaker) {
                t.speaker = s.clone();
            }
            t
        })
        .collect();
    let out_emb = clusters
        .into_iter()
        .filter_map(|c| c.centroid.map(|e| (c.key, e)))
        .collect();
    (out_turns, out_emb)
}

/// One sweep configuration.
#[derive(Clone, Copy, PartialEq)]
struct EvalConfig {
    /// Consolidation floor; `None` = consolidation pass skipped entirely.
    floor: Option<f32>,
    /// WS1 cascade bounds (count floor / merge budget / snowball guard) on?
    bounds: bool,
    /// `true` = AtMost(true speaker count) — the calendar-linked-meeting mode;
    /// `false` = Auto — the ad-hoc-meeting mode (see `resolve_speaker_count`).
    at_most_true_n: bool,
}

impl EvalConfig {
    fn label(&self) -> String {
        let floor = match self.floor {
            Some(f) => format!("floor {f:.2}"),
            None => "cons OFF ".to_string(),
        };
        let bounds = if self.floor.is_none() {
            "   -  "
        } else if self.bounds {
            "on    "
        } else {
            "off   "
        };
        let mode = if self.at_most_true_n {
            "AtMost(n)"
        } else {
            "Auto     "
        };
        format!("{floor} | bounds {bounds} | {mode}")
    }
}

/// Apply one config's pure post-processing chain to the cached raw pass,
/// mirroring `diarize_with_embeddings`: consolidation first, then the
/// AtMost cap.
fn apply_config(
    raw: &TurnsEmb,
    cfg: EvalConfig,
    true_n: usize,
) -> Vec<app_lib::diarization::SpeakerTurn> {
    let mut cur = raw.clone();
    if let Some(floor) = cfg.floor {
        cur = if cfg.bounds {
            let opts = ConsolidateOpts {
                floor,
                min_clusters: cfg.at_most_true_n.then_some(true_n),
            };
            consolidate_clusters(cur.0, cur.1, opts)
        } else {
            consolidate_unbounded(cur, floor)
        };
    }
    if cfg.at_most_true_n {
        cur = merge_clusters_to_at_most(cur.0, cur.1, true_n as u32);
    }
    cur.0
}

/// Scored result for one (file, config).
struct FileScore {
    der: f64,
    miss: f64,
    falarm: f64,
    confusion: f64,
    pred_speakers: usize,
    dom_share_hyp: f64,
    dom_share_ref: f64,
    /// (true speaker, predicted-as speaker, seconds) worst confusion pairs.
    confusion_pairs: Vec<(String, String, f64)>,
}

/// Time-weighted DER with a ±[`EVAL_COLLAR`] collar at [`EVAL_FRAME`]
/// resolution; greedy-by-overlap cluster→reference mapping (documented at the
/// section header). Overlap regions are included per standard DER conventions.
fn score_hypothesis(hyp: &[app_lib::diarization::SpeakerTurn], reference: &Reference) -> FileScore {
    // --- frame grids -------------------------------------------------------
    let ref_end = reference.turns.iter().map(|t| t.end).fold(0.0, f64::max);
    let hyp_end = hyp.iter().map(|t| t.end as f64).fold(0.0, f64::max);
    let n_frames = ((ref_end.max(hyp_end) + EVAL_COLLAR) / EVAL_FRAME).ceil() as usize + 1;

    // Reference speakers per frame + collar exclusion around cue boundaries.
    let mut ref_frames: Vec<Vec<u16>> = vec![Vec::new(); n_frames];
    let mut excluded: Vec<bool> = vec![false; n_frames];
    let collar_frames = (EVAL_COLLAR / EVAL_FRAME).round() as usize;
    let clamp = |f: f64| -> usize { (f.max(0.0) as usize).min(n_frames - 1) };
    for t in &reference.turns {
        let (a, b) = (clamp(t.start / EVAL_FRAME), clamp(t.end / EVAL_FRAME));
        let idx = t.speaker as u16;
        for frame in &mut ref_frames[a..b] {
            if !frame.contains(&idx) {
                frame.push(idx);
            }
        }
        for boundary in [a, b] {
            let lo = boundary.saturating_sub(collar_frames);
            let hi = (boundary + collar_frames).min(n_frames - 1);
            for e in &mut excluded[lo..=hi] {
                *e = true;
            }
        }
    }

    // Hypothesis clusters per frame.
    let mut cluster_keys: Vec<String> = hyp
        .iter()
        .map(|t| t.speaker.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    cluster_keys.sort();
    let cluster_idx: HashMap<&str, u16> = cluster_keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i as u16))
        .collect();
    let mut hyp_frames: Vec<Vec<u16>> = vec![Vec::new(); n_frames];
    for t in hyp {
        let (a, b) = (
            clamp(t.start as f64 / EVAL_FRAME),
            clamp(t.end as f64 / EVAL_FRAME),
        );
        let idx = cluster_idx[t.speaker.as_str()];
        for frame in &mut hyp_frames[a..b] {
            if !frame.contains(&idx) {
                frame.push(idx);
            }
        }
    }

    // --- greedy-by-overlap cluster -> reference mapping (scored frames) -----
    let n_ref = reference.names.len();
    let n_hyp = cluster_keys.len();
    let mut overlap = vec![vec![0u64; n_ref]; n_hyp];
    for f in 0..n_frames {
        if excluded[f] {
            continue;
        }
        for &c in &hyp_frames[f] {
            for &r in &ref_frames[f] {
                overlap[c as usize][r as usize] += 1;
            }
        }
    }
    let mut mapping: Vec<Option<usize>> = vec![None; n_hyp]; // cluster -> ref
    let mut ref_taken = vec![false; n_ref];
    let mut hyp_taken = vec![false; n_hyp];
    loop {
        let mut best: Option<(usize, usize, u64)> = None;
        for (c, row) in overlap.iter().enumerate() {
            if hyp_taken[c] {
                continue;
            }
            for (r, &o) in row.iter().enumerate() {
                if ref_taken[r] || o == 0 {
                    continue;
                }
                if best.is_none_or(|(_, _, prev)| o > prev) {
                    best = Some((c, r, o));
                }
            }
        }
        let Some((c, r, _)) = best else { break };
        mapping[c] = Some(r);
        hyp_taken[c] = true;
        ref_taken[r] = true;
    }

    // --- DER accumulation ---------------------------------------------------
    let (mut total_ref, mut miss, mut falarm, mut confusion) = (0u64, 0u64, 0u64, 0u64);
    let mut confusion_secs: HashMap<(usize, usize), u64> = HashMap::new(); // (true r, predicted-as r')
    for f in 0..n_frames {
        if excluded[f] {
            continue;
        }
        let rs = &ref_frames[f];
        let hs = &hyp_frames[f];
        let (nr, nh) = (rs.len() as u64, hs.len() as u64);
        total_ref += nr;
        miss += nr.saturating_sub(nh);
        falarm += nh.saturating_sub(nr);
        let n_correct = rs
            .iter()
            .filter(|&&r| hs.iter().any(|&c| mapping[c as usize] == Some(r as usize)))
            .count() as u64;
        confusion += nr.min(nh) - n_correct;
        // Worst-pair bookkeeping (single-speaker frames only — the common case).
        if rs.len() == 1 && n_correct == 0 {
            let r = rs[0] as usize;
            if let Some(rp) = hs
                .iter()
                .filter_map(|&c| mapping[c as usize])
                .find(|&rp| rp != r)
            {
                *confusion_secs.entry((r, rp)).or_insert(0) += 1;
            }
        }
    }

    // --- summary metrics ----------------------------------------------------
    let mut hyp_dur: HashMap<&str, f64> = HashMap::new();
    for t in hyp {
        *hyp_dur.entry(t.speaker.as_str()).or_insert(0.0) += t.duration() as f64;
    }
    let hyp_total: f64 = hyp_dur.values().sum();
    let dom_share_hyp = hyp_dur.values().fold(0.0, |m, &v| v.max(m)) / hyp_total.max(1e-9);
    let mut ref_dur: HashMap<usize, f64> = HashMap::new();
    for t in &reference.turns {
        *ref_dur.entry(t.speaker).or_insert(0.0) += t.end - t.start;
    }
    let ref_total: f64 = ref_dur.values().sum();
    let dom_share_ref = ref_dur.values().fold(0.0, |m, &v| v.max(m)) / ref_total.max(1e-9);

    let mut pairs: Vec<(String, String, f64)> = confusion_secs
        .into_iter()
        .map(|((r, rp), frames)| {
            (
                reference.names[r].clone(),
                reference.names[rp].clone(),
                frames as f64 * EVAL_FRAME,
            )
        })
        .collect();
    pairs.sort_by(|a, b| b.2.total_cmp(&a.2));
    pairs.truncate(4);

    let denom = (total_ref as f64).max(1.0);
    FileScore {
        der: (miss + falarm + confusion) as f64 / denom,
        miss: miss as f64 / denom,
        falarm: falarm as f64 / denom,
        confusion: confusion as f64 / denom,
        pred_speakers: cluster_keys.len(),
        dom_share_hyp,
        dom_share_ref,
        confusion_pairs: pairs,
    }
}

/// Shared setup for both eval tests: (samples, raw diarizations). `None` = skip
/// (message already printed).
fn eval_setup() -> Option<Vec<(EvalSample, TurnsEmb)>> {
    let Some(dir) = eval_dir() else {
        eprintln!(
            "SKIP: ground-truth eval dir not found (set NIXON_DIARIZATION_EVAL_DIR or place \
             zoom-samples/ next to the repo root)"
        );
        return None;
    };
    let Some(model_dir) = resolve_model_dir() else {
        eprintln!(
            "SKIP: diarization models not present — the app downloads \
             {SEGMENTATION_FILE} + {EMBEDDING_FILE} into <app-data>/models/diarization/ \
             the first time diarization is enabled (Settings → Transcript → speaker labels); \
             NIXON_TEST_DIARIZATION_DIR overrides"
        );
        return None;
    };
    let samples = discover_samples(&dir);
    if samples.is_empty() {
        eprintln!(
            "SKIP: no <sample>/{{*.mp4 + *.vtt}} pairs under {}",
            dir.display()
        );
        return None;
    }
    let mut out = Vec::new();
    for sample in samples {
        eprintln!(
            "sample '{}': {} ref speakers, {} cues",
            sample.name,
            sample.reference.true_speaker_count(),
            sample.reference.turns.len()
        );
        let raw = raw_diarization_cached(&sample, &model_dir);
        out.push((sample, raw));
    }
    Some(out)
}

/// specs/0041 WS1 — the full ground-truth parameter sweep. Prints per-file DER
/// for every (floor × bounds × speaker-count-mode) config plus a ranked
/// macro-average table. Slow only on the first run (raw passes are cached).
#[test]
#[ignore = "real-file ground-truth eval; run explicitly with --ignored"]
fn eval_ground_truth_sweep() {
    let Some(data) = eval_setup() else { return };

    const FLOORS: [f32; 7] = [0.50, 0.55, 0.60, 0.65, 0.70, 0.75, 0.80];
    let mut configs: Vec<EvalConfig> = Vec::new();
    for &at_most in &[false, true] {
        // Reference row: consolidation disabled entirely.
        configs.push(EvalConfig {
            floor: None,
            bounds: true,
            at_most_true_n: at_most,
        });
        for &bounds in &[true, false] {
            for &floor in &FLOORS {
                configs.push(EvalConfig {
                    floor: Some(floor),
                    bounds,
                    at_most_true_n: at_most,
                });
            }
        }
    }

    // score[config][file]
    let mut rows: Vec<(EvalConfig, Vec<FileScore>)> = Vec::new();
    for &cfg in &configs {
        let mut scores = Vec::new();
        for (sample, raw) in &data {
            let true_n = sample.reference.true_speaker_count();
            let hyp = apply_config(raw, cfg, true_n);
            scores.push(score_hypothesis(&hyp, &sample.reference));
        }
        rows.push((cfg, scores));
    }

    // Per-file detail table.
    for (fi, (sample, raw)) in data.iter().enumerate() {
        let raw_n = raw
            .0
            .iter()
            .map(|t| t.speaker.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        eprintln!(
            "\n=== {} (true speakers {}, raw clusters {}, dominant ref share {:.0}%) ===",
            sample.name,
            sample.reference.true_speaker_count(),
            raw_n,
            rows[0].1[fi].dom_share_ref * 100.0
        );
        eprintln!("config                                | DER%   | miss% | fa%  | conf% | pred_n | dom_hyp%");
        eprintln!("--------------------------------------|--------|-------|------|-------|--------|---------");
        for (cfg, scores) in &rows {
            let s = &scores[fi];
            eprintln!(
                "{} | {:>6.2} | {:>5.2} | {:>4.2} | {:>5.2} | {:>6} | {:>7.1}",
                cfg.label(),
                s.der * 100.0,
                s.miss * 100.0,
                s.falarm * 100.0,
                s.confusion * 100.0,
                s.pred_speakers,
                s.dom_share_hyp * 100.0
            );
        }
    }

    // Ranked macro table.
    let mut ranked: Vec<(usize, f64)> = rows
        .iter()
        .enumerate()
        .map(|(i, (_, scores))| {
            let macro_avg = scores.iter().map(|s| s.der).sum::<f64>() / scores.len() as f64;
            (i, macro_avg)
        })
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
    eprintln!("\n=== Ranked by macro-average DER ===");
    eprint!("config                                | macro% |");
    for (sample, _) in &data {
        eprint!(" {:>12.12} |", sample.name);
    }
    eprintln!();
    for (i, macro_avg) in &ranked {
        let (cfg, scores) = &rows[*i];
        eprint!("{} | {:>6.2} |", cfg.label(), macro_avg * 100.0);
        for s in scores {
            eprint!(" {:>12.2} |", s.der * 100.0);
        }
        eprintln!();
    }

    // Worst confusion pairs at the shipped config (bounds on, shipped floor),
    // both modes — the qualitative "who gets mistaken for whom" summary.
    for &at_most in &[false, true] {
        let shipped = EvalConfig {
            floor: Some(CONSOLIDATE_FLOOR),
            bounds: true,
            at_most_true_n: at_most,
        };
        if let Some((_, scores)) = rows.iter().find(|(c, _)| *c == shipped) {
            eprintln!(
                "\n=== Worst confusion pairs at shipped config ({}) ===",
                shipped.label()
            );
            for ((sample, _), s) in data.iter().zip(scores) {
                for (truth, pred, secs) in &s.confusion_pairs {
                    eprintln!(
                        "  {}: '{truth}' scored as '{pred}' for {secs:.0}s",
                        sample.name
                    );
                }
            }
        }
    }
}

// --- scorer sanity checks (pure, fast, NOT ignored — no models/audio) ------

/// A hypothesis identical to the reference must score DER 0 and map cleanly.
#[test]
fn eval_scorer_perfect_hypothesis_is_zero_der() {
    let reference = Reference {
        names: vec!["A".into(), "B".into()],
        turns: vec![
            RefTurn {
                start: 0.0,
                end: 10.0,
                speaker: 0,
            },
            RefTurn {
                start: 12.0,
                end: 20.0,
                speaker: 1,
            },
            RefTurn {
                start: 22.0,
                end: 30.0,
                speaker: 0,
            },
        ],
    };
    let hyp: Vec<app_lib::diarization::SpeakerTurn> = [
        (0.0f32, 10.0f32, "spk_0"),
        (12.0, 20.0, "spk_1"),
        (22.0, 30.0, "spk_0"),
    ]
    .into_iter()
    .map(|(start, end, k)| app_lib::diarization::SpeakerTurn {
        start,
        end,
        speaker: k.into(),
    })
    .collect();
    let s = score_hypothesis(&hyp, &reference);
    assert!(s.der < 1e-9, "perfect hypothesis scored DER {}", s.der);
    assert_eq!(s.pred_speakers, 2);
}

/// Two reference speakers collapsed into one cluster: the un-mapped speaker's
/// speech is pure confusion (~ its share of scored time), none of it miss/fa.
#[test]
fn eval_scorer_collapsed_cluster_scores_confusion() {
    let reference = Reference {
        names: vec!["A".into(), "B".into()],
        turns: vec![
            RefTurn {
                start: 0.0,
                end: 10.0,
                speaker: 0,
            },
            RefTurn {
                start: 12.0,
                end: 22.0,
                speaker: 1,
            },
        ],
    };
    let hyp: Vec<app_lib::diarization::SpeakerTurn> = [(0.0f32, 10.0f32), (12.0, 22.0)]
        .into_iter()
        .map(|(start, end)| app_lib::diarization::SpeakerTurn {
            start,
            end,
            speaker: "spk_0".into(),
        })
        .collect();
    let s = score_hypothesis(&hyp, &reference);
    assert!(s.miss < 1e-9 && s.falarm < 1e-9);
    // Half the reference speech is the collapsed speaker (collar shaves edges).
    assert!(
        (s.confusion - 0.5).abs() < 0.03,
        "expected ~50% confusion, got {}",
        s.confusion
    );
    assert!((s.der - s.confusion).abs() < 1e-9);
}

/// Hypothesis speech where the reference has silence is false alarm; missing
/// reference speech is miss.
#[test]
fn eval_scorer_miss_and_false_alarm() {
    let reference = Reference {
        names: vec!["A".into()],
        turns: vec![RefTurn {
            start: 0.0,
            end: 10.0,
            speaker: 0,
        }],
    };
    // Covers 0..5 (correct half), then hallucinates 20..25.
    let hyp: Vec<app_lib::diarization::SpeakerTurn> = vec![
        app_lib::diarization::SpeakerTurn {
            start: 0.0,
            end: 5.0,
            speaker: "spk_0".into(),
        },
        app_lib::diarization::SpeakerTurn {
            start: 20.0,
            end: 25.0,
            speaker: "spk_0".into(),
        },
    ];
    let s = score_hypothesis(&hyp, &reference);
    assert!(s.confusion < 1e-9);
    assert!(
        (s.miss - 0.5).abs() < 0.06,
        "expected ~50% miss, got {}",
        s.miss
    );
    // 5s hallucinated over ~9.5s scored ref speech ≈ 52% false alarm.
    assert!(
        s.falarm > 0.4,
        "expected large false alarm, got {}",
        s.falarm
    );
}

/// Pinned regression thresholds for [`eval_regression_gate`]: at the shipped
/// defaults (specs/0043 W2.2: TitaNet-L embeddings, raw threshold 0.80,
/// `MERGE_FLOOR` 0.45, `CONSOLIDATE_FLOOR` 0.50), per-file DER measured
/// 2026-07-12 (raw pass on CPU provider) plus ~3 pp absolute headroom for
/// provider/raw-pass nondeterminism. Measured values (2026-07-10 wave-1 CAM++
/// values in parens — the model swap is the wave-2 step change; the 2026-07-08
/// pre-0043 baseline was 42–66%):
///
/// | sample              | Auto DER        | AtMost(true n) DER |
/// |---------------------|-----------------|--------------------|
/// | short-4-speakers    | 10.25% (30.37%) | 10.10% (28.32%)    |
/// | long-1-1            |  5.98% (16.71%) |  5.73% (15.29%)    |
/// | long-many-speakers  |  8.33% (62.44%) |  8.39% (54.83%)    |
///
/// Known caveat carried forward: Auto mode still over-splits cluster *counts*
/// (25/98/29 predicted vs 2/20/4 true; mostly sub-10s slivers, so DER-cheap).
/// AtMost counts are essentially exact (3/21/5). If Auto-mode label sprawl
/// bothers real no-calendar meetings, the next lever is multi-pass
/// consolidation — see specs/0043 task 9 notes.
///
/// (mode, name-substring, max allowed DER). See specs/0043 for the sweep data.
/// specs/0050 — reproduce the production offline self-seed exactly: consolidate with
/// the Auto budget (the invite is a ceiling now, not a floor), estimate the
/// audio-derived speaker count, and cap at `min(n_audio, clean_invite_ceiling)`
/// (clamped ≥ 2). `invite_ceiling` = `None` for an ad-hoc meeting, `Some(n)` for a
/// clean calendar invite. Mirrors `SherpaDiarizer::diarize_with_embeddings`.
fn apply_audio_seed(
    raw: &TurnsEmb,
    invite_ceiling: Option<usize>,
) -> Vec<app_lib::diarization::SpeakerTurn> {
    let (turns, emb) = consolidate_clusters(
        raw.0.clone(),
        raw.1.clone(),
        ConsolidateOpts {
            floor: CONSOLIDATE_FLOOR,
            min_clusters: None,
        },
    );
    let n_audio = estimate_speakers_by_duration(&turns, N_AUDIO_MIN_SECS);
    let cap = invite_ceiling.map_or(n_audio, |c| n_audio.min(c)).max(2) as u32;
    merge_clusters_to_at_most(turns, emb, cap).0
}

/// specs/0050: pins are for the shipped AUDIO-SEED path (Auto-budget consolidation +
/// AtMost(min(n_audio, invite))). Measured 2026-08-10 = the AtMost(true-n) oracle
/// numbers (long-1-1 5.73%, long-many 8.39%, short-4 10.10%), since n_audio ≈ true
/// count; ceilings keep the pre-0050 headroom.
const EVAL_PINNED_DER: &[(&str, bool, f64)] = &[
    // (sample substring, at_most_true_n, max DER = measured + ~0.03)
    ("short-4-speakers", false, 0.14),
    ("short-4-speakers", true, 0.14),
    ("long-1-1", false, 0.09),
    ("long-1-1", true, 0.09),
    ("long-many-speakers", false, 0.12),
    ("long-many-speakers", true, 0.12),
];

/// specs/0041 WS1 — regression gate: at the shipped defaults the per-file DER
/// must not exceed the pinned measured value + headroom.
#[test]
#[ignore = "real-file ground-truth eval; run explicitly with --ignored"]
fn eval_regression_gate() {
    let Some(data) = eval_setup() else { return };
    let mut failures = Vec::new();
    for (sample, raw) in &data {
        let true_n = sample.reference.true_speaker_count();
        for &(substr, at_most, max_der) in EVAL_PINNED_DER {
            if !sample.name.contains(substr) {
                continue;
            }
            // specs/0050: the shipped offline path is the audio self-seed. `at_most`
            // here means "a CLEAN calendar invite is present as a ceiling" (=true_n)
            // vs an ad-hoc meeting with no ceiling.
            let hyp = apply_audio_seed(raw, at_most.then_some(true_n));
            let s = score_hypothesis(&hyp, &sample.reference);
            let mode = if at_most {
                "audio-seed + invite"
            } else {
                "audio-seed (ad-hoc)"
            };
            eprintln!(
                "gate {} [{mode}]: DER {:.2}% (pinned max {:.2}%), pred {} / true {}",
                sample.name,
                s.der * 100.0,
                max_der * 100.0,
                s.pred_speakers,
                true_n
            );
            if s.der > max_der {
                failures.push(format!(
                    "{} [{mode}]: DER {:.4} > pinned {max_der:.4}",
                    sample.name, s.der
                ));
            }
        }
    }
    assert!(failures.is_empty(), "DER regression(s): {failures:?}");
}

/// specs/0050 — SEED SIMULATION. The DER sweep showed clustering
/// is near-optimal and the whole game is the `AtMost(n)` speaker-count seed. Real
/// calendar invites give a WRONG `n` two ways (owner-reported): a distribution-list
/// invite UNDER-counts (one list entry = many people) and a big optional invite
/// OVER-counts (50 invited, 5 speak). Under-count is the harmful one — `AtMost`
/// caps BELOW the true count and force-merges real speakers (the DER cliff).
///
/// This simulates both on the labelled samples (long-many as a 1-entry dist list,
/// short-4 as a 50-person invite) and prototypes an audio-derived count
/// [`estimate_n_audio`] that ignores the invite. Prints DER + predicted count for:
/// Auto (no seed), AtMost(true n) [oracle], AtMost(bad invite) [today], and
/// AtMost(n_audio) across thresholds — so we can pick a robust threshold on ground
/// truth before touching the calendar path. Test-only; no production change.
#[test]
#[ignore = "real-file ground-truth eval; run explicitly with --ignored"]
fn eval_seed_simulation() {
    let Some(data) = eval_setup() else { return };

    fn simulated_invite(name: &str) -> Option<(u32, &'static str)> {
        if name.contains("long-many") {
            Some((1, "distribution-list invite (1 entry)"))
        } else if name.contains("short-4") {
            Some((50, "large optional invite (50)"))
        } else {
            None // long-1-1 = correct-invite control
        }
    }

    for (sample, raw) in &data {
        let true_n = sample.reference.true_speaker_count();
        // Shipped Auto consolidation — exactly what production produces pre-cap.
        let (cons_turns, cons_emb) = consolidate_clusters(
            raw.0.clone(),
            raw.1.clone(),
            ConsolidateOpts {
                floor: CONSOLIDATE_FLOOR,
                min_clusters: None,
            },
        );
        let cap = |k: u32| -> Vec<app_lib::diarization::SpeakerTurn> {
            merge_clusters_to_at_most(cons_turns.clone(), cons_emb.clone(), k).0
        };
        let show = |label: &str, hyp: &[app_lib::diarization::SpeakerTurn]| {
            let s = score_hypothesis(hyp, &sample.reference);
            eprintln!(
                "  {label:<40} DER {:>6.2}%  pred {:>3}  (true {true_n})",
                s.der * 100.0,
                s.pred_speakers,
            );
        };

        eprintln!("\n=== {} (true {true_n}) ===", sample.name);
        show("Auto (no seed)", &cons_turns);
        show(&format!("AtMost(true n={true_n}) [oracle]"), &cap(true_n as u32));
        if let Some((k, desc)) = simulated_invite(&sample.name) {
            show(&format!("AtMost(invite={k}) [{desc}]"), &cap(k));
        }
        for &t in &[5.0f32, N_AUDIO_MIN_SECS, 15.0, 20.0, 30.0] {
            let n = estimate_speakers_by_duration(&cons_turns, t);
            let tag = if (t - N_AUDIO_MIN_SECS).abs() < f32::EPSILON {
                " <-- shipped"
            } else {
                ""
            };
            show(&format!("n_audio(>={t:>4.0}s)={n} -> AtMost({n}){tag}"), &cap(n as u32));
        }
    }
}
