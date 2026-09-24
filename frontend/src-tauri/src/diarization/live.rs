//! Live (real-time) speaker diarization core. (specs/0011 Task 10, ADR-0006)
//!
//! **Approach (ADR-0006, approved):** *periodic re-diarization on the growing
//! buffer.* During recording we accumulate the (clean, pre-mix) **system** channel
//! at 16 kHz mono and, every [`PASS_INTERVAL`], run the **existing offline**
//! [`SherpaDiarizer`] over the whole-buffer-so-far on a CPU `spawn_blocking`
//! thread. This is NOT streaming and NOT a sliding window — it reuses the shipped,
//! tested clustering path, run earlier and repeatedly, and the offline pass at stop
//! stays authoritative.
//!
//! **Why this is real-time safe (the hard constraint — never starve live STT):**
//! - The diarizer is **CPU-only** (`provider: "cpu"`), never touching the
//!   Metal/CoreML path STT uses.
//! - Passes run on `spawn_blocking` — off the async executor and off the UI thread.
//! - **Single in-flight, debounced:** a pass starts only if none is running and
//!   ≥ [`MIN_NEW_AUDIO_SECONDS`] of new system audio has accrued; otherwise the
//!   tick is skipped (never queued).
//! - When live diarization is disabled (the default), no `LiveDiarizer` is created
//!   and the pipeline tap is inert (see `audio/pipeline.rs`) — zero cost vs v0.4.0.
//!
//! **Label stability — the `stable-once-shown` contract (the core problem):**
//! sherpa's cluster ids (`spk_0`, `spk_1`, …) are per-run and arbitrary; `spk_0`
//! this pass may be `spk_1` next pass. We map each pass's raw clusters onto
//! *session-stable* keys via an embedding-anchored registry ([`SpeakerRegistry`]):
//! each session speaker keeps a running-mean centroid and a stable key allocated
//! in first-seen order; a fresh cluster matches an existing speaker by cosine ≥
//! [`SESSION_MATCH_THRESHOLD`] (reuse its key) or allocates a new key. Crucially,
//! **a stable key, once shown for a transcript segment, is never changed by a later
//! live pass** ([`LiveDiarizer`] tracks already-labeled segment ids and only emits
//! `None → key` transitions). The single authoritative correction happens once, at
//! stop, via the offline pass — never as on-screen churn.
//!
//! **Seams for the next stage (rust-core IPC/events/persistence):**
//! - [`LiveDiarizer::feed_48k`] — the pipeline tap pushes 48 kHz system windows here
//!   (downsampled internally to 16 kHz, matching `channel_writer`'s parity).
//! - [`LiveDiarizer::set_segments`] — the caller supplies current transcript segment
//!   windows `(id, start, end)`; the live pass aligns turns to these (reusing
//!   `align.rs`'s overlap + nearest-turn/`unknown` semantics, specs/0043 W1.3 —
//!   the live pass never labels a segment `local`/"You") and reports which became
//!   newly labeled.
//! - The `on_labels` callback (injected at construction) receives
//!   `Vec<NewSegmentLabel { segment_id, speaker_key, display_name }>` after each pass
//!   — the next stage emits these as `live-diarization-update`. **This module emits
//!   no Tauri events and touches no DB** by design, so it stays testable.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::diarization::align::align_system_turns_to_segments;
use crate::diarization::embedding::{cosine_similarity, weighted_mean};
use crate::diarization::pipeline::display_name_for_key;
use crate::diarization::{Diarizer, SpeakerTurn};

/// Capture sample rate of the recording pipeline tap (mic + system normalized here).
const CAPTURE_SAMPLE_RATE: u32 = 48_000;
/// Sample rate the diarizer + embedding model expect.
const DIARIZATION_SAMPLE_RATE: u32 = 16_000;

/// How often the background task wakes to consider running a pass. Easy to tune —
/// the accuracy/latency trade-off knob from ADR-0006 ("T = ~5–10 s"). Lower = labels
/// appear sooner but more CPU; higher = less contention.
pub const PASS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(8);

