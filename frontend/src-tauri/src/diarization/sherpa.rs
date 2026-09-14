//! sherpa-onnx-backed [`Diarizer`] implementation. (specs/0010, ADR-0005)
//!
//! Wraps the offline pipeline of pyannote segmentation → speaker embedding →
//! fast clustering. Runs on CPU (batch, post-meeting; it must not compete with
//! live STT). Input is 16 kHz mono f32 — the format the VAD stage already
//! produces.
//!
//! The two ONNX models are managed by [`crate::diarization::models`]; pass their
//! resolved paths to [`SherpaDiarizer::new`].

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// specs/0053 W2: sherpa-rs pins the diarizer's ONNX models to one thread; our
// own binding makes num_threads settable. Everything else here is unchanged.
use crate::diarization::sherpa_sys::{ThreadedDiarize, ThreadedDiarizeConfig};

use crate::diarization::{speaker_key_for_cluster, Diarizer, SpeakerTurn};

/// Sample rate the diarization models expect.
const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Peak-amplitude gate below which the system channel is treated as having no
/// remote speech, so we skip sherpa entirely and return zero turns.
///
/// 1e-3 in f32 PCM is ~-60 dBFS. Real speech sits well above this (the silent
/// `system.wav` that triggered this bug measured ~-91 dB, while the spoken
/// `mic.wav` was ~-21 dB), so -60 dBFS comfortably separates "digital silence /
/// idle line noise" from "someone actually spoke" without risking a false skip
/// on genuine, quiet remote audio. A silent/no-remote-speech system channel is a
/// legitimate common case (solo recording, or a meeting where the remote side
/// never spoke); the alignment stage then labels every segment as the local
/// user ("You"), which is the correct outcome — not an error.
const SILENCE_PEAK_THRESHOLD: f32 = 1e-3;

/// Substring sherpa-rs surfaces when its segmentation model finds no speech in
/// the (silent or speechless) input. We map *only* this case to zero turns;
/// anything else from `compute()` is re-raised.
const NO_SEGMENTS_MARKER: &str = "No segments found";

/// True when `samples` carries no meaningful speech: empty, or whose peak
/// amplitude is below [`SILENCE_PEAK_THRESHOLD`]. Pure so it's unit-testable
/// without loading the ONNX models.
fn is_silent(samples: &[f32]) -> bool {
    let peak = samples.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    peak < SILENCE_PEAK_THRESHOLD
}

/// sherpa's clustering treats `num_clusters <= 0` as "decide automatically using
/// `threshold`". Note: sherpa-rs's `DiarizeConfig::Default` and `num_clusters:
/// None` both map to a fixed `4` inside `sherpa-rs` (it `unwrap_or(4)`s
/// `None`), so to get the *auto* behavior the spec/ADR call for we must pass
/// an explicit non-positive value here rather than `None`.
const AUTO_NUM_CLUSTERS: i32 = -1;

/// Speaker count mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SpeakerCount {
    /// Let clustering decide using the distance `threshold` (ADR-0005 default).
    #[default]
    Auto,
    /// Force exactly `n` speakers (the settings "expected speakers" override —
    /// the user asserted an exact number, so sherpa's `num_clusters` is forced).
    Fixed(u32),
    /// Upper bound of `n` speakers (specs/0017). Cluster *automatically* (the Auto
    /// `threshold`), then guarantee no more than `n` distinct speakers by merging
    /// the closest cluster centroids until `<= n` remain. Unlike [`Fixed`], this is
    /// a cap, not a target: a 4-speaker meeting with 10 invitees still yields 4
    /// clusters. sherpa has no native "at most n", so the cap is enforced
    /// *post-clustering* (see [`merge_clusters_to_at_most`]); `n == 0` means "no
    /// cap" and behaves exactly like [`Auto`].
    ///
    /// [`Fixed`]: SpeakerCount::Fixed
    /// [`Auto`]: SpeakerCount::Auto
    AtMost(u32),
}

impl SpeakerCount {
    /// Stable wire string for the resolved mode, emitted as `speakerCountMode` on
    /// `diarization-complete` (specs/0017): `"fixed"` (exact), `"at_most"` (cap), or
    /// `"auto"` (clustering decides). Distinct from `speakerCountSource`
    /// (`manual|calendar|auto`) — the source is *who* chose the count, the mode is
    /// *how* the clusterer treats it.
    pub fn mode_str(self) -> &'static str {
        match self {
            SpeakerCount::Fixed(_) => "fixed",
            SpeakerCount::AtMost(_) => "at_most",
            SpeakerCount::Auto => "auto",
        }
    }
}

/// Clustering / segmentation tuning knobs passed straight to
/// `ThreadedDiarizeConfig` (specs/0011 accuracy gate). Pulled out so the offline pipeline can keep the
/// tuned defaults while the ignored real-file harness can sweep them.
///
/// ## How these map to sherpa's behaviour (3D-Speaker CAM++ embeddings)
///
/// In **Auto** mode (`num_clusters <= 0`), sherpa runs agglomerative clustering
/// and `threshold` is the cosine *distance* cut height: two clusters merge while
/// their distance is **below** `threshold`. So a **higher** `threshold` merges
/// more aggressively → **fewer** speakers; a **lower** `threshold` splits more →
/// **more** speakers. (In **Fixed** mode `threshold` is ignored — `num_clusters`
/// wins.)
///
/// `min_duration_on` drops speech segments shorter than this (seconds) before
/// clustering; `min_duration_off` bridges gaps shorter than this within one
/// speaker. Both reduce spurious micro-segments that otherwise seed extra
/// clusters.
///
/// ### Chosen defaults (evidence — specs/0011 P3-0 accuracy gate)
///
/// Measured by `tests/diarization_tuning.rs` (ignored) on a real ~10-speaker
/// `system.wav`. The shipped default of `threshold = 0.5` over-clustered badly
/// (100+ speakers on the full ~90-min file). Sweeping `threshold` on a 12-min
/// slice of that file showed the expected monotonic curve — higher merges more:
///
/// | threshold | distinct speakers |
/// |-----------|-------------------|
/// | 0.30      | 35                |
/// | 0.40      | 32                |
/// | 0.50      | 23  (old default) |
/// | 0.60      | 17                |
/// | 0.70      | 14                |
/// | **0.80**  | **10**  (= truth) |
/// | 0.85      | 9                 |
/// | 0.90      | 8                 |
/// | 0.95      | 7                 |
///
/// `0.80` lands exactly on the ~10-speaker ground truth on the slice and keeps
/// the full file in a sane range (vs 100+ before). Off-by-a-few is fixable via
/// the P2 merge UI; the user-set "expected speakers" override forces an exact
/// count. `min_duration_*` kept at sherpa's pyannote-recommended values — the
/// sweep did not need to touch them to fix the cluster count.
#[derive(Debug, Clone, Copy)]
pub struct DiarizeTuning {
    /// Agglomerative-clustering distance cut height (Auto mode only). Higher =
    /// fewer speakers.
    pub threshold: f32,
    /// Minimum on-speech duration (s) kept before clustering.
    pub min_duration_on: f32,
    /// Minimum off-speech gap (s) bridged within one speaker.
    pub min_duration_off: f32,
}

impl Default for DiarizeTuning {
    fn default() -> Self {
        Self {
            // Tuned up from the original 0.5 (which over-clustered to 100+ on the
            // real ~10-speaker file) to merge same-speaker clusters far more
            // aggressively for the CAM++ embedding distance scale. See the
            // type-level table + tests/diarization_tuning.rs for the sweep.
            threshold: DEFAULT_AUTO_THRESHOLD,
            min_duration_on: 0.3,
            min_duration_off: 0.5,
        }
    }
}

/// Tuned Auto-mode clustering threshold (cosine *distance*; higher merges more).
///
/// **Measured (2026-07-12, specs/0043 W2.1/W2.2, TitaNet-L embeddings):** swept
/// 0.40–0.80 against the three-file ground-truth harness
/// (`eval_ground_truth_sweep`, `NIXON_EVAL_RAW_THRESHOLD` ×
/// `NIXON_EVAL_EMBEDDING_MODEL`). TitaNet is remarkably threshold-robust —
/// macro DER stays within 8.1–8.7% across the whole sweep — with 0.80 the
/// optimum (8.1% macro; 5.9/8.1/10.2% per file). No cliff anywhere, unlike
/// CAM++ (whose 2026-07-10 W1.2 sweep found a 0.55 optimum with a cliff at
/// 0.60; that tuning is obsolete with the model swap, see ADR-0011).
pub const DEFAULT_AUTO_THRESHOLD: f32 = 0.80;

/// Above this many Auto-mode clusters for a single recording we consider the
/// result pathological over-clustering and log a loud warning recommending the
/// "expected speakers" override (we never silently truncate — the P2 merge UI is
/// the user's recovery path). Real meetings essentially never have this many
/// distinct voices on one channel.
const OVER_CLUSTER_WARN_CAP: usize = 30;

/// Cosine-similarity floor below which [`merge_clusters_to_at_most`] refuses to
/// merge a pair, even when the cluster count still exceeds the requested cap
/// (specs/0017 anti-false-merge guard).
///
/// `AtMost(n)` enforces an *upper bound* by merging the closest cluster
/// centroids. But if the audio genuinely contains more than `n` distinct voices,
/// the "closest" remaining pair may still be two clearly different people — and
/// fusing two real speakers into one is a worse error than slightly over-counting.
/// So we never merge a pair whose centroid cosine similarity is below this floor:
/// if the best available merge is below it, we STOP and accept `> n` clusters,
/// logging the reason.
///
/// **Measured (2026-07-12, specs/0043 W2.2, TitaNet-L embeddings):** swept
/// 0.30/0.40/0.45/0.60 at raw threshold 0.80 — 0.45 is the AtMost optimum
/// (8.07% macro). 0.30 scores 20.3% (distinct TitaNet voices reach cos > 0.30,
/// so the cap fuses real people); 0.40 scores 9.0%; 0.60 scores 8.17% but would
/// sit above [`CONSOLIDATE_FLOOR`] (0.50), breaking the clamp invariant.
/// Refusing a bad merge sends overflow to the [`UNKNOWN_SPEAKER_KEY`] bucket —
/// an honest "Unknown speaker" beats fusing two real people. Raise it to merge
/// more cautiously, lower it to honor the cap harder.
pub const MERGE_FLOOR: f32 = 0.45;

