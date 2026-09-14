//! Owner-mic mute gate (specs/0049).
//!
//! A process-wide flag the Zoom-mute poll task ([`crate::zoom::mute_monitor`]) sets
//! and the capture path reads: while set, `RecordingState::send_audio_chunk` drops
//! MICROPHONE chunks, so the mixer zero-pads the mic window and muted spans are
//! system-only in the transcript and silent in `mic.wav`. System/remote audio is
//! unaffected. Distinct from pause, which drops both channels.
//!
//! It's a single global (not per-`RecordingState`) so the poll can flip it without a
//! handle to the active recording; `stop_recording` clears it so it never persists
//! across sessions.

use std::sync::atomic::{AtomicBool, Ordering};

static OWNER_MUTED: AtomicBool = AtomicBool::new(false);

/// Set the owner-muted gate. Called by the Zoom-mute poll task.
pub fn set_muted(muted: bool) {
    OWNER_MUTED.store(muted, Ordering::SeqCst);
}

/// Whether the owner mic is currently gated (muted). Read on the capture path.
pub fn is_muted() -> bool {
    OWNER_MUTED.load(Ordering::SeqCst)
}