/// Minimum new system audio (seconds) that must have accrued since the last pass
/// before a new pass runs. Below this, the tick is skipped — re-clustering the same
/// buffer wastes CPU and can't reveal new speakers. (Debounce, ADR-0006.)
const MIN_NEW_AUDIO_SECONDS: f32 = 3.0;

/// specs/0076: whether the running recording's pass loop may start passes. The setting is
/// read once when a recording starts (`build_live_diarizer`), so without this, switching
/// it off in Settings left a meeting already recording at ~700% CPU in bursts until it
/// stopped (owner report, v0.10.0). One recording runs at a time, so one flag is enough.
static LIVE_PASSES_ENABLED: AtomicBool = AtomicBool::new(true);

/// Called by the live-labels setting: off stops passes in the recording under way; on
/// resumes them if that recording started with live labels (otherwise it has no diarizer).
pub fn set_live_passes_enabled(enabled: bool) {
    LIVE_PASSES_ENABLED.store(enabled, Ordering::SeqCst);
}

/// A tick's gate, apart from the single-flight check: the setting is on and enough new
/// audio has come in since the last pass.
fn should_run_pass(passes_enabled: bool, new_audio_seconds: f32) -> bool {
    passes_enabled && new_audio_seconds >= MIN_NEW_AUDIO_SECONDS
}

/// specs/0028: cap the live diarization buffer to a sliding window of the most recent audio,
/// instead of re-clustering the *whole growing buffer* every pass. The old approach was
/// O(n²) in CPU over a meeting (each pass re-clusters everything) and grew RAM without bound
/// (~230 MB/hr at 16 kHz f32). A bounded window makes each pass O(window) and caps RAM;
/// cross-window speaker identity is preserved by the persistent embedding registry, and the
/// offline pass at stop remains authoritative. 180 s gives clustering ample context (~11.5 MB).
const MAX_LIVE_BUFFER_SECONDS: usize = 180;
const MAX_LIVE_BUFFER_SAMPLES: usize = MAX_LIVE_BUFFER_SECONDS * DIARIZATION_SAMPLE_RATE as usize;

/// Cosine threshold for matching a fresh raw cluster centroid to an existing
/// *session* speaker. CAM++ cosine sits ~0.5 for "same speaker" (sherpa's own
/// `DEFAULT_SIMILARITY_THRESHOLD`); tune on the accuracy benchmark. Higher = more
/// conservative (more likely to split one person into two session keys); lower =
/// more likely to merge two people. We err slightly conservative since a stale split
/// is corrected by the offline pass at stop, while a wrong merge shows a wrong name.
const SESSION_MATCH_THRESHOLD: f32 = 0.5;

/// A transcript segment window to align against (recording-relative seconds).
/// The caller (next stage) maps DB rows / live `TranscriptUpdate`s to this.
#[derive(Debug, Clone)]
pub struct LiveSegment {
    pub id: String,
    pub start: f32,
    pub end: f32,
}

/// A newly-resolved label to surface to the next stage (it emits the event / patches
/// the DB). Only ever a `None → key` transition (stable-once-shown).
#[derive(Debug, Clone, PartialEq)]
pub struct NewSegmentLabel {
    /// Transcript segment id this label is for.
    pub segment_id: String,
    /// Session-stable speaker key (`spk_0`, `spk_1`, … or `unknown`; never `local` —
    /// the live pass has no channel tags, specs/0043 W1.3).
    pub speaker_key: String,
    /// Resolved display name ("Speaker 1" / "Unknown speaker"), consistent with the
    /// offline path.
    pub display_name: String,
}

/// Callback invoked (on the blocking pass thread) with the segments newly labeled by
/// a pass. Boxed so it can be injected by the IPC stage and stubbed in tests.
pub type LabelCallback = Box<dyn Fn(Vec<NewSegmentLabel>) + Send + Sync>;

// ---------------------------------------------------------------------------
// Embedding-anchored session stable-id registry (the label-stability core).
// ---------------------------------------------------------------------------

/// One session speaker: a stable key + a running-mean L2-normalized centroid and the
/// number of cluster observations folded into it (for weighted updates).
struct SessionSpeaker {
    stable_key: String,
    centroid: Vec<f32>,
    observations: u32,
}