/// Speaker key for the WS3.3 (specs/0029) overflow bucket: when
/// [`merge_clusters_to_at_most`] cannot honor an `AtMost(n)` bound without merging
/// centroids below [`MERGE_FLOOR`], the top-`n` clusters by total speech duration stay
/// first-class (`spk_N`) and every remaining cluster is relabeled to this single key,
/// rendered as "Unknown speaker" (see `pipeline::display_name_for_key`). The bucket is
/// a mix of voices, so it deliberately carries **no** centroid/voiceprint.
pub const UNKNOWN_SPEAKER_KEY: &str = "unknown";

/// Cosine-similarity floor at or above which [`consolidate_clusters`] fuses two
/// same-voice clusters — the WS1 (specs/0039) long-meeting drift-split repair.
///
/// Deliberately **higher** than [`MERGE_FLOOR`]. Consolidation is an
/// *unrequested*, automatic merge that runs in Auto/AtMost mode on every long call,
/// so it must be strictly *more conservative* than the anti-false-merge floor that
/// only guards a *user-requested* `AtMost(n)` cap: fusing two genuinely distinct
/// voices is a worse error than leaving a same-voice drift-split, and the automatic
/// pass has no user in the loop to catch a bad merge. We therefore only consolidate a
/// pair whose centroids are clearly the *same* speaker.
///
/// The floor must sit above the normal inter-speaker band *for the current
/// embedding model's cosine scale* — high enough that two distinct
/// same-gender/similar-timbre voices don't clear it, low enough that genuine
/// same-voice drift-splits do. Get it wrong low and whole meetings collapse into
/// the dominant speaker (the specs/0041 WS1 regression at CAM++ floor 0.50);
/// get it wrong high and drift-splits stay broken. **Empirical** — tune on real
/// long files via `tests/diarization_tuning.rs`. Overridable per run via the
/// optional `consolidation_floor` setting; the diarizer clamps any override to
/// stay above `MERGE_FLOOR`.
///
/// **Measured (2026-07-12, specs/0043 W2.2, TitaNet-L embeddings):** TitaNet's
/// cosine scale is compressed vs CAM++ — its same-voice drift-splits sit around
/// cos 0.50–0.55 (a real long-1-1 split pair merges at floor 0.50 but not 0.55,
/// worth 23 DER points on that file), while its distinct voices fall below.
/// Floor 0.50 is the sweep optimum at every raw threshold 0.40–0.80. (CAM++
/// history: 0.70 per specs/0041 WS1, 0.65 per the W1.1 re-tune — obsolete with
/// the model swap; the 0.50-collapse regression above was a CAM++-scale fact,
/// not a universal one. See ADR-0011.)
pub const CONSOLIDATE_FLOOR: f32 = 0.50;

/// Extra similarity margin (on top of the consolidation floor) a pair must clear when
/// its would-be survivor has already absorbed a merge in the same pass — the
/// specs/0041 WS1 **snowball guard**. Every merge duration-weights the surviving
/// centroid toward the longer-speaking half, dragging it through embedding space;
/// unguarded, the dominant cluster's centroid can creep toward — and then absorb —
/// each remaining cluster in turn. Requiring `floor + 0.05` for a repeat survivor
/// breaks that drag while still letting a genuine multi-way drift-split (very high
/// pairwise similarity) consolidate fully.
pub const CONSOLIDATE_SNOWBALL_MARGIN: f32 = 0.05;

/// Offline, CPU, sherpa-onnx diarizer.
///
/// `ThreadedDiarize` holds a raw C handle and `compute(&mut self, …)` needs
/// exclusive access, so we guard it behind a `Mutex` to satisfy the
/// `Diarizer: Sync` bound and to serialize inference (each call itself now
/// runs the ONNX models across multiple threads — see `sherpa_sys`).
pub struct SherpaDiarizer {
    inner: Mutex<ThreadedDiarize>,
    segmentation_path: PathBuf,
    embedding_path: PathBuf,
    /// Cluster-count mode this instance was built with (Auto vs Fixed). Kept so
    /// the over-cluster guardrail only fires in Auto mode (Fixed can't over-cluster).
    speaker_count: SpeakerCount,
    /// specs/0039 WS1 — cosine-similarity floor for the same-voice consolidation pass
    /// (see [`CONSOLIDATE_FLOOR`]). Defaults to the const; overridable via the optional
    /// `consolidation_floor` setting through [`Self::with_consolidation_floor`].
    consolidate_floor: f32,
    /// specs/0039 WS1 — whether the same-voice consolidation pass runs in
    /// `diarize_with_embeddings`. On by default (the **offline** pass, the authoritative
    /// path). The **live** windowed pass disables it via [`Self::with_consolidation`]
    /// (`false`): live labels are provisional and reconciled by the offline pass, so
    /// consolidation is a Non-goal there (specs/0039).
    consolidate_enabled: bool,
    /// Lazily-created re-embed extractor for `diarize_with_embeddings` (specs/0011).
    /// `None` until the first embedding pass; `Some(None)` if init failed once (so we
    /// don't retry every pass). Separate from `inner` so the offline `diarize()` path
    /// is untouched and never loads it.
    embedder: Mutex<Option<Option<crate::diarization::embedding::ClusterEmbedder>>>,
}

impl SherpaDiarizer {
    /// Construct from the two ONNX model paths, auto speaker count.
    pub fn new(segmentation_model: &Path, embedding_model: &Path) -> Result<Self> {
        Self::with_speaker_count(segmentation_model, embedding_model, SpeakerCount::Auto)
    }

    /// Construct with an explicit [`SpeakerCount`] mode and the tuned default
    /// clustering knobs.
    pub fn with_speaker_count(
        segmentation_model: &Path,
        embedding_model: &Path,
        speakers: SpeakerCount,
    ) -> Result<Self> {
        Self::with_config(
            segmentation_model,
            embedding_model,
            speakers,
            DiarizeTuning::default(),
        )
    }

    /// Construct with an explicit [`SpeakerCount`] mode and explicit
    /// [`DiarizeTuning`]. The general constructor the others delegate to; used
    /// directly by the ignored real-file tuning harness (specs/0011).
    pub fn with_config(
        segmentation_model: &Path,
        embedding_model: &Path,
        speakers: SpeakerCount,
        tuning: DiarizeTuning,
    ) -> Result<Self> {
        if !segmentation_model.exists() {
            return Err(anyhow!(
                "segmentation model not found: {}",
                segmentation_model.display()
            ));
        }
        if !embedding_model.exists() {
            return Err(anyhow!(
                "embedding model not found: {}",
                embedding_model.display()
            ));
        }

        let num_clusters = match speakers {
            // AtMost clusters freely (Auto threshold) and the cap is applied
            // *after* clustering via centroid merge, so it maps to AUTO here.
            SpeakerCount::Auto | SpeakerCount::AtMost(_) => AUTO_NUM_CLUSTERS,
            SpeakerCount::Fixed(n) => n.max(1) as i32,
        };

        // Build on the preferred provider (CoreML on macOS — our bundled ORT has
        // the CoreML EP) and fall back to CPU on any init error, so the working
        // CPU path is always a guaranteed fallback. Accuracy semantics are
        // unchanged: same models, same thresholds/clustering — only the ONNX
        // execution backend differs.
        let diarize = crate::diarization::accel::build_with_provider_fallback(
            "diarizer (segmentation+embedding)",
            |provider| {
                let config = ThreadedDiarizeConfig::with_defaults(
                    num_clusters,
                    tuning.threshold,
                    tuning.min_duration_on,
                    tuning.min_duration_off,
                    provider.to_string(),
                );
                ThreadedDiarize::new(segmentation_model, embedding_model, config)
            },
        )
        .context("ThreadedDiarize::new")?;

        Ok(Self {
            inner: Mutex::new(diarize),
            segmentation_path: segmentation_model.to_path_buf(),
            embedding_path: embedding_model.to_path_buf(),
            speaker_count: speakers,
            consolidate_floor: CONSOLIDATE_FLOOR,
            consolidate_enabled: true,
            embedder: Mutex::new(None),
        })
    }

    /// Enable/disable the WS1 (specs/0039) same-voice consolidation pass for this
    /// instance. On by default (offline pass). The live windowed pass disables it —
    /// live labels are provisional and reconciled by the offline pass (a Non-goal to
    /// consolidate live).
    pub fn with_consolidation(mut self, enabled: bool) -> Self {
        self.consolidate_enabled = enabled;
        self
    }

    /// Override the WS1 (specs/0039) consolidation similarity floor for this instance.
    ///
    /// Clamped to stay strictly above [`MERGE_FLOOR`] (and `<= 1.0`) so a bad setting
    /// can never make consolidation *less* conservative than the anti-false-merge cap
    /// floor — the ordering invariant `CONSOLIDATE_FLOOR > MERGE_FLOOR` is a safety
    /// property, not a tunable. A clamped value is logged.
    pub fn with_consolidation_floor(mut self, floor: f32) -> Self {
        let min = MERGE_FLOOR + 0.01;
        let clamped = floor.clamp(min, 1.0);
        if (clamped - floor).abs() > f32::EPSILON {
            log::warn!(
                "diarization: consolidation_floor {floor} out of range; clamped to {clamped} \
                 (must stay above MERGE_FLOOR {MERGE_FLOOR})"
            );
        }
        self.consolidate_floor = clamped;
        self
    }

    /// Convenience constructor that resolves the cached model paths and ensures
    /// they're present first (downloading on demand). Blocking; run off the UI
    /// thread.
    pub fn from_cached_models() -> Result<Self> {
        let paths = crate::diarization::models::ensure_models(None)?;
        Self::new(&paths.segmentation, &paths.embedding)
    }

    /// Path to the segmentation model this instance loaded.
    pub fn segmentation_path(&self) -> &Path {
        &self.segmentation_path
    }

    /// Path to the embedding model this instance loaded.
    pub fn embedding_path(&self) -> &Path {
        &self.embedding_path
    }
}

