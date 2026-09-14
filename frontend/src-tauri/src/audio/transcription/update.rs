//! The `transcript-update` event payload (Rust -> frontend).
//!
//! Split out of `worker.rs` under the specs/0042 file-size ratchet: the wire type
//! is a contract the frontend depends on, not part of the worker's loop.

use serde::{Deserialize, Serialize};

use crate::audio::common::ChannelRun;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TranscriptUpdate {
    pub text: String,
    pub timestamp: String, // Wall-clock time for reference (e.g., "14:30:05")
    pub source: String,
    pub sequence_id: u64,
    pub chunk_start_time: f64, // Legacy field, kept for compatibility
    pub is_partial: bool,
    pub confidence: f32,
    // NEW: Recording-relative timestamps for playback sync
    pub audio_start_time: f64, // Seconds from recording start (e.g., 125.3)
    pub audio_end_time: f64,   // Seconds from recording start (e.g., 128.6)
    pub duration: f64,         // Segment duration in seconds (e.g., 3.3)
    // specs/0011 P3-B: resolved live speaker display name ("You" / "Speaker 2"), or
    // `None` until the live diarization pass labels this segment (and always `None`
    // when live diarization is disabled — the default). Lets the first-pass live label
    // ride the normal transcript event; later retroactive labels arrive via the
    // dedicated `live-diarization-update` event. The persisted authoritative key is
    // still written by the offline pass at stop.
    #[serde(default)]
    pub speaker: Option<String>,
    // specs/0029 WS3.4: capture-channel tag for this segment's audio, from per-window
    // RMS dominance of the pre-mix tracks: "microphone" (the local user — "You"),
    // "system" (remote participants), or "mixed" (overlapped speech). `None` when the
    // pipeline had no channel evidence. The frontend carries this through to the save
    // payload (persisted as `transcripts.channel`) and may render "You" live for
    // microphone-tagged segments.
    #[serde(default)]
    pub channel: Option<String>,
    // specs/0055: the same evidence at 600 ms resolution (see `ChannelRun`), so the
    // live transcript can split a row that straddles an owner<->remote handoff
    // instead of taking `channel`'s lossy whole-row verdict.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channel_runs: Vec<ChannelRun>,
}