/// Maps each diarization pass's arbitrary raw cluster ids onto session-stable keys by
/// matching cluster centroids to known session speakers. Stable keys are allocated
/// `spk_0`, `spk_1`, … in first-seen order. Pure (no model/audio) so it unit-tests by
/// feeding synthetic centroids across "passes".
#[derive(Default)]
pub struct SpeakerRegistry {
    speakers: Vec<SessionSpeaker>,
    next_index: u32,
}

impl SpeakerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve one pass's `raw_key -> centroid` map to `raw_key -> stable_key`,
    /// folding matched centroids into the running means and allocating new stable
    /// keys for unmatched clusters.
    ///
    /// Greedy nearest-match: for each raw cluster (in deterministic key order) pick
    /// the best-scoring *unclaimed-this-pass* session speaker ≥
    /// [`SESSION_MATCH_THRESHOLD`]. "Unclaimed-this-pass" prevents two raw clusters
    /// from both mapping to the same session speaker in a single pass (which would
    /// hide a real second speaker).
    pub fn resolve_pass(&mut self, raw: &HashMap<String, Vec<f32>>) -> HashMap<String, String> {
        let mut out = HashMap::new();
        // Deterministic order so allocation of new keys is reproducible.
        let mut raw_keys: Vec<&String> = raw.keys().collect();
        raw_keys.sort();

        let mut claimed: HashSet<usize> = HashSet::new();

        for rk in raw_keys {
            let centroid = &raw[rk];
            // Find best unclaimed existing session speaker.
            let mut best: Option<(usize, f32)> = None;
            for (i, sp) in self.speakers.iter().enumerate() {
                if claimed.contains(&i) {
                    continue;
                }
                let score = cosine_similarity(&sp.centroid, centroid);
                if score >= SESSION_MATCH_THRESHOLD && best.map(|(_, b)| score > b).unwrap_or(true)
                {
                    best = Some((i, score));
                }
            }

            match best {
                Some((i, _)) => {
                    claimed.insert(i);
                    let sp = &mut self.speakers[i];
                    sp.centroid = weighted_mean(&sp.centroid, sp.observations, centroid, 1);
                    sp.observations = sp.observations.saturating_add(1);
                    out.insert(rk.clone(), sp.stable_key.clone());
                }
                None => {
                    let stable_key = format!("spk_{}", self.next_index);
                    self.next_index += 1;
                    self.speakers.push(SessionSpeaker {
                        stable_key: stable_key.clone(),
                        centroid: centroid.clone(),
                        observations: 1,
                    });
                    // A freshly-allocated speaker is claimed for this pass too.
                    claimed.insert(self.speakers.len() - 1);
                    out.insert(rk.clone(), stable_key);
                }
            }
        }
        out
    }

    /// Number of distinct session speakers seen so far.
    pub fn len(&self) -> usize {
        self.speakers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.speakers.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Growing 16 kHz buffer fed from the 48 kHz pipeline tap.
// ---------------------------------------------------------------------------

/// Accumulating system-channel buffer with an internal 48 kHz → 16 kHz persistent
/// resampler. Uses the **same** `ChannelWavWriter` downsample contract (fixed 512
/// input blocks through one persistent sinc resampler) so the live buffer matches
/// `system.wav` sample-for-sample — guaranteeing live and offline see the same audio
/// and avoiding the per-window RMS amplification a fresh resampler would cause.
struct GrowingBuffer {
    resampler: crate::audio::channel_writer::WindowDownsampler,
    /// The most recent 16 kHz mono samples, bounded to [`MAX_LIVE_BUFFER_SAMPLES`]
    /// (specs/0028: sliding window, not the whole call — bounds RAM + per-pass O(n²) cost).
    samples_16k: Vec<f32>,
    /// Cumulative count of 16 kHz samples ever appended (monotonic; independent of front
    /// trimming). Used to measure "new audio since last pass" and the window's start time.
    total_pushed_16k: u64,
    /// `total_pushed_16k` at the last pass start, to compute "new audio since".
    total_at_last_pass: u64,
}

impl GrowingBuffer {
    fn new() -> Result<Self> {
        Ok(Self {
            resampler: crate::audio::channel_writer::WindowDownsampler::new(
                CAPTURE_SAMPLE_RATE,
                DIARIZATION_SAMPLE_RATE,
            )?,
            samples_16k: Vec::new(),
            total_pushed_16k: 0,
            total_at_last_pass: 0,
        })
    }

    /// Push one 48 kHz mono window; downsamples, appends 16 kHz samples, and trims the front
    /// so the retained buffer never exceeds the sliding-window cap.
    fn push_48k(&mut self, window_48k: &[f32]) -> Result<()> {
        let before = self.samples_16k.len();
        self.resampler.push(window_48k, &mut self.samples_16k)?;
        let added = self.samples_16k.len() - before;
        self.total_pushed_16k += added as u64;
        // Drop the oldest samples beyond the cap (bounded sliding window).
        if self.samples_16k.len() > MAX_LIVE_BUFFER_SAMPLES {
            let excess = self.samples_16k.len() - MAX_LIVE_BUFFER_SAMPLES;
            self.samples_16k.drain(0..excess);
        }
        Ok(())
    }

    /// New 16 kHz samples accrued since the last pass started (uses the monotonic total so
    /// front-trimming can't skew it).
    fn new_seconds(&self) -> f32 {
        (self
            .total_pushed_16k
            .saturating_sub(self.total_at_last_pass)) as f32
            / DIARIZATION_SAMPLE_RATE as f32
    }

    /// Recording-relative time (seconds) of the FIRST retained sample — the offset a pass must
    /// add to its window-relative turn timestamps to place them on the recording timeline.
    fn window_start_seconds(&self) -> f32 {
        (self.total_pushed_16k - self.samples_16k.len() as u64) as f32
            / DIARIZATION_SAMPLE_RATE as f32
    }

    /// Snapshot the current (bounded) 16 kHz window for a pass, mark the pass boundary, and
    /// return the window's recording-relative start time for timestamp alignment.
    fn snapshot_for_pass(&mut self) -> (Vec<f32>, f32) {
        self.total_at_last_pass = self.total_pushed_16k;
        (self.samples_16k.clone(), self.window_start_seconds())
    }
}

// ---------------------------------------------------------------------------
// LiveDiarizer — owns the buffer, the registry, the segment ledger, the task.
// ---------------------------------------------------------------------------

/// State shared between the pipeline-tap feed (`feed_48k`), the caller's
/// `set_segments`, and the background pass task. Guarded by a single `Mutex` —
/// contention is negligible (the feed pushes small windows; passes snapshot once).
struct Shared {
    buffer: GrowingBuffer,
    registry: SpeakerRegistry,
    /// Current transcript segments to align against (set by the caller each tick/pass).
    segments: Vec<LiveSegment>,
    /// Segment ids already labeled (stable-once-shown ledger): never re-emitted, never
    /// changed by a later live pass.
    labeled: HashSet<String>,
    /// Whether feeding has been permanently disabled by a fatal resample error
    /// (best-effort contract: a tap failure must never break recording/STT).
    feed_disabled: bool,
}

/// The live diarization handle. Created only when live diarization is enabled.
/// Drop / [`LiveDiarizer::stop`] ends the background task.
pub struct LiveDiarizer {
    shared: Arc<Mutex<Shared>>,
    /// Set false to stop the background pass task.
    running: Arc<AtomicBool>,
}

impl LiveDiarizer {
    /// Create a live diarizer for a meeting and start its background pass task.
    ///
    /// - `diarizer` is the engine (an `Arc<SherpaDiarizer>`, CPU/offline) shared so a
    ///   pass can run it on a blocking thread.
    /// - `on_labels` is invoked after each pass with the newly-resolved labels (the
    ///   next stage emits them). It runs on the blocking pass thread.
    ///
    /// Best-effort: if the internal resampler can't be created, returns an error and
    /// the caller simply doesn't enable live diarization (recording is unaffected).
    pub fn start(diarizer: Arc<dyn Diarizer>, on_labels: LabelCallback) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            buffer: GrowingBuffer::new()?,
            registry: SpeakerRegistry::new(),
            segments: Vec::new(),
            labeled: HashSet::new(),
            feed_disabled: false,
        }));
        let running = Arc::new(AtomicBool::new(true));
        // Starting means the setting is on (the caller checked), whatever a previous
        // recording's mid-meeting switch left behind.
        set_live_passes_enabled(true);
        // Single-in-flight debounce flag — owned solely by the background loop.
        let pass_in_flight = Arc::new(AtomicBool::new(false));

        let task_shared = shared.clone();
        let task_running = running.clone();
        let on_labels = Arc::new(on_labels);

        tauri::async_runtime::spawn(async move {
            run_pass_loop(
                diarizer,
                task_shared,
                task_running,
                pass_in_flight,
                on_labels,
            )
            .await;
        });

        Ok(Self { shared, running })
    }

    /// Push one 48 kHz mono **system** window (the pipeline tap fan-out). Mic windows
    /// are never fed (mic = `You`). Best-effort: on a resample error the feed is
    /// permanently disabled (logged once) so it can never break recording/STT.
    pub fn feed_48k(&self, window_48k: &[f32]) {
        let Ok(mut s) = self.shared.lock() else {
            return; // poisoned lock: drop the window rather than panic the pipeline.
        };
        if s.feed_disabled {
            return;
        }
        if let Err(e) = s.buffer.push_48k(window_48k) {
            log::warn!(
                "live diarization: feed resample failed ({e}); disabling live feed (recording unaffected)"
            );
            s.feed_disabled = true;
        }
    }

    /// Update the transcript segment windows the next pass aligns against. The caller
    /// (IPC stage) supplies these as live transcription produces segments; already-
    /// labeled segments are kept stable regardless.
    pub fn set_segments(&self, segments: Vec<LiveSegment>) {
        if let Ok(mut s) = self.shared.lock() {
            s.segments = segments;
        }
    }

    /// Stop the background pass task. Idempotent; also runs on drop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Drop for LiveDiarizer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The background loop: wake every [`PASS_INTERVAL`], run a debounced single-flight