impl SherpaDiarizer {
    /// Shared turn-computation used by both [`Diarizer::diarize`] and
    /// [`Diarizer::diarize_with_embeddings`]. Identical logic to the original
    /// `diarize()` body so the offline path is unchanged.
    fn compute_turns(
        &self,
        samples_16k_mono: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<SpeakerTurn>> {
        self.compute_turns_with_progress(samples_16k_mono, sample_rate, None)
    }

    /// Like [`compute_turns`](Self::compute_turns) but threads sherpa's chunk
    /// progress callback through. `on_progress(fraction)` receives a value in
    /// `0.0..=1.0` as segmentation/clustering advances over the file; the engine
    /// runs to completion regardless (we always return `0` "continue" to sherpa).
    fn compute_turns_with_progress(
        &self,
        samples_16k_mono: &[f32],
        sample_rate: u32,
        mut on_progress: Option<&mut dyn FnMut(f32)>,
    ) -> Result<Vec<SpeakerTurn>> {
        if sample_rate != TARGET_SAMPLE_RATE {
            return Err(anyhow!(
                "SherpaDiarizer expects {TARGET_SAMPLE_RATE} Hz mono, got {sample_rate} Hz; \
                 resample before calling diarize()"
            ));
        }
        // Cheap silence/near-silence pre-check (covers the empty case too): a
        // silent system channel means no remote speakers, so skip sherpa — which
        // would otherwise hard-error with "No segments found" — and return zero
        // turns. The alignment stage then labels every transcript segment local.
        if is_silent(samples_16k_mono) {
            let peak = samples_16k_mono
                .iter()
                .fold(0.0_f32, |m, &s| m.max(s.abs()));
            log::info!(
                "diarization: system channel silent/near-silent (peak={peak:.2e} < {SILENCE_PEAK_THRESHOLD:.0e}), \
                 no remote speakers — labeling all segments local"
            );
            return Ok(Vec::new());
        }

        // `compute` takes ownership of the sample buffer.
        let samples = samples_16k_mono.to_vec();

        // specs/0024 WS4.1 diagnostics: the "Identifying speakers" indicator reportedly still
        // sticks at 0%. The emit chain is wired correctly, so the suspect is sherpa's per-chunk
        // callback never advancing (a reported `total <= 0`, or the callback firing only at the
        // end). Count invocations + capture the reported total so a real run reveals the cause.
        let cb_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cb_last_total = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(-1));
        let cb_calls_cb = cb_calls.clone();
        let cb_last_total_cb = cb_last_total.clone();

        let progress_cb = on_progress.as_mut().map(|sink| {
            // SAFETY: `compute` below blocks until sherpa is done and sherpa
            // invokes the callback inline, so the stack-local sink outlives it.
            unsafe {
                crate::diarization::sherpa_sys::fraction_progress_callback(
                    *sink,
                    cb_calls_cb.clone(),
                    cb_last_total_cb.clone(),
                )
            }
        });

        let segments = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| anyhow!("sherpa diarizer mutex poisoned"))?;
            match guard.compute(samples, progress_cb) {
                Ok(segments) => segments,
                // Defensive: audio that passed the silence gate but still has no
                // detectable speech makes sherpa's segmentation return its empty
                // result as an error. Treat that as zero remote speakers (same as
                // the silent case) rather than a scary user-facing failure. We
                // match the message narrowly and re-raise anything that doesn't
                // look like the empty-result case, so genuine model failures
                // still surface. (Model-load/init errors come from
                // `ThreadedDiarize::new`, a separate path, so they can't reach here.)
                Err(e) if e.to_string().contains(NO_SEGMENTS_MARKER) => {
                    log::warn!(
                        "diarization: sherpa found no speech segments in system channel \
                         ({e}) — treating as no remote speakers"
                    );
                    return Ok(Vec::new());
                }
                Err(e) => {
                    return Err(anyhow!("sherpa diarization compute failed: {e}"));
                }
            }
        };

        // WS4.1 diagnostics: if progress was requested but the callback never advanced, the
        // indicator would sit at 0% for the whole run. Surface exactly why on a real run.
        if on_progress.is_some() {
            let calls = cb_calls.load(std::sync::atomic::Ordering::Relaxed);
            let last_total = cb_last_total.load(std::sync::atomic::Ordering::Relaxed);
            if calls == 0 {
                log::warn!(
                    "diarization progress: sherpa never invoked the chunk callback (0 calls) — \
                     the indicator cannot advance past 0% (WS4.1)"
                );
            } else if last_total <= 0 {
                log::warn!(
                    "diarization progress: sherpa reported total<=0 ({last_total}) across {calls} \
                     callback(s) — progress stays at 0% because frac can't be computed (WS4.1)"
                );
            } else {
                log::info!(
                    "diarization progress: sherpa chunk callback fired {calls} time(s), last \
                     total={last_total} (WS4.1 diagnostics)"
                );
            }
        }

        // Guardrail (specs/0011): in Auto mode, an absurd cluster count for one
        // recording means pathological over-clustering (the bug this work fixes).
        // We never silently truncate — that would drop real speakers — but we log
        // loudly so it's visible and recommend the deterministic escape hatch.
        if matches!(self.speaker_count, SpeakerCount::Auto) {
            let distinct: std::collections::HashSet<i32> =
                segments.iter().map(|s| s.speaker).collect();
            if distinct.len() > OVER_CLUSTER_WARN_CAP {
                log::warn!(
                    "diarization: Auto clustering produced {} distinct speakers for one \
                     recording (> {OVER_CLUSTER_WARN_CAP}) — likely over-clustering. \
                     Consider setting an expected speaker count in Settings (forces an \
                     exact count) or merging speakers in the meeting view.",
                    distinct.len()
                );
            }
        }

        // sherpa returns recording-relative seconds + an integer cluster id.
        // Map the cluster id to a stable per-meeting speaker key.
        let turns = segments
            .into_iter()
            .map(|s| SpeakerTurn {
                start: s.start,
                end: s.end,
                speaker: speaker_key_for_cluster(s.speaker),
            })
            .collect();

        Ok(turns)
    }

    /// Diarize, reporting real progress via `on_progress(fraction)` (0.0..=1.0)
    /// as sherpa's segmentation/clustering advances over the file. The closure is
    /// invoked synchronously on the calling thread; keep it cheap. Used by the
    /// offline pipeline to drive the `diarization-progress` percentage.
    pub fn diarize_with_progress(
        &self,
        samples_16k_mono: &[f32],
        sample_rate: u32,
        mut on_progress: impl FnMut(f32),
    ) -> Result<Vec<SpeakerTurn>> {
        self.compute_turns_with_progress(samples_16k_mono, sample_rate, Some(&mut on_progress))
    }
}

