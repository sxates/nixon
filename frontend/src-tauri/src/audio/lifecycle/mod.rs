//! Audio lifecycle (specs/0072, ADR-0014): capture is separate from retention, and Rust owns
//! when a meeting's audio is kept, compressed or deleted.
//!
//! - `policy.rs`: the pure [`policy::disposition`] and the retention setting.
//! - `state.rs`: `meetings.audio_state` / `speakers_identified_at`, the facts loader and the
//!   shared backlog predicate.
//! - `sweep.rs`: the executor (delete / compress under the folder lease) and the preview.
//! - `compress.rs`: channel WAV → Opus, verified, both channels or neither.
//! - `hooks.rs`: the one-line calls processing steps make, the startup reconcile, `spawn`.
//! - `commands.rs`: the Tauri commands.

pub mod commands;
pub mod compress;
mod hooks;
pub mod policy;
pub mod state;
pub mod sweep;

pub use hooks::{
    diarization_gate, finish_processing, on_diarization_outcome, on_transcript_replaced,
    reevaluate_after_backlog, reset_for_resume, spawn, spawn_apply, sweep_now, AudioStateChanged,
    FinishOutcome, EVENT_AUDIO_STATE_CHANGED,
};

/// A meeting is "not really transcribed" below this many transcript segments (specs/0029
/// WS7.2): its audio may be the only record of it. Part of the backlog predicate.
pub const MIN_TRANSCRIPT_SEGMENTS: i64 = 3;

/// Whether the sweep compresses kept channel WAVs to Opus. Off until every channel reader
/// (speaker identification, owner turns, retranscription channel tags) resolves `.opus`
/// (0072 W2); the W2 change that teaches them flips this. The policy, the compressor and
/// the one-meeting-per-tick backfill are already in place behind it.
pub const COMPRESSION_ENABLED: bool = false;