/// pass. Factored out (not a closure) for readability.
async fn run_pass_loop(
    diarizer: Arc<dyn Diarizer>,
    shared: Arc<Mutex<Shared>>,
    running: Arc<AtomicBool>,
    pass_in_flight: Arc<AtomicBool>,
    on_labels: Arc<LabelCallback>,
) {
    while running.load(Ordering::SeqCst) {
        tokio::time::sleep(PASS_INTERVAL).await;
        if !running.load(Ordering::SeqCst) {
            break;
        }
        // Debounce: skip if a pass is already running.
        if pass_in_flight.load(Ordering::SeqCst) {
            continue;
        }

        // Snapshot under lock: buffer + segments, and gate on enough new audio.
        let (samples, window_start_seconds, segments) = {
            let Ok(mut s) = shared.lock() else { continue };
            if !should_run_pass(
                LIVE_PASSES_ENABLED.load(Ordering::SeqCst),
                s.buffer.new_seconds(),
            ) {
                continue; // switched off, or not enough new audio — skip this tick (don't queue).
            }
            let (samples, window_start_seconds) = s.buffer.snapshot_for_pass();
            let segments = s.segments.clone();
            (samples, window_start_seconds, segments)
        };

        pass_in_flight.store(true, Ordering::SeqCst);
        let diarizer = diarizer.clone();
        let shared = shared.clone();
        let pass_in_flight_done = pass_in_flight.clone();
        let on_labels = on_labels.clone();

        // CPU-only diarization on a blocking thread — never on the async executor.
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let new_labels = run_single_pass(
                &*diarizer,
                &shared,
                &samples,
                window_start_seconds,
                &segments,
            );
            if !new_labels.is_empty() {
                (on_labels)(new_labels);
            }
            pass_in_flight_done.store(false, Ordering::SeqCst);
        })
        .await;
        // Defensive: clear the flag even if the blocking task join failed.
        pass_in_flight.store(false, Ordering::SeqCst);
    }
    log::info!("live diarization pass loop stopped");
}