/// Distinct speaker keys present in `turns`. The turns are the source of truth for
/// "what clusters exist" (the embedding map can be a sub/superset). Shared by the cap
/// path, the consolidation path, and the diarize-with-embeddings logging.
fn distinct_speaker_count(turns: &[SpeakerTurn]) -> usize {
    turns
        .iter()
        .map(|t| t.speaker.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// A working cluster during a centroid-merge pass: its surviving key, its centroid
/// (absent when the cluster was too short to embed), and its pooled speech duration
/// (the merge weight). Shared by [`merge_clusters_to_at_most`] and
/// [`consolidate_clusters`].
struct Cluster {
    key: String,
    centroid: Option<Vec<f32>>,
    duration: f32,
}

/// Build the deterministic starting cluster list from `turns` + `embeddings`: one
/// entry per distinct speaker key, its centroid (if any), and its summed duration,
/// sorted by key so a merge pass is order-stable.
fn build_clusters(
    turns: &[SpeakerTurn],
    embeddings: &std::collections::HashMap<String, Vec<f32>>,
) -> Vec<Cluster> {
    use std::collections::HashMap;
    let mut by_key: HashMap<&str, f32> = HashMap::new();
    for t in turns {
        *by_key.entry(t.speaker.as_str()).or_insert(0.0) += t.duration();
    }
    let mut v: Vec<Cluster> = by_key
        .into_iter()
        .map(|(key, duration)| Cluster {
            key: key.to_string(),
            centroid: embeddings.get(key).cloned(),
            duration,
        })
        .collect();
    v.sort_by(|a, b| a.key.cmp(&b.key));
    v
}

/// Duration-weighted centroid combine (both sides carry a centroid). Weight by pooled
/// speech duration (rounded to whole units, min 1) so the merged centroid leans toward
/// the longer-speaking cluster, then re-`l2_normalize`. The one true definition of the
/// combine — the cap and consolidation paths must stay bit-identical, so they both call
/// this rather than copy the math.
fn combine_centroids(sc: &[f32], sd: f32, lc: &[f32], ld: f32) -> Vec<f32> {
    use crate::diarization::embedding::{l2_normalize, weighted_mean};
    let sw = sd.max(0.0).round().max(1.0) as u32;
    let lw = ld.max(0.0).round().max(1.0) as u32;
    l2_normalize(&weighted_mean(sc, sw, lc, lw))
}

/// Why [`merge_closest_pairs`] stopped — lets the caller log the reason (the cap path
/// distinguishes them; consolidation ignores it).
enum MergeStop {
    /// `keep_merging(len)` returned false — the caller's target was reached.
    TargetReached,
    /// Fewer than two remaining clusters carry a centroid — nothing left to compare.
    NoComparablePair,
    /// The closest mergeable pair's centroid similarity is below `floor` — refusing to
    /// fuse two clearly-distinct voices. Carries that best similarity for logging.
    BelowFloor(f32),
}

/// The shared closest-pair centroid-merge primitive behind both [`merge_clusters_to_at_most`]
/// (bounded by a count cap) and [`consolidate_clusters`] (bounded by a similarity floor).
///
/// Repeatedly: find the mutually-closest pair that both carry centroids, and — while
/// `keep_merging(clusters.len())` holds and that pair's [`cosine_similarity`] clears
/// its floor — fuse them. A pair's floor is `floor`, raised to
/// `floor + survivor_floor_bonus` when the pair's would-be survivor has already
/// absorbed a merge in this call — the specs/0041 WS1 snowball guard against a
/// duration-weighted centroid dragging toward the dominant cluster and absorbing
/// everyone (pass `0.0` to disable, as the cap path does). The surviving key is the
/// lexicographically-smaller of the pair (deterministic), centroids combine via
/// [`combine_centroids`] (duration-weighted), durations sum, and `remap` records
/// loser→survivor (redirecting any earlier merges that pointed at the loser). Mutates
/// `clusters`/`remap` in place; returns why it stopped so the caller can log/branch.
///
/// [`cosine_similarity`]: crate::diarization::embedding::cosine_similarity
fn merge_closest_pairs(
    clusters: &mut Vec<Cluster>,
    remap: &mut std::collections::HashMap<String, String>,
    floor: f32,
    survivor_floor_bonus: f32,
    mut keep_merging: impl FnMut(usize) -> bool,
) -> MergeStop {
    use crate::diarization::embedding::cosine_similarity;
    // Keys that absorbed a merge in this call (snowball guard). Survivor keys are
    // stable identifiers here: a survivor keeps its key until it is itself consumed,
    // at which point it leaves `clusters` and its taint entry is simply unreachable.
    let mut absorbed: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        if !keep_merging(clusters.len()) {
            return MergeStop::TargetReached;
        }
        // Most-similar *eligible* pair (both must have centroids and the similarity
        // must clear the pair's floor); deterministic on ties via the `sim > prev`
        // strict comparison over the key-sorted `clusters`. `best_any` tracks the
        // closest comparable pair regardless of eligibility, for the stop reason.
        let mut best: Option<(usize, usize, f32)> = None;
        let mut best_any: Option<f32> = None;
        for (i, a) in clusters.iter().enumerate() {
            let Some(ci) = a.centroid.as_deref() else {
                continue;
            };
            for (j, b) in clusters.iter().enumerate().skip(i + 1) {
                let Some(cj) = b.centroid.as_deref() else {
                    continue;
                };
                let sim = cosine_similarity(ci, cj);
                if best_any.is_none_or(|prev| sim > prev) {
                    best_any = Some(sim);
                }
                // The would-be survivor is the lexicographically-smaller key; one
                // that already absorbed a merge must clear the raised floor.
                let survivor_key = if a.key <= b.key { &a.key } else { &b.key };
                let pair_floor = if absorbed.contains(survivor_key.as_str()) {
                    floor + survivor_floor_bonus
                } else {
                    floor
                };
                if sim >= pair_floor && best.is_none_or(|(_, _, prev)| sim > prev) {
                    best = Some((i, j, sim));
                }
            }
        }

        let Some((i, j, _sim)) = best else {
            return match best_any {
                // No pair carried two centroids — nothing left to compare.
                None => MergeStop::NoComparablePair,
                // Comparable pairs exist but none cleared its floor; carry the best
                // similarity for logging (with a zero bonus this is exactly the old
                // "best pair is below `floor`" stop).
                Some(sim) => MergeStop::BelowFloor(sim),
            };
        };

        // Merge loser into survivor, keeping the lexicographically-smaller key so output
        // ordering is deterministic and stable.
        let (survivor_idx, loser_idx) = if clusters[i].key <= clusters[j].key {
            (i, j)
        } else {
            (j, i)
        };
        let loser = clusters.remove(loser_idx);
        // `survivor_idx` shifts left by one if the loser was before it.
        let survivor_idx = if loser_idx < survivor_idx {
            survivor_idx - 1
        } else {
            survivor_idx
        };
        let survivor = &mut clusters[survivor_idx];
        if let (Some(sc), Some(lc)) = (survivor.centroid.as_deref(), loser.centroid.as_deref()) {
            survivor.centroid = Some(combine_centroids(sc, survivor.duration, lc, loser.duration));
        }
        survivor.duration += loser.duration;

        // Record the relabel loser→survivor, redirecting any earlier merges that
        // survived under the loser's key — and taint the survivor for the snowball
        // guard (its centroid now carries a merge's worth of drift).
        let survivor_key = survivor.key.clone();
        absorbed.insert(survivor_key.clone());
        for v in remap.values_mut() {
            if *v == loser.key {
                *v = survivor_key.clone();
            }
        }
        remap.insert(loser.key.clone(), survivor_key);
    }
}

/// Apply `remap` (original key → surviving key / [`UNKNOWN_SPEAKER_KEY`]) to the turns
/// — order and timing preserved, only the `speaker` label changes — and rebuild the
/// embedding map from the surviving `clusters`' centroids (one entry per surviving key
/// that has one; the mixed Unknown bucket carries none). When `remap` is empty the
/// inputs are returned **byte-identically**, so a no-op merge pass is provably a no-op.
fn apply_remap(
    turns: Vec<SpeakerTurn>,
    embeddings: std::collections::HashMap<String, Vec<f32>>,
    remap: &std::collections::HashMap<String, String>,
    clusters: Vec<Cluster>,
) -> (
    Vec<SpeakerTurn>,
    std::collections::HashMap<String, Vec<f32>>,
) {
    if remap.is_empty() {
        return (turns, embeddings);
    }
    let out_turns: Vec<SpeakerTurn> = turns
        .into_iter()
        .map(|t| SpeakerTurn {
            start: t.start,
            end: t.end,
            speaker: remap.get(&t.speaker).cloned().unwrap_or(t.speaker),
        })
        .collect();
    let mut out_emb: std::collections::HashMap<String, Vec<f32>> = std::collections::HashMap::new();
    for c in clusters {
        if let Some(centroid) = c.centroid {
            out_emb.insert(c.key, centroid);
        }
    }
    (out_turns, out_emb)
}

/// Enforce an "at most `max_speakers`" cap on an already-clustered diarization
/// result by **post-cluster centroid merge** (specs/0017). Pure: no I/O, no model,
/// no audio — unit-testable in isolation.
///
/// `turns` are the per-cluster speaker turns (keyed by `spk_N`); `embeddings` maps
/// each cluster key to its L2-normalized centroid (as produced by
/// [`crate::diarization::embedding::ClusterEmbedder`]). Returns the relabeled turns
/// and the rebuilt embedding map so downstream voiceprint/identity sees the merged
/// clusters.
///
/// Algorithm:
/// 1. If `max_speakers == 0` ("no cap") or the distinct cluster count is already
///    `<= max_speakers`, the inputs are returned **unchanged**.
/// 2. Otherwise, iteratively: compute pairwise [`cosine_similarity`] between the
///    current cluster centroids, pick the **most-similar pair**, and merge it —
///    relabel the loser's turns to the winner's key, combine their centroids with
///    [`weighted_mean`] weighted by each cluster's total speech duration, and
///    re-`l2_normalize`. Repeat until `<= max_speakers` distinct clusters remain.
/// 3. **Merge-floor guard:** never merge a pair whose similarity is below
///    [`MERGE_FLOOR`]. If the best available merge is below the floor, STOP merging
///    (better to over-count than fabricate a merge of two genuinely different
///    voices); the reason is logged.
/// 4. **Overflow-as-Unknown (WS3.3, specs/0029, owner-approved):** if step 3 left
///    more than `max_speakers` clusters, keep the top `max_speakers` clusters by
///    total speech duration as first-class speakers and relabel every remaining
///    cluster to the single [`UNKNOWN_SPEAKER_KEY`] bucket ("Unknown speaker"). The
///    bucket mixes distinct voices, so its entry is dropped from the embedding map
///    (no voiceprint for a mixed bucket). The result then has at most
///    `max_speakers + 1` distinct keys (`n` named + one "Unknown speaker").
///
/// Determinism: the surviving key of a merged pair is always the one that sorts
/// first (`min` by key), and ties in similarity break toward the lexicographically
/// smaller pair, so the same input always yields the same output keys. Turn order
/// and timing are preserved — only the `speaker` label changes.
///
/// Clusters with **no centroid** in `embeddings` (too short to embed) can't be
/// compared by cosine; they are kept as-is and still count toward the cap. If they
/// alone exceed `max_speakers` we stop without dropping any real speaker.
///
/// [`cosine_similarity`]: crate::diarization::embedding::cosine_similarity
/// [`weighted_mean`]: crate::diarization::embedding::weighted_mean
pub fn merge_clusters_to_at_most(
    turns: Vec<SpeakerTurn>,
    embeddings: std::collections::HashMap<String, Vec<f32>>,
    max_speakers: u32,
) -> (
    Vec<SpeakerTurn>,
    std::collections::HashMap<String, Vec<f32>>,
) {
    use std::collections::HashMap;

    // The turns are the source of truth for "what clusters exist" (the embedding map
    // can be a super/subset). Nothing to cap if we're already at/under the bound.
    let max = max_speakers as usize;
    if max == 0 || distinct_speaker_count(&turns) <= max {
        return (turns, embeddings);
    }

    let mut clusters = build_clusters(&turns, &embeddings);
    let mut remap: HashMap<String, String> = HashMap::new();

    // Merge the closest cluster pair while we're still above the cap and the pair is
    // above the anti-false-merge floor — the shared primitive does the merge/remap/
    // centroid-combine; we just supply the count target and read back why it stopped.
    // No snowball bonus (`0.0`): the cap merge is a user-requested bound, and raising
    // repeat-survivor floors here would only push more clusters into the Unknown
    // overflow bucket.
    let stop = merge_closest_pairs(&mut clusters, &mut remap, MERGE_FLOOR, 0.0, |len| len > max);

    // WS3.3 overflow-as-Unknown (specs/0029): the merge stopped above the bound (merge
    // floor hit, or too few centroids to compare). Rather than presenting all of them
    // as first-class speakers, keep the top `max` by total speech duration and fold the
    // remainder into the single "Unknown speaker" bucket.
    if clusters.len() > max {
        match stop {
            MergeStop::NoComparablePair => log::info!(
                "diarization: AtMost cap could not be reached by merging — {} clusters remain \
                 (> {max}) but fewer than two have embeddings to merge; overflow policy applies.",
                clusters.len()
            ),
            MergeStop::BelowFloor(sim) => log::info!(
                "diarization: AtMost cap not reached by merging — best remaining centroid \
                 similarity {sim:.3} is below the merge floor {MERGE_FLOOR:.2} with {} distinct \
                 speakers (> {max}); overflow policy applies.",
                clusters.len()
            ),
            MergeStop::TargetReached => {}
        }

        let before = clusters.len();
        // Longest-speaking clusters survive; deterministic tie-break on key.
        clusters.sort_by(|a, b| {
            b.duration
                .total_cmp(&a.duration)
                .then_with(|| a.key.cmp(&b.key))
        });
        let overflow = clusters.split_off(max);
        for c in &overflow {
            // Redirect any earlier merges that survived under an overflow key.
            for v in remap.values_mut() {
                if *v == c.key {
                    *v = UNKNOWN_SPEAKER_KEY.to_string();
                }
            }
            remap.insert(c.key.clone(), UNKNOWN_SPEAKER_KEY.to_string());
        }
        log::warn!(
            "diarization: AtMost({max}) bound not directly honored — {before} clusters remained \
             after centroid merge; keeping the top {max} by speech duration and bucketing \
             {} cluster(s) as \"Unknown speaker\" (WS3.3 overflow policy).",
            overflow.len()
        );
    }

    apply_remap(turns, embeddings, &remap, clusters)
}

/// Tuning inputs for [`consolidate_clusters`]. Pure value type so the pass is
/// unit-testable with hand-built fixtures.
#[derive(Debug, Clone, Copy)]
pub struct ConsolidateOpts {
    /// Fuse cluster pairs whose centroid cosine similarity is `>= floor`
    /// ([`CONSOLIDATE_FLOOR`] by default).
    pub floor: f32,
    /// specs/0041 WS1 — never consolidate the distinct-cluster count below this bound.
    /// `AtMost(n)` mode passes `Some(n)` (clamped to at least 2 inside the pass): the
    /// calendar/attendee seed is a consolidation *floor*, not just an upper cap, and
    /// consolidation runs before the cap so an over-merge here could never be undone.
    /// `None` (Auto mode, and the default) bounds the cascade by the per-pass merge
    /// budget instead — at most `ceil(clusters / 2)` merges (halves the count, rounding
    /// up), since a drift-split repair can at most halve the number of *voices*.
    pub min_clusters: Option<usize>,
}

impl Default for ConsolidateOpts {
    fn default() -> Self {
        Self {
            floor: CONSOLIDATE_FLOOR,
            min_clusters: None,
        }
    }
}

/// Global, **bounded same-voice consolidation** over an already-clustered
/// diarization result (WS1, specs/0039; cascade bounds from specs/0041 WS1). Pure:
/// no I/O, no model, no audio — unit-testable in isolation, exactly like
/// [`merge_clusters_to_at_most`].
///
/// Repairs the "long-meeting drift" symptom where sherpa's single global clustering
/// pass splits **one** voice into an early + a late cluster:
///
/// 1. **Same-voice merge (threshold-driven, bounded).** Repeatedly fuse the
///    mutually-closest cluster pair **while** their centroid [`cosine_similarity`] is
///    `>= opts.floor`. Unlike [`merge_clusters_to_at_most`], this is driven by the
///    similarity floor, not a target count — but the cascade is *bounded* (specs/0041
///    WS1; the original open-ended pass snowballed whole meetings into the dominant
///    speaker): with an `AtMost(n)` seed it never consolidates below
///    `max(opts.min_clusters, 2)`, and in Auto mode it performs at most
///    `ceil(clusters / 2)` merges per pass — halves the count, rounding up, so an odd
///    drift-split count (3 same-voice clusters) can still collapse to 1. This is the
///    drift-split repair, sharing [`merge_closest_pairs`] with the cap path.
/// 2. **Duration-weighted centroids, snowball-guarded.** Every merge combines
///    centroids with [`weighted_mean`] weighted by pooled speech duration, then
///    re-[`l2_normalize`]s — identical math to the cap path, so a consolidated
///    centroid leans toward the longer-speaking half and stays comparable for the
///    cross-meeting matcher. Because that lean is exactly what drags a dominant
///    cluster toward everyone else, a cluster that already absorbed a merge this pass
///    only survives another if the new pair clears
///    `opts.floor + CONSOLIDATE_SNOWBALL_MARGIN` (specs/0041 WS1).
///
/// Consolidation only *merges* drift-split same-voice clusters; it never *deletes* a
/// distinct speaker. A genuinely brief-but-real speaker (e.g. one short "yes, agreed")
/// whose centroid is clearly its own voice is preserved as a first-class cluster so it
/// can still be labeled/voiceprinted downstream — earlier revisions suppressed any
/// sub-1s cluster to the Unknown bucket, which erased real brief speakers; that pass
/// was removed (specs/0039 WS1 review).
///
/// Determinism mirrors [`merge_clusters_to_at_most`]: the surviving key of a merged
/// pair is the lexicographically-smaller one, and turn order/timing is preserved (only
/// the `speaker` label changes). When nothing merges (already-distinct clusters), the
/// inputs are returned **byte-identical** so a clean call is a provable no-op. Clusters
/// without a centroid (too short to embed) can't be compared and are left as-is.
///
/// Runs *before* the [`SpeakerCount::AtMost`] cap in `diarize_with_embeddings`
/// (same-voice cleanup first, then the cap enforces its upper bound — the two compose,
/// and consolidation legitimately dropping the count below the cap is fine: the cap is
/// an upper bound). Not run in `Fixed(n)` mode (the user forced an exact count).
///
/// [`cosine_similarity`]: crate::diarization::embedding::cosine_similarity
/// [`weighted_mean`]: crate::diarization::embedding::weighted_mean
/// [`l2_normalize`]: crate::diarization::embedding::l2_normalize
pub fn consolidate_clusters(
    turns: Vec<SpeakerTurn>,
    embeddings: std::collections::HashMap<String, Vec<f32>>,
    opts: ConsolidateOpts,
) -> (
    Vec<SpeakerTurn>,
    std::collections::HashMap<String, Vec<f32>>,
) {
    use std::collections::HashMap;

    // Nothing to consolidate with 0/1 distinct clusters.
    if distinct_speaker_count(&turns) <= 1 {
        return (turns, embeddings);
    }

    let mut clusters = build_clusters(&turns, &embeddings);
    let mut remap: HashMap<String, String> = HashMap::new();

    // Same-voice threshold merge, with the specs/0041 WS1 cascade bounds: an AtMost
    // seed is a count floor (never below max(n, 2)); Auto instead budgets
    // ceil(initial / 2) merges — halves the count, rounding up, so an odd drift-split
    // count (e.g. 3 clusters of one voice) can still collapse fully instead of
    // stalling one merge short. The snowball guard (raised floor for a repeat
    // survivor) rides along inside the shared primitive. No suppression of brief
    // distinct speakers: consolidation merges, it never deletes a real speaker
    // (specs/0039 WS1 review).
    let initial = clusters.len();
    let (count_floor, merge_budget) = match opts.min_clusters {
        Some(min) => (min.max(2), usize::MAX),
        None => (1, initial.div_ceil(2)),
    };
    merge_closest_pairs(
        &mut clusters,
        &mut remap,
        opts.floor,
        CONSOLIDATE_SNOWBALL_MARGIN,
        |len| len > count_floor && initial - len < merge_budget,
    );

    apply_remap(turns, embeddings, &remap, clusters)
}

impl Diarizer for SherpaDiarizer {
    fn diarize(&self, samples_16k_mono: &[f32], sample_rate: u32) -> Result<Vec<SpeakerTurn>> {
        self.compute_turns(samples_16k_mono, sample_rate)
    }

    /// Override the trait default: diarize, then re-embed each remote cluster's
    /// pooled speech with the loaded embedding ONNX (sherpa-rs exposes no cluster
    /// centroids — see `embedding.rs` for the spike outcome). CPU-only, matching the
    /// diarizer's posture. If the extractor can't be created, we still return the
    /// turns (with an empty embedding map) — embeddings are an enhancement, not a
    /// hard requirement, so a missing extractor must not fail diarization.
    fn diarize_with_embeddings(
        &self,
        samples_16k_mono: &[f32],
        sample_rate: u32,
    ) -> Result<crate::diarization::TurnsWithEmbeddings> {
        // Delegate to the progress-aware variant with a no-op sink so there is a single
        // implementation of the embedding/merge logic.
        self.diarize_with_embeddings_with_progress(samples_16k_mono, sample_rate, &mut |_| {})
    }

    /// specs/0019 WS2.5 — same as `diarize_with_embeddings` but threads sherpa's
    /// per-chunk progress through `compute_turns_with_progress` so the offline pipeline
    /// can drive a real "Identifying speakers" percentage.
    fn diarize_with_embeddings_with_progress(
        &self,
        samples_16k_mono: &[f32],
        sample_rate: u32,
        on_progress: &mut dyn FnMut(f32),
    ) -> Result<crate::diarization::TurnsWithEmbeddings> {
        let turns =
            self.compute_turns_with_progress(samples_16k_mono, sample_rate, Some(on_progress))?;
        if turns.is_empty() {
            return Ok((turns, std::collections::HashMap::new()));
        }

        let mut guard = self
            .embedder
            .lock()
            .map_err(|_| anyhow!("sherpa embedder mutex poisoned"))?;
        // Lazily create the extractor on first use; cache the failure so we don't
        // re-attempt (and re-log) every pass.
        if guard.is_none() {
            *guard = Some(
                match crate::diarization::embedding::ClusterEmbedder::new(&self.embedding_path) {
                    Ok(e) => Some(e),
                    Err(e) => {
                        log::warn!(
                            "diarization: could not init embedding extractor ({e}); \
                             continuing without per-cluster embeddings"
                        );
                        None
                    }
                },
            );
        }

        let embeddings = match guard.as_mut().and_then(|o| o.as_mut()) {
            Some(embedder) => embedder.embeddings_for_turns(&turns, samples_16k_mono),
            None => std::collections::HashMap::new(),
        };
        // Release the embedder lock before the (CPU-only, pure) consolidation/merge.
        drop(guard);

        // Live windowed pass: partial audio, so no consolidation/audio-seed — preserve
        // old behavior (honor an AtMost(n) cap directly; the offline pass reconciles).
        if !self.consolidate_enabled {
            return Ok(match self.speaker_count {
                SpeakerCount::AtMost(n) => merge_clusters_to_at_most(turns, embeddings, n),
                _ => (turns, embeddings),
            });
        }
        // Fixed(n): user forced an exact count at clustering — untouched (specs/0039).
        if let SpeakerCount::Fixed(_) = self.speaker_count {
            return Ok((turns, embeddings));
        }

        // specs/0039 WS1 consolidation, Auto budget in all non-Fixed modes (specs/0050:
        // the invite is a CEILING applied below, not a consolidation floor).
        let before = distinct_speaker_count(&turns);
        let opts = ConsolidateOpts {
            floor: self.consolidate_floor,
            min_clusters: None,
        };
        let (turns, embeddings) = consolidate_clusters(turns, embeddings, opts);
        let after = distinct_speaker_count(&turns);
        if after != before {
            log::info!("diarization: consolidation adjusted {before} clusters to {after} (drift-split repair, floor {:.2}; specs/0039 WS1).", self.consolidate_floor);
        }

        // specs/0050: cap to an AUDIO-DERIVED count. Auto -> AtMost(n_audio);
        // AtMost(invite) -> AtMost(min(n_audio, invite)) — the clean invite is only a
        // ceiling, so a dist-list under-count / big over-count can't distort the result.
        let n_audio = crate::diarization::seed::estimate_speakers_by_duration(
            &turns,
            crate::diarization::seed::N_AUDIO_MIN_SECS,
        );
        let cap = match self.speaker_count {
            SpeakerCount::AtMost(n) => n_audio.min(n as usize),
            _ => n_audio, // Auto
        }
        .max(2) as u32;
        let before = distinct_speaker_count(&turns);
        let (turns, embeddings) = merge_clusters_to_at_most(turns, embeddings, cap);
        let after = distinct_speaker_count(&turns);
        if after < before {
            log::info!(
                "diarization: audio seed AtMost({cap}) merged {before} clusters down to {after} \
                 (n_audio ≥ {}s; specs/0050).",
                crate::diarization::seed::N_AUDIO_MIN_SECS
            );
        }
        Ok((turns, embeddings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_is_silent() {
        assert!(is_silent(&[]));
    }

    #[test]
    fn all_zeros_is_silent() {
        assert!(is_silent(&[0.0f32; 16_000]));
    }

    #[test]
    fn below_threshold_is_silent() {
        // ~-90 dBFS digital silence / idle line noise, like the real `system.wav`
        // that triggered the bug — must be treated as no remote speech.
        let near_silent = vec![3.0e-5f32; 16_000];
        assert!(is_silent(&near_silent));
    }

    #[test]
    fn speech_level_is_not_silent() {
        // A sample above the -60 dBFS gate must NOT be skipped.
        let mut buf = vec![0.0f32; 16_000];
        buf[8_000] = 0.2; // ~-14 dBFS peak
        assert!(!is_silent(&buf));
    }

    #[test]
    fn no_segments_marker_matches_real_error_text() {
        // Guards the defensive `compute()` mapping against drift in the message
        // sherpa-rs surfaces for the empty-result case.
        let err = "sherpa diarization compute failed: No segments found or invalid pointer";
        assert!(err.contains(NO_SEGMENTS_MARKER));
    }

    // -----------------------------------------------------------------------
    // merge_clusters_to_at_most (specs/0017) — pure, no model/audio.
    // -----------------------------------------------------------------------
    mod merge {
        use super::*;
        use crate::diarization::embedding::l2_normalize;
        use std::collections::{HashMap, HashSet};

        /// Build a turn for `key` of `dur` seconds, packed back-to-back per key.
        fn turn(key: &str, start: f32, dur: f32) -> SpeakerTurn {
            SpeakerTurn {
                start,
                end: start + dur,
                speaker: key.to_string(),
            }
        }

        fn distinct_keys(turns: &[SpeakerTurn]) -> HashSet<String> {
            turns.iter().map(|t| t.speaker.clone()).collect()
        }

        /// A near-orthonormal basis vector in `dim` dims with `1.0` at `axis`,
        /// plus a small wobble so two clusters on the *same* axis are very similar
        /// but not byte-identical (realistic same-speaker split).
        fn axis_vec(dim: usize, axis: usize, wobble: f32) -> Vec<f32> {
            let mut v = vec![0.0f32; dim];
            v[axis] = 1.0;
            // tiny off-axis component to make distinct clusters non-identical
            v[(axis + 1) % dim] += wobble;
            l2_normalize(&v)
        }

        #[test]
        fn five_clusters_three_groups_atmost3_merges_to_3() {
            // 5 raw clusters along 3 axes: {A0,A1} same voice, {B0,B1} same voice,
            // {C0} alone. AtMost(3) must collapse to exactly 3 groups, each the
            // union of its same-axis members.
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.02)), // A0
                ("spk_1".into(), axis_vec(dim, 0, 0.05)), // A1 (same axis as A0)
                ("spk_2".into(), axis_vec(dim, 3, 0.02)), // B0
                ("spk_3".into(), axis_vec(dim, 3, 0.05)), // B1 (same axis as B0)
                ("spk_4".into(), axis_vec(dim, 6, 0.02)), // C0 (alone)
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 5.0),
                turn("spk_1", 5.0, 5.0),
                turn("spk_2", 10.0, 5.0),
                turn("spk_3", 15.0, 5.0),
                turn("spk_4", 20.0, 5.0),
            ];

            let (out_turns, out_emb) = merge_clusters_to_at_most(turns, emb, 3);
            let keys = distinct_keys(&out_turns);
            assert_eq!(keys.len(), 3, "should merge down to exactly 3 groups");
            // Embedding map reflects the merge: one centroid per surviving key.
            assert_eq!(out_emb.len(), 3);
            assert_eq!(out_emb.keys().cloned().collect::<HashSet<_>>(), keys);

            // The two A members must share a key; the two B members must share a key.
            // Recover each merged member's surviving key by its original time slot.
            let at = |start: f32| {
                out_turns
                    .iter()
                    .find(|t| (t.start - start).abs() < 1e-6)
                    .unwrap()
                    .speaker
                    .clone()
            };
            assert_eq!(at(0.0), at(5.0), "A0 and A1 (same axis) must merge");
            assert_eq!(at(10.0), at(15.0), "B0 and B1 (same axis) must merge");
            // C0 must be its own group, distinct from A and B.
            assert_ne!(at(20.0), at(0.0));
            assert_ne!(at(20.0), at(10.0));
            assert_ne!(at(0.0), at(10.0));
        }

        #[test]
        fn distinct_count_at_or_below_cap_is_unchanged() {
            let dim = 4;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), axis_vec(dim, 1, 0.0)),
            ]);
            let turns = vec![turn("spk_0", 0.0, 3.0), turn("spk_1", 3.0, 3.0)];
            let (out_turns, out_emb) = merge_clusters_to_at_most(turns.clone(), emb.clone(), 3);
            assert_eq!(out_turns, turns, "<= cap returns turns unchanged");
            assert_eq!(out_emb, emb, "<= cap returns embeddings unchanged");
        }

        #[test]
        fn atmost_zero_means_no_cap_unchanged() {
            let dim = 4;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), axis_vec(dim, 1, 0.0)),
                ("spk_2".into(), axis_vec(dim, 2, 0.0)),
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 3.0),
                turn("spk_1", 3.0, 3.0),
                turn("spk_2", 6.0, 3.0),
            ];
            let (out_turns, out_emb) = merge_clusters_to_at_most(turns.clone(), emb.clone(), 0);
            assert_eq!(out_turns, turns, "AtMost(0) = no cap, turns unchanged");
            assert_eq!(out_emb, emb, "AtMost(0) = no cap, embeddings unchanged");
        }

        #[test]
        fn merge_floor_blocks_forcing_distinct_voices() {
            // 3 mutually near-orthogonal clusters (all pairwise similarity ~0,
            // below MERGE_FLOOR). AtMost(1) must NOT fuse them into one voice; the
            // WS3.3 overflow policy keeps the longest-speaking cluster first-class
            // and buckets the rest as one "Unknown speaker".
            let dim = 6;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), axis_vec(dim, 2, 0.0)),
                ("spk_2".into(), axis_vec(dim, 4, 0.0)),
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 3.0),
                turn("spk_1", 3.0, 5.0), // longest talker — must stay first-class
                turn("spk_2", 8.0, 3.0),
            ];
            let (out_turns, out_emb) = merge_clusters_to_at_most(turns, emb, 1);
            let keys = distinct_keys(&out_turns);
            assert_eq!(
                keys,
                HashSet::from(["spk_1".to_string(), UNKNOWN_SPEAKER_KEY.to_string()]),
                "floor must prevent merging; overflow buckets into Unknown"
            );
            // The mixed Unknown bucket must carry no centroid/voiceprint.
            assert!(!out_emb.contains_key(UNKNOWN_SPEAKER_KEY));
            assert_eq!(out_emb.len(), 1);
            assert!(out_emb.contains_key("spk_1"));
        }

        #[test]
        fn overflow_keeps_top_n_by_duration_and_buckets_rest_as_unknown() {
            // WS3.3 (specs/0029): 4 mutually near-orthogonal voices, AtMost(2). No
            // pair clears the merge floor, so the two longest-speaking clusters stay
            // first-class and the two others fold into one "Unknown speaker" bucket
            // → exactly max+1 distinct keys.
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), axis_vec(dim, 2, 0.0)),
                ("spk_2".into(), axis_vec(dim, 4, 0.0)),
                ("spk_3".into(), axis_vec(dim, 6, 0.0)),
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 1.0),  // short → overflow
                turn("spk_1", 1.0, 10.0), // long  → keeps its key
                turn("spk_2", 11.0, 1.5), // short → overflow
                turn("spk_3", 12.5, 8.0), // long  → keeps its key
            ];
            let (out_turns, out_emb) = merge_clusters_to_at_most(turns, emb, 2);

            let keys = distinct_keys(&out_turns);
            assert_eq!(
                keys,
                HashSet::from([
                    "spk_1".to_string(),
                    "spk_3".to_string(),
                    UNKNOWN_SPEAKER_KEY.to_string(),
                ]),
                "top-2 by duration stay first-class; the rest become one Unknown bucket"
            );

            // Turn timing/order preserved; only the labels changed.
            let at = |start: f32| {
                out_turns
                    .iter()
                    .find(|t| (t.start - start).abs() < 1e-6)
                    .unwrap()
                    .speaker
                    .clone()
            };
            assert_eq!(at(0.0), UNKNOWN_SPEAKER_KEY);
            assert_eq!(at(1.0), "spk_1");
            assert_eq!(at(11.0), UNKNOWN_SPEAKER_KEY);
            assert_eq!(at(12.5), "spk_3");

            // Embeddings: survivors keep theirs; the mixed bucket has none.
            assert_eq!(
                out_emb.keys().cloned().collect::<HashSet<_>>(),
                HashSet::from(["spk_1".to_string(), "spk_3".to_string()])
            );
        }

        #[test]
        fn deterministic_same_input_same_output_keys() {
            let dim = 8;
            let build = || {
                let emb: HashMap<String, Vec<f32>> = HashMap::from([
                    ("spk_0".into(), axis_vec(dim, 0, 0.02)),
                    ("spk_1".into(), axis_vec(dim, 0, 0.05)),
                    ("spk_2".into(), axis_vec(dim, 3, 0.02)),
                    ("spk_3".into(), axis_vec(dim, 3, 0.05)),
                    ("spk_4".into(), axis_vec(dim, 6, 0.02)),
                ]);
                let turns = vec![
                    turn("spk_0", 0.0, 5.0),
                    turn("spk_1", 5.0, 5.0),
                    turn("spk_2", 10.0, 5.0),
                    turn("spk_3", 15.0, 5.0),
                    turn("spk_4", 20.0, 5.0),
                ];
                (turns, emb)
            };
            let (t1, e1) = build();
            let (t2, e2) = build();
            let r1 = merge_clusters_to_at_most(t1, e1, 3);
            let r2 = merge_clusters_to_at_most(t2, e2, 3);
            assert_eq!(r1.0, r2.0, "turn labels must be deterministic");
            let k1: HashSet<String> = r1.1.keys().cloned().collect();
            let k2: HashSet<String> = r2.1.keys().cloned().collect();
            assert_eq!(k1, k2, "surviving embedding keys must be deterministic");
        }

        #[test]
        fn merged_centroid_leans_toward_larger_cluster() {
            // Two same-axis clusters with very different durations. The merged
            // centroid must be closer to the LONGER-speaking cluster's centroid
            // than to the shorter one's (duration-weighted combine).
            use crate::diarization::embedding::cosine_similarity;
            let dim = 4;
            let big = axis_vec(dim, 0, 0.02); // long talker
            let small = axis_vec(dim, 0, 0.40); // short talker, more off-axis
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), big.clone()),
                ("spk_1".into(), small.clone()),
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 30.0), // 30s — dominant
                turn("spk_1", 30.0, 2.0), // 2s — minor
            ];
            let (_t, out_emb) = merge_clusters_to_at_most(turns, emb, 1);
            assert_eq!(out_emb.len(), 1);
            let merged = out_emb.values().next().unwrap();
            let to_big = cosine_similarity(merged, &big);
            let to_small = cosine_similarity(merged, &small);
            assert!(
                to_big > to_small,
                "merged centroid should lean toward the longer cluster: \
                 to_big={to_big:.4} should exceed to_small={to_small:.4}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // consolidate_clusters (specs/0039 WS1) — pure, no model/audio.
    // -----------------------------------------------------------------------
    mod consolidate {
        use super::*;
        use crate::diarization::embedding::l2_normalize;
        use std::collections::{HashMap, HashSet};

        fn turn(key: &str, start: f32, dur: f32) -> SpeakerTurn {
            SpeakerTurn {
                start,
                end: start + dur,
                speaker: key.to_string(),
            }
        }

        fn distinct_keys(turns: &[SpeakerTurn]) -> HashSet<String> {
            turns.iter().map(|t| t.speaker.clone()).collect()
        }

        /// Near-orthonormal basis vector at `axis` with a small off-axis wobble, so
        /// two clusters on the *same* axis are very similar (same-speaker drift) but
        /// not byte-identical.
        fn axis_vec(dim: usize, axis: usize, wobble: f32) -> Vec<f32> {
            let mut v = vec![0.0f32; dim];
            v[axis] = 1.0;
            v[(axis + 1) % dim] += wobble;
            l2_normalize(&v)
        }

        /// Unit vector at *exactly* cosine `cos` from the `axis_a` basis vector, with
        /// the orthogonal remainder on `axis_b` — lets a test pin a pairwise
        /// similarity deterministically (handcrafted geometry, no randomness).
        fn vec_at_cos(dim: usize, axis_a: usize, axis_b: usize, cos: f32) -> Vec<f32> {
            assert!(axis_a != axis_b && (0.0..=1.0).contains(&cos));
            let mut v = vec![0.0f32; dim];
            v[axis_a] = cos;
            v[axis_b] = (1.0 - cos * cos).sqrt();
            v
        }

        /// The chosen floor must stay strictly above the anti-false-merge cap floor,
        /// so an automatic consolidation is always more conservative than a
        /// user-requested `AtMost` merge (specs/0039 WS1 safety invariant).
        #[test]
        fn consolidate_floor_is_more_conservative_than_merge_floor() {
            const {
                assert!(
                    CONSOLIDATE_FLOOR > MERGE_FLOOR,
                    "CONSOLIDATE_FLOOR must exceed MERGE_FLOOR"
                );
            }
        }

        /// (a) Drift-split: one voice split into two high-similarity clusters merges
        /// to a single stable key, while a genuinely distinct voice is left apart.
        #[test]
        fn drift_split_same_voice_merges_to_one_key() {
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.02)), // voice A, early
                ("spk_1".into(), axis_vec(dim, 0, 0.05)), // voice A, late (drift-split)
                ("spk_2".into(), axis_vec(dim, 4, 0.02)), // voice B (distinct)
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 40.0),
                turn("spk_1", 40.0, 30.0),
                turn("spk_2", 70.0, 25.0),
            ];
            let (out_turns, out_emb) = consolidate_clusters(turns, emb, ConsolidateOpts::default());

            let keys = distinct_keys(&out_turns);
            assert_eq!(
                keys.len(),
                2,
                "the two drift-split halves collapse to one voice"
            );

            let at = |start: f32| {
                out_turns
                    .iter()
                    .find(|t| (t.start - start).abs() < 1e-6)
                    .unwrap()
                    .speaker
                    .clone()
            };
            assert_eq!(
                at(0.0),
                at(40.0),
                "early + late halves of voice A must share a key"
            );
            assert_ne!(at(0.0), at(70.0), "voice B must stay distinct");
            // One centroid per surviving key.
            assert_eq!(out_emb.keys().cloned().collect::<HashSet<_>>(), keys);
        }

        /// (b) Brief distinct speaker: a short, clearly-distinct cluster (well under the
        /// old 1s suppression threshold, e.g. someone who only says "yes, agreed") is
        /// **preserved** as its own first-class speaker — consolidation merges same-voice
        /// drift-splits, it must never delete a real brief speaker. This is the specs/0039
        /// WS1 review regression fix (the removed Pass 2 used to bucket it as Unknown).
        #[test]
        fn brief_distinct_speaker_is_preserved() {
            let dim = 9;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.02)), // real voice A
                ("spk_1".into(), axis_vec(dim, 3, 0.02)), // real voice B
                ("spk_2".into(), axis_vec(dim, 6, 0.02)), // brief but distinct (near-orthogonal)
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 30.0),
                turn("spk_1", 30.0, 25.0),
                turn("spk_2", 55.0, 0.3), // 300 ms — a brief "yes, agreed"
            ];
            let (out_turns, out_emb) = consolidate_clusters(turns, emb, ConsolidateOpts::default());

            let keys = distinct_keys(&out_turns);
            assert_eq!(
                keys,
                HashSet::from([
                    "spk_0".to_string(),
                    "spk_1".to_string(),
                    "spk_2".to_string(),
                ]),
                "the brief distinct speaker stays first-class (not bucketed as Unknown)"
            );
            let at = |start: f32| {
                out_turns
                    .iter()
                    .find(|t| (t.start - start).abs() < 1e-6)
                    .unwrap()
                    .speaker
                    .clone()
            };
            assert_eq!(at(55.0), "spk_2", "brief speaker keeps its own label");
            assert!(
                !keys.contains(UNKNOWN_SPEAKER_KEY),
                "no Unknown bucketing — nothing was suppressed"
            );
            // The brief speaker keeps a centroid so it can still be labeled/voiceprinted.
            assert!(out_emb.contains_key("spk_2"));
            assert_eq!(
                out_emb.keys().cloned().collect::<HashSet<_>>(),
                HashSet::from([
                    "spk_0".to_string(),
                    "spk_1".to_string(),
                    "spk_2".to_string()
                ])
            );
        }

        /// A distinct 2-speaker meeting must be a true no-op: no pair reaches the floor,
        /// so [`merge_closest_pairs`] merges nothing and the inputs pass through untouched
        /// (guards the `> MERGE_FLOOR` invariant's "clean call is a provable no-op" claim).
        #[test]
        fn clean_two_speakers_pass1_is_noop() {
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), axis_vec(dim, 4, 0.0)), // near-orthogonal → sim well below floor
            ]);
            let turns = vec![turn("spk_0", 0.0, 20.0), turn("spk_1", 20.0, 20.0)];
            let (out_turns, out_emb) =
                consolidate_clusters(turns.clone(), emb.clone(), ConsolidateOpts::default());
            assert_eq!(out_turns, turns);
            assert_eq!(out_emb, emb);
        }

        /// (c) No-op: a clean, already-distinct 2-speaker case is left byte-identical
        /// (consolidation must not touch clusters that are already distinct).
        #[test]
        fn clean_two_speakers_is_byte_identical_noop() {
            let dim = 6;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), axis_vec(dim, 3, 0.0)), // near-orthogonal → distinct
            ]);
            let turns = vec![turn("spk_0", 0.0, 5.0), turn("spk_1", 5.0, 5.0)];
            let (out_turns, out_emb) =
                consolidate_clusters(turns.clone(), emb.clone(), ConsolidateOpts::default());
            assert_eq!(out_turns, turns, "distinct clusters: turns unchanged");
            assert_eq!(out_emb, emb, "distinct clusters: embeddings unchanged");
        }

        /// A single cluster is a trivial no-op (early return before any work).
        #[test]
        fn single_cluster_is_noop() {
            let emb: HashMap<String, Vec<f32>> =
                HashMap::from([("spk_0".into(), axis_vec(4, 0, 0.0))]);
            let turns = vec![turn("spk_0", 0.0, 10.0)];
            let (out_turns, out_emb) =
                consolidate_clusters(turns.clone(), emb.clone(), ConsolidateOpts::default());
            assert_eq!(out_turns, turns);
            assert_eq!(out_emb, emb);
        }

        /// Determinism: same input → same output labels + surviving keys.
        #[test]
        fn deterministic_same_input_same_output() {
            let dim = 8;
            let build = || {
                let emb: HashMap<String, Vec<f32>> = HashMap::from([
                    ("spk_0".into(), axis_vec(dim, 0, 0.02)),
                    ("spk_1".into(), axis_vec(dim, 0, 0.05)),
                    ("spk_2".into(), axis_vec(dim, 4, 0.02)),
                ]);
                let turns = vec![
                    turn("spk_0", 0.0, 20.0),
                    turn("spk_1", 20.0, 20.0),
                    turn("spk_2", 40.0, 20.0),
                ];
                (turns, emb)
            };
            let (t1, e1) = build();
            let (t2, e2) = build();
            let r1 = consolidate_clusters(t1, e1, ConsolidateOpts::default());
            let r2 = consolidate_clusters(t2, e2, ConsolidateOpts::default());
            assert_eq!(r1.0, r2.0, "turn labels must be deterministic");
            assert_eq!(
                r1.1.keys().cloned().collect::<HashSet<_>>(),
                r2.1.keys().cloned().collect::<HashSet<_>>(),
                "surviving embedding keys must be deterministic"
            );
        }

        /// A high floor makes even a same-axis pair fall below the threshold → no merge
        /// (proves the floor is honored, not just the shape of the data).
        #[test]
        fn floor_gates_merging() {
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.02)),
                ("spk_1".into(), axis_vec(dim, 0, 0.05)),
            ]);
            let turns = vec![turn("spk_0", 0.0, 10.0), turn("spk_1", 10.0, 10.0)];
            // Floor above the pair's (near-1.0) similarity → nothing merges.
            let opts = ConsolidateOpts {
                floor: 0.999_9,
                ..ConsolidateOpts::default()
            };
            let (out_turns, _out_emb) = consolidate_clusters(turns.clone(), emb, opts);
            assert_eq!(
                out_turns, turns,
                "an unreachable floor leaves clusters split"
            );
        }

        // -------------------------------------------------------------------
        // specs/0041 WS1 — bounded-cascade regression tests (the 1.8.0
        // "dominant speaker absorbs everyone" fix).
        // -------------------------------------------------------------------

        /// specs/0041 WS1 (a): a genuine drift-split — one voice split into two
        /// clusters whose centroids still sit at cos >= 0.75 — must merge at the
        /// raised 0.70 floor. Guards the repair the pass exists for: raising the
        /// floor must not disable the drift-split fix.
        #[test]
        fn drift_split_at_cos_075_still_merges() {
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)), // voice A, early
                ("spk_1".into(), vec_at_cos(dim, 0, 1, 0.75)), // voice A, late (cos 0.75)
            ]);
            let turns = vec![turn("spk_0", 0.0, 60.0), turn("spk_1", 60.0, 30.0)];
            let (out_turns, _out_emb) =
                consolidate_clusters(turns, emb, ConsolidateOpts::default());
            assert_eq!(
                distinct_keys(&out_turns),
                HashSet::from(["spk_0".to_string()]),
                "a cos-0.75 pair is the same voice and must consolidate to one key"
            );
        }

        /// specs/0041 WS1 (b): three *distinct* voices in the inter-speaker band
        /// (pairwise cos exactly 0.60, i.e. <= 0.65) with one dominant-duration
        /// cluster must NOT merge — the exact 1.8.0 regression shape: the old 0.50
        /// floor fused all three into the dominant talker.
        #[test]
        fn distinct_voices_with_dominant_cluster_do_not_merge() {
            let dim = 8;
            // Pairwise cos just below the floor — distinct-voices territory on any
            // embedding scale (constant-relative so floor retunes keep the scenario).
            // v_i = sqrt(c)·e0 + sqrt(1-c)·e_i → pairwise cos = c for all pairs.
            let c = CONSOLIDATE_FLOOR - 0.05;
            let mk = |axis: usize| {
                let mut v = vec![0.0f32; dim];
                v[0] = c.sqrt();
                v[axis] = (1.0 - c).sqrt();
                v
            };
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), mk(1)), // the dominant talker
                ("spk_1".into(), mk(2)),
                ("spk_2".into(), mk(3)),
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 1800.0), // dominates the meeting
                turn("spk_1", 1800.0, 90.0),
                turn("spk_2", 1890.0, 45.0),
            ];
            let (out_turns, out_emb) =
                consolidate_clusters(turns.clone(), emb.clone(), ConsolidateOpts::default());
            assert_eq!(
                out_turns, turns,
                "pairwise cos below CONSOLIDATE_FLOOR is distinct-voices territory — \
                 no merges, no absorption"
            );
            assert_eq!(out_emb, emb);
        }

        /// specs/0041 WS1 (c): with an AtMost(3) attendee seed and 4 clusters holding
        /// two independently-mergeable same-voice pairs, consolidation may reach 3 but
        /// never 2 — the seed is a *floor* for the cascade, not just an upper cap.
        #[test]
        fn at_most_seed_is_a_consolidation_floor() {
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), vec_at_cos(dim, 0, 1, 0.90)), // same voice as spk_0
                ("spk_2".into(), axis_vec(dim, 4, 0.0)),
                ("spk_3".into(), vec_at_cos(dim, 4, 5, 0.85)), // same voice as spk_2
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 40.0),
                turn("spk_1", 40.0, 30.0),
                turn("spk_2", 70.0, 20.0),
                turn("spk_3", 90.0, 10.0),
            ];
            let opts = ConsolidateOpts {
                min_clusters: Some(3), // the AtMost(3) seed
                ..ConsolidateOpts::default()
            };
            let (out_turns, _out_emb) = consolidate_clusters(turns, emb, opts);
            let keys = distinct_keys(&out_turns);
            assert_eq!(
                keys.len(),
                3,
                "the closest split (cos 0.90) merges 4 → 3, then the seed floor stops \
                 the cascade — never 2"
            );
            assert!(
                !keys.contains("spk_1"),
                "the merge that did happen is the highest-similarity pair (spk_1 → spk_0)"
            );
        }

        /// specs/0041 WS1 snowball guard: after the dominant cluster absorbs one
        /// merge, its duration-weighted centroid sits just inside the guard band
        /// (above [`CONSOLIDATE_FLOOR`], below floor + margin) relative to the next
        /// cluster — a repeat survivor must clear
        /// `floor + CONSOLIDATE_SNOWBALL_MARGIN`, so the second merge is refused.
        /// Without the guard this cascades: the 1.8.0 dominant-speaker snowball in
        /// miniature. (Geometry is constant-relative so floor retunes — 0.70 → 0.65
        /// in specs/0043 W1.1 — don't invalidate the scenario.)
        #[test]
        fn snowball_guard_blocks_repeat_survivor_below_margin() {
            let dim = 8;
            // A distinct voice in the "risky band": just above the floor, inside the
            // snowball-guard margin.
            let risky = CONSOLIDATE_FLOOR + 0.02;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)), // dominant voice
                ("spk_1".into(), vec_at_cos(dim, 0, 1, 0.90)), // same voice → merges in
                ("spk_2".into(), vec_at_cos(dim, 0, 2, risky)), // distinct, risky band
                ("spk_3".into(), axis_vec(dim, 5, 0.0)), // distinct
            ]);
            let turns = vec![
                // 300 s vs 5 s: the combined centroid stays ~= spk_0's, so its
                // similarity to spk_2 lands just under `risky` — between the floor
                // and floor + margin, exactly where only the guard says no.
                turn("spk_0", 0.0, 300.0),
                turn("spk_1", 300.0, 5.0),
                turn("spk_2", 305.0, 30.0),
                turn("spk_3", 335.0, 20.0),
            ];
            let (out_turns, _out_emb) =
                consolidate_clusters(turns, emb, ConsolidateOpts::default());
            let keys = distinct_keys(&out_turns);
            assert_eq!(keys.len(), 3, "exactly one merge: spk_1 into spk_0");
            assert!(
                keys.contains("spk_2"),
                "the 0.72-similar distinct voice survives the tainted survivor's pull"
            );
            assert!(keys.contains("spk_3"), "the orthogonal voice is untouched");
        }

        /// Code-review fix (specs/0041 WS1): the Auto merge budget is
        /// `ceil(initial / 2)`, not floor — with 3 same-voice clusters the old
        /// `initial / 2 = 1` budget merged 3 → 2 and stalled one merge short of the
        /// full drift-split repair. Near-identical centroids keep every pair (and the
        /// merged survivor) above `floor + CONSOLIDATE_SNOWBALL_MARGIN`, so only the
        /// budget can stop the cascade.
        #[test]
        fn auto_budget_rounds_up_three_same_voice_clusters_collapse_to_one() {
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), axis_vec(dim, 0, 0.01)),
                ("spk_2".into(), axis_vec(dim, 0, 0.02)),
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 40.0),
                turn("spk_1", 40.0, 30.0),
                turn("spk_2", 70.0, 20.0),
            ];
            let (out_turns, _out_emb) =
                consolidate_clusters(turns, emb, ConsolidateOpts::default());
            assert_eq!(
                distinct_keys(&out_turns),
                HashSet::from(["spk_0".to_string()]),
                "an odd same-voice drift-split must collapse fully (3 → 1), \
                 not stall at 2 on a floor-division budget"
            );
        }

        /// Even counts are unchanged by the rounding fix: `ceil(4 / 2) == 4 / 2 == 2`
        /// merges, so four same-voice clusters stop at 2 — the budget still caps the
        /// cascade at halving the count.
        #[test]
        fn auto_budget_even_count_still_caps_at_two_merges() {
            let dim = 8;
            let emb: HashMap<String, Vec<f32>> = HashMap::from([
                ("spk_0".into(), axis_vec(dim, 0, 0.0)),
                ("spk_1".into(), axis_vec(dim, 0, 0.01)),
                ("spk_2".into(), axis_vec(dim, 0, 0.02)),
                ("spk_3".into(), axis_vec(dim, 0, 0.03)),
            ]);
            let turns = vec![
                turn("spk_0", 0.0, 40.0),
                turn("spk_1", 40.0, 30.0),
                turn("spk_2", 70.0, 20.0),
                turn("spk_3", 90.0, 10.0),
            ];
            let (out_turns, _out_emb) =
                consolidate_clusters(turns, emb, ConsolidateOpts::default());
            assert_eq!(
                distinct_keys(&out_turns).len(),
                2,
                "initial = 4 budgets exactly 2 merges (4 → 2), same as before the fix"
            );
        }
    }
}
