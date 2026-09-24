//! Speaker diarization ("who said what"), on-device. (specs/0010, ADR-0005)
//!
//! This is the **P1-A core**: the engine + model management + alignment library.
//! DB schema, IPC commands, the per-channel `system.wav` capture, and the
//! frontend are later slices (P1-B onward) — nothing here touches them.
//!
//! Design (see `specs/0010` "Diarization module"):
//! - [`Diarizer`] is the engine-agnostic trait so the sherpa-onnx backend can be
//!   swapped for `speakrs` later (P3) without touching callers.
//! - [`sherpa::SherpaDiarizer`] is the P1 implementation over
//!   `sherpa_rs::diarize` (pyannote segmentation → speaker embedding →
//!   clustering), CPU/offline.
//! - [`models`] downloads + caches the two ONNX models on demand under
//!   `app_data_dir()/models/diarization/`, mirroring the Parakeet/Whisper pattern.
//! - [`align`] maps diarization speaker turns onto transcript segments by maximum
//!   temporal overlap, with the mic-channel `local`/`You` short-circuit.
//!
//! All audio in/out of this module is **16 kHz mono f32** — exactly what the VAD
//! stage already produces.

pub mod accel;
pub mod align;
pub mod auto_label;
pub mod candidates; // specs/0078: matcher inputs (moved from pipeline.rs), owner as a room candidate
pub mod commands;
pub mod corrections;
pub mod embedding;
pub mod folder_locate; // specs/0073 W1: NULL-folder_path fallback scan over every known root
pub mod folder_match;
pub mod identity;
pub mod launch;
pub mod live;
pub mod model_commands;
pub mod models;
pub mod owner_turns;
pub mod pipeline;
pub mod room; // specs/0078: room detection, input resolution, owner cluster
pub mod room_commands; // specs/0078: audio-setup override + "This is me"
pub mod room_types; // specs/0078: AudioSetup / AudioSetupOverride
pub mod seed; // specs/0050: audio-derived speaker-count estimator
pub mod segments;
pub mod settings;
pub mod sherpa;
pub(crate) mod sherpa_sys;
pub mod speaker_maintenance;
pub mod split;

/// specs/0047 real-meeting attribution eval — dev/diagnostic `#[ignore]` harness.
#[cfg(test)]
mod real_eval;

pub use align::{
    align_system_turns_to_segments, align_turns_to_segments, AlignMode, AlignableSegment, Channel,
    LOCAL_SPEAKER_KEY,
};
pub use sherpa::{SherpaDiarizer, SpeakerCount, UNKNOWN_SPEAKER_KEY};

/// A diarization "speaker turn": a contiguous span of the recording attributed to
/// one (clustered) speaker. Times are **recording-relative seconds**.
///
/// `speaker` is a stable per-meeting key (e.g. `spk_0`, `spk_1`) derived from the
/// engine's integer cluster id — *not* a display name. The mic/local user uses
/// [`LOCAL_SPEAKER_KEY`] (`"local"`). Display names ("You", "Speaker 1", renames)
/// live in the DB `speakers` table (a later slice), keyed off this value.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerTurn {
    /// Start time in recording-relative seconds.
    pub start: f32,
    /// End time in recording-relative seconds.
    pub end: f32,
    /// Stable per-meeting speaker key (`spk_0`, `spk_1`, …, or `"local"`).
    pub speaker: String,
}

impl SpeakerTurn {
    /// Duration of the turn in seconds (clamped to be non-negative).
    pub fn duration(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }
}

/// Result of [`Diarizer::diarize_with_embeddings`]: the speaker turns plus a
/// `speaker_key -> L2-normalized embedding` map for the remote clusters that had
/// enough speech to embed. (specs/0011)
pub type TurnsWithEmbeddings = (
    Vec<SpeakerTurn>,
    std::collections::HashMap<String, Vec<f32>>,
);

/// Map an engine cluster id (sherpa's `i32`) to a stable per-meeting speaker key.
///
/// Kept in one place so the engine impl and any tests agree on the format.
pub fn speaker_key_for_cluster(cluster_id: i32) -> String {
    format!("spk_{cluster_id}")
}

/// Engine-agnostic diarizer. Implementors run their own segmentation/embedding/
/// clustering pipeline and return recording-relative speaker turns.
///
/// `samples_16k_mono` must be 16 kHz mono f32; `sample_rate` is passed explicitly
/// so an implementation can validate/resample rather than silently assume.
pub trait Diarizer: Send + Sync {
    /// Diarize a (single-channel) audio buffer into speaker turns.
    fn diarize(
        &self,
        samples_16k_mono: &[f32],
        sample_rate: u32,
    ) -> anyhow::Result<Vec<SpeakerTurn>>;

    /// Diarize *and* return a representative L2-normalized embedding per remote
    /// cluster key (`spk_N` → vector). Used by the live session stable-id registry
    /// (specs/0011) to match this pass's clusters onto stable keys, and by the
    /// cross-meeting identity matcher (a later slice) to persist `speakers.embedding`.
    ///
    /// The default implementation calls [`Diarizer::diarize`] and returns an **empty**
    /// embedding map, so existing implementors (and the offline `pipeline.rs` path)
    /// are unaffected byte-for-byte. [`sherpa::SherpaDiarizer`] overrides it to
    /// surface per-cluster embeddings via the re-embed fallback (the centroids sherpa
    /// computes internally are not exposed — see [`embedding`]).
    fn diarize_with_embeddings(
        &self,
        samples_16k_mono: &[f32],
        sample_rate: u32,
    ) -> anyhow::Result<TurnsWithEmbeddings> {
        let turns = self.diarize(samples_16k_mono, sample_rate)?;
        Ok((turns, std::collections::HashMap::new()))
    }

    /// Same as [`Diarizer::diarize_with_embeddings`] but reports incremental progress
    /// via `on_progress(fraction)` where `fraction` is in `0.0..=1.0` as the
    /// segmentation/clustering advances. The offline pipeline uses this to drive the
    /// "Identifying speakers" percentage instead of leaving it pinned at 0%
    /// (specs/0019 WS2.5). The default ignores progress and delegates, so existing
    /// implementors are unaffected; [`sherpa::SherpaDiarizer`] overrides it to thread
    /// sherpa's per-chunk callback. `on_progress` is invoked synchronously on the
    /// calling thread — keep it cheap.
    fn diarize_with_embeddings_with_progress(
        &self,
        samples_16k_mono: &[f32],
        sample_rate: u32,
        on_progress: &mut dyn FnMut(f32),
    ) -> anyhow::Result<TurnsWithEmbeddings> {
        let _ = on_progress;
        self.diarize_with_embeddings(samples_16k_mono, sample_rate)
    }
}