/// Run one diarization pass over `samples`, map raw clusters to session-stable keys,
/// align to `segments`, and return the segments newly labeled this pass (respecting
/// the stable-once-shown ledger). Pure side effects: updates the registry + ledger
/// under the shared lock. Errors are logged and yield no new labels (best-effort).
fn run_single_pass(
    diarizer: &dyn Diarizer,
    shared: &Arc<Mutex<Shared>>,
    samples: &[f32],
    // specs/0028: recording-relative start time of the (bounded) window `samples` covers. Raw
    // turn timestamps come back window-relative and must be shifted by this to align to the
    // recording-relative transcript segments. 0.0 when the window starts at recording start.
    window_start_seconds: f32,
    segments: &[LiveSegment],
) -> Vec<NewSegmentLabel> {
    // 1. Diarize + embeddings (the silence pre-check inside returns empty cleanly).
    let (raw_turns, raw_embeddings) =
        match diarizer.diarize_with_embeddings(samples, DIARIZATION_SAMPLE_RATE) {
            Ok(pair) => pair,
            Err(e) => {
                log::warn!("live diarization pass failed: {e:#}");
                return Vec::new();
            }
        };

    // 2. Map raw cluster keys -> session-stable keys via the registry, then rewrite
    //    the turns to carry stable keys. A turn whose cluster produced no embedding
    //    (too little speech) keeps its raw key — it'll align but won't anchor the
    //    registry; acceptable, and the offline pass corrects any drift.
    let mut guard = match shared.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let mapping = guard.registry.resolve_pass(&raw_embeddings);

    // Shift window-relative turn timestamps onto the recording timeline (specs/0028) so they
    // align with the recording-relative transcript segments below.
    let stable_turns: Vec<SpeakerTurn> = raw_turns
        .iter()
        .map(|t| SpeakerTurn {
            start: t.start + window_start_seconds,
            end: t.end + window_start_seconds,
            speaker: mapping
                .get(&t.speaker)
                .cloned()
                .unwrap_or_else(|| t.speaker.clone()),
        })
        .collect();

    // 3. Align to the current segments (live semantics, specs/0043 W1.3: max
    //    overlap → nearest turn within the window → `unknown`; never `local`).
    // specs/0044 W1.1: score overlaps on the pad-trimmed speech core — live
    // segment timestamps carry the same VAD pre/post pads as stored rows.
    let windows: Vec<(f32, f32)> = segments
        .iter()
        .map(|s| crate::diarization::align::pad_trimmed(s.start, s.end))
        .collect();
    let keys = align_system_turns_to_segments(&stable_turns, &windows);

    // 4. Emit only newly-labeled segments (stable-once-shown). A segment already in
    //    the ledger is never re-emitted or changed by this live pass.
    let mut new_labels = Vec::new();
    for (seg, key) in segments.iter().zip(keys) {
        if guard.labeled.contains(&seg.id) {
            continue;
        }
        // specs/0028: with a bounded window, segments that end before the window starts have
        // no window audio to align against and would spuriously resolve to `unknown` (or a
        // wrong nearest turn). Leave them unlabeled for the authoritative offline pass at
        // stop rather than locking a wrong label (stable-once-shown). Never triggers while
        // the window spans recording start.
        if seg.end < window_start_seconds {
            continue;
        }
        let display_name = display_name_for_key(&key);
        guard.labeled.insert(seg.id.clone());
        new_labels.push(NewSegmentLabel {
            segment_id: seg.id.clone(),
            speaker_key: key,
            display_name,
        });
    }

    new_labels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(dir: &[f32]) -> Vec<f32> {
        crate::diarization::embedding::l2_normalize(dir)
    }

    // --- SpeakerRegistry: stable-id matching + stable keys never flip ---

    #[test]
    fn registry_allocates_keys_in_first_seen_order() {
        let mut reg = SpeakerRegistry::new();
        let mut pass = HashMap::new();
        pass.insert("spk_0".to_string(), unit(&[1.0, 0.0, 0.0]));
        pass.insert("spk_1".to_string(), unit(&[0.0, 1.0, 0.0]));
        let m = reg.resolve_pass(&pass);
        // Deterministic key-sorted allocation: raw spk_0 -> stable spk_0, etc.
        assert_eq!(m["spk_0"], "spk_0");
        assert_eq!(m["spk_1"], "spk_1");
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn registry_reuses_key_for_same_voice_across_passes() {
        let mut reg = SpeakerRegistry::new();
        let voice_a = unit(&[1.0, 0.0, 0.0]);
        let voice_b = unit(&[0.0, 1.0, 0.0]);

        // Pass 1: A is raw spk_0, B is raw spk_1.
        let mut p1 = HashMap::new();
        p1.insert("spk_0".to_string(), voice_a.clone());
        p1.insert("spk_1".to_string(), voice_b.clone());
        let m1 = reg.resolve_pass(&p1);
        let a_key = m1["spk_0"].clone();
        let b_key = m1["spk_1"].clone();

        // Pass 2: sherpa RE-NUMBERS — now A is raw spk_1, B is raw spk_0 (the churn
        // we must absorb). Same voices ⇒ same stable keys, just swapped raw ids.
        let mut p2 = HashMap::new();
        p2.insert("spk_1".to_string(), voice_a.clone());
        p2.insert("spk_0".to_string(), voice_b.clone());
        let m2 = reg.resolve_pass(&p2);
        assert_eq!(
            m2["spk_1"], a_key,
            "voice A keeps its stable key despite re-numbering"
        );
        assert_eq!(
            m2["spk_0"], b_key,
            "voice B keeps its stable key despite re-numbering"
        );
        assert_eq!(reg.len(), 2, "no spurious new speakers");
    }

    #[test]
    fn registry_allocates_new_key_for_new_voice() {
        let mut reg = SpeakerRegistry::new();
        let voice_a = unit(&[1.0, 0.0, 0.0]);
        let mut p1 = HashMap::new();
        p1.insert("spk_0".to_string(), voice_a.clone());
        let m1 = reg.resolve_pass(&p1);
        let a_key = m1["spk_0"].clone();

        // A new, dissimilar voice joins → must get a fresh stable key, A keeps hers.
        let voice_c = unit(&[0.0, 0.0, 1.0]);
        let mut p2 = HashMap::new();
        p2.insert("spk_0".to_string(), voice_a.clone()); // A still here, re-numbered raw
        p2.insert("spk_1".to_string(), voice_c.clone());
        let m2 = reg.resolve_pass(&p2);
        assert_eq!(m2["spk_0"], a_key);
        assert_ne!(m2["spk_1"], a_key);
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn registry_two_clusters_one_pass_dont_collapse_to_one_speaker() {
        // Two distinct-but-not-orthogonal voices in one pass must map to two keys,
        // not both claim the single closest existing speaker.
        let mut reg = SpeakerRegistry::new();
        let a = unit(&[1.0, 0.0, 0.0]);
        reg.resolve_pass(&HashMap::from([("spk_0".to_string(), a.clone())]));

        let near_a = unit(&[0.9, 0.1, 0.0]); // close to A
        let far = unit(&[0.0, 1.0, 0.0]); // new voice
        let m = reg.resolve_pass(&HashMap::from([
            ("spk_0".to_string(), near_a),
            ("spk_1".to_string(), far),
        ]));
        // One reuses A's key, the other is new — they don't both become A.
        assert_ne!(m["spk_0"], m["spk_1"]);
    }

    // specs/0076: switching the setting off mid-recording must stop passes at once.
    #[test]
    fn no_pass_runs_while_switched_off_even_with_plenty_of_new_audio() {
        assert!(!should_run_pass(false, 60.0));
        assert!(should_run_pass(true, 60.0));
        assert!(!should_run_pass(true, MIN_NEW_AUDIO_SECONDS - 0.1));
    }

    // --- run_single_pass via a stub Diarizer: end-to-end stable-once-shown ---

    struct StubDiarizer {
        turns: Vec<SpeakerTurn>,
        embeddings: HashMap<String, Vec<f32>>,
    }
    impl Diarizer for StubDiarizer {
        fn diarize(&self, _s: &[f32], _sr: u32) -> Result<Vec<SpeakerTurn>> {
            Ok(self.turns.clone())
        }
        fn diarize_with_embeddings(
            &self,
            _s: &[f32],
            _sr: u32,
        ) -> Result<crate::diarization::TurnsWithEmbeddings> {
            Ok((self.turns.clone(), self.embeddings.clone()))
        }
    }

    fn shared_with_segments(segs: Vec<LiveSegment>) -> Arc<Mutex<Shared>> {
        Arc::new(Mutex::new(Shared {
            buffer: GrowingBuffer::new().unwrap(),
            registry: SpeakerRegistry::new(),
            segments: segs,
            labeled: HashSet::new(),
            feed_disabled: false,
        }))
    }

    #[test]
    fn pass_labels_new_segments_and_never_relabels() {
        let segs = vec![
            LiveSegment {
                id: "s0".into(),
                start: 0.0,
                end: 3.0,
            },
            LiveSegment {
                id: "s1".into(),
                start: 3.0,
                end: 6.0,
            },
        ];
        let shared = shared_with_segments(segs.clone());

        // Pass 1: spk_0 covers s0; s1 overlaps no turn but is adjacent to spk_0's
        // turn (gap 0 ≤ nearest-turn window) → spk_0 too. (specs/0043 W1.3: the
        // live pass never resolves to "You"; pre-W1.3 s1 fell back to local.)
        let stub1 = StubDiarizer {
            turns: vec![SpeakerTurn {
                start: 0.0,
                end: 3.0,
                speaker: "spk_0".into(),
            }],
            embeddings: HashMap::from([("spk_0".to_string(), unit(&[1.0, 0.0, 0.0]))]),
        };
        let labels1 = run_single_pass(&stub1, &shared, &[], 0.0, &segs);
        let l1: HashMap<_, _> = labels1
            .iter()
            .map(|l| (l.segment_id.clone(), l.display_name.clone()))
            .collect();
        assert_eq!(l1["s0"], "Speaker 1");
        assert_eq!(l1["s1"], "Speaker 1"); // no overlap → nearest turn within 1.0s
        assert_eq!(labels1.len(), 2);

        // Pass 2: sherpa now (wrongly) thinks s0 was spk_1, re-numbered. Because s0 is
        // already in the ledger, it is NOT re-emitted or changed — stable-once-shown.
        let stub2 = StubDiarizer {
            turns: vec![SpeakerTurn {
                start: 0.0,
                end: 6.0,
                speaker: "spk_0".into(),
            }],
            embeddings: HashMap::from([("spk_0".to_string(), unit(&[0.0, 1.0, 0.0]))]),
        };
        let labels2 = run_single_pass(&stub2, &shared, &[], 0.0, &segs);
        assert!(
            labels2.is_empty(),
            "already-labeled segments are never re-emitted"
        );
    }

    #[test]
    fn pass_labels_only_newly_added_segments_on_later_pass() {
        let segs1 = vec![LiveSegment {
            id: "s0".into(),
            start: 0.0,
            end: 3.0,
        }];
        let shared = shared_with_segments(segs1.clone());
        let stub = StubDiarizer {
            turns: vec![SpeakerTurn {
                start: 0.0,
                end: 3.0,
                speaker: "spk_0".into(),
            }],
            embeddings: HashMap::from([("spk_0".to_string(), unit(&[1.0, 0.0, 0.0]))]),
        };
        let l1 = run_single_pass(&stub, &shared, &[], 0.0, &segs1);
        assert_eq!(l1.len(), 1);

        // A new segment s1 appears; only it should be labeled this pass.
        let segs2 = vec![
            LiveSegment {
                id: "s0".into(),
                start: 0.0,
                end: 3.0,
            },
            LiveSegment {
                id: "s1".into(),
                start: 3.0,
                end: 6.0,
            },
        ];
        let stub2 = StubDiarizer {
            turns: vec![SpeakerTurn {
                start: 0.0,
                end: 6.0,
                speaker: "spk_0".into(),
            }],
            embeddings: HashMap::from([("spk_0".to_string(), unit(&[1.0, 0.0, 0.0]))]),
        };
        let l2 = run_single_pass(&stub2, &shared, &[], 0.0, &segs2);
        assert_eq!(l2.len(), 1);
        assert_eq!(l2[0].segment_id, "s1");
    }
}
