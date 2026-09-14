//! The pipeline's VAD/STT gating stage (low-power-mode spec §3).
//!
//! Owns the optional `ContinuousVadProcessor` and a shared `AtomicBool` the
//! session's toggle command flips, so live transcription can attach/detach
//! MID-recording: `sync()` is called once per mix window and lazily
//! constructs (or drops) the VAD to match the flag. Extracted from
//! `pipeline.rs` (specs/0029 WS7.2 record-only gating) — behavior at a fixed
//! flag value is unchanged from the previous inline `Option<ContinuousVadProcessor>`.

use super::vad::{ContinuousVadProcessor, SpeechSegment};
use anyhow::Result;
use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// VAD redemption time (ms) — the value previously inlined in
/// `AudioPipeline::new` (same on all platforms: macOS Core Audio and Windows
/// are both tuned to 400ms).
const REDEMPTION_TIME_MS: u32 = 400;

/// What `sync()` did to the VAD/STT stage this window (spec 0051 WS1). `Detached`
/// carries the flushed tail so the caller can dispatch it — going deferred no longer
/// throws away the last words spoken before the switch.
#[derive(Debug)]
pub enum SyncOutcome {
    Attached,
    Detached(Vec<SpeechSegment>),
    Unchanged,
}

/// Move a batch of VAD segments onto the session timeline (spec 0051 WS1). The VAD's
/// own clock restarts at 0 every time it is constructed, so without this a re-attached
/// stage emits segments that sort to the TOP of the live transcript.
fn shift_segments(segments: &mut [SpeechSegment], offset_ms: f64) {
    for segment in segments.iter_mut() {
        segment.start_timestamp_ms += offset_ms;
        segment.end_timestamp_ms += offset_ms;
    }
}

pub struct SttStage {
    vad: Option<ContinuousVadProcessor>,
    live_stt: Arc<AtomicBool>,
    sample_rate: u32,
    /// Session-audio position (ms) at which the current VAD was attached. Added to
    /// every segment this stage emits. 0 for a session that starts live.
    offset_ms: f64,
}

impl SttStage {
    /// Constructs the VAD up-front when the flag starts true (a start-time
    /// failure must abort the recording start, as before). When the flag
    /// starts false the whole STT-feeding stage is skipped (record-only mode).
    pub fn new(sample_rate: u32, live_stt: Arc<AtomicBool>) -> Result<Self> {
        let vad = if live_stt.load(Ordering::SeqCst) {
            match ContinuousVadProcessor::new(sample_rate, REDEMPTION_TIME_MS) {
                Ok(p) => {
                    info!("VAD-driven pipeline: VAD segments will be sent directly to Whisper (no time-based accumulation)");
                    Some(p)
                }
                Err(e) => {
                    error!("Failed to create VAD processor: {}", e);
                    return Err(anyhow::anyhow!("VAD processor creation failed: {}", e));
                }
            }
        } else {
            info!("🎙️ Record-only mode: live transcription disabled — VAD/STT stage skipped (audio is still recorded; transcribe later)");
            None
        };
        Ok(Self {
            vad,
            live_stt,
            sample_rate,
            offset_ms: 0.0,
        })
    }

    /// True while the VAD/STT stage is attached (live transcription running).
    pub fn is_active(&self) -> bool {
        self.vad.is_some()
    }

    /// Session-audio position the current VAD was attached at.
    pub fn offset_ms(&self) -> f64 {
        self.offset_ms
    }

    /// Reconcile the VAD with the session flag. Called once per mix window,
    /// BEFORE the channel-window check and `process()`, so the clock starts
    /// advancing the same window the stage attaches. `recorded_ms` is the
    /// session's audio position (see `AudioPipeline::recorded_ms`) — an attach
    /// re-bases the stage's offset onto it. A mid-recording attach failure logs
    /// and resets the flag (the session stays deferred, and the offset is left
    /// untouched so a later successful attach still lands correctly) rather than
    /// killing the pipeline.
    pub fn sync(&mut self, recorded_ms: f64) -> SyncOutcome {
        let want = self.live_stt.load(Ordering::SeqCst);
        if want && self.vad.is_none() {
            match ContinuousVadProcessor::new(self.sample_rate, REDEMPTION_TIME_MS) {
                Ok(p) => {
                    info!("🎙️ Live transcription enabled mid-recording — VAD/STT stage attached at {recorded_ms:.0}ms");
                    self.vad = Some(p);
                    self.offset_ms = recorded_ms;
                    SyncOutcome::Attached
                }
                Err(e) => {
                    error!("Mid-recording VAD attach failed (staying deferred): {e}");
                    self.live_stt.store(false, Ordering::SeqCst);
                    SyncOutcome::Unchanged
                }
            }
        } else if !want && self.vad.is_some() {
            // spec 0051 WS1: flush the tail BEFORE dropping the VAD. The previous
            // "drop it, we retranscribe later anyway" rationale died with WS2, which
            // makes that retranscription conditional rather than guaranteed.
            let tail = match self.flush() {
                Some(Ok(segments)) => segments,
                Some(Err(e)) => {
                    warn!("VAD flush at mid-recording detach failed; tail dropped: {e}");
                    Vec::new()
                }
                None => Vec::new(),
            };
            info!(
                "🎙️ Live transcription disabled mid-recording — VAD/STT stage detached ({} tail segment(s))",
                tail.len()
            );
            self.vad = None;
            SyncOutcome::Detached(tail)
        } else {
            SyncOutcome::Unchanged
        }
    }

    /// Feed one mixed window through the VAD, if attached, shifting the returned
    /// segments onto the session timeline. `None` in record-only mode (matching the
    /// previous `Option::map` at the call site, so the segment-dispatch loop is
    /// untouched).
    pub fn process(&mut self, mixed: &[f32]) -> Option<Result<Vec<SpeechSegment>>> {
        let offset_ms = self.offset_ms;
        self.vad.as_mut().map(|vad| {
            vad.process_audio(mixed).map(|mut segments| {
                shift_segments(&mut segments, offset_ms);
                segments
            })
        })
    }

    /// Flush the VAD's remaining audio, if attached, on the session timeline. `None`
    /// in record-only mode — same shape the pipeline's flush consumer expects.
    pub fn flush(&mut self) -> Option<Result<Vec<SpeechSegment>>> {
        let offset_ms = self.offset_ms;
        self.vad.as_mut().map(|vad| {
            vad.flush().map(|mut segments| {
                shift_segments(&mut segments, offset_ms);
                segments
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start_ms: f64, end_ms: f64) -> SpeechSegment {
        SpeechSegment {
            samples: vec![0.0; 1600],
            start_timestamp_ms: start_ms,
            end_timestamp_ms: end_ms,
            confidence: 0.9,
        }
    }

    #[test]
    fn shift_segments_moves_both_bounds() {
        let mut segments = vec![segment(100.0, 900.0), segment(1_200.0, 1_800.0)];
        shift_segments(&mut segments, 60_000.0);
        assert_eq!(segments[0].start_timestamp_ms, 60_100.0);
        assert_eq!(segments[0].end_timestamp_ms, 60_900.0);
        assert_eq!(segments[1].start_timestamp_ms, 61_200.0);
        assert_eq!(segments[1].end_timestamp_ms, 61_800.0);
    }

    #[test]
    fn shift_segments_with_zero_offset_is_identity() {
        let mut segments = vec![segment(100.0, 900.0)];
        shift_segments(&mut segments, 0.0);
        assert_eq!(segments[0].start_timestamp_ms, 100.0);
        assert_eq!(segments[0].end_timestamp_ms, 900.0);
    }

    /// spec 0051 WS1: EVERY attach re-bases the offset onto the caller's session
    /// audio clock. Before this, a re-attached VAD restarted at 0 and its segments
    /// sorted to the TOP of the live transcript.
    #[test]
    fn sync_rebases_offset_on_each_attach() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut stage = SttStage::new(16_000, flag.clone()).expect("construct deferred stage");
        assert!(!stage.is_active(), "starts detached when the flag is false");
        assert_eq!(stage.offset_ms(), 0.0);

        // First go-live, one minute into the recording.
        flag.store(true, Ordering::SeqCst);
        assert!(matches!(stage.sync(60_000.0), SyncOutcome::Attached));
        assert!(stage.is_active());
        assert_eq!(stage.offset_ms(), 60_000.0);

        // Steady state: no repeated attach, offset held.
        assert!(matches!(stage.sync(61_000.0), SyncOutcome::Unchanged));
        assert_eq!(stage.offset_ms(), 60_000.0);

        // Go deferred.
        flag.store(false, Ordering::SeqCst);
        assert!(matches!(stage.sync(90_000.0), SyncOutcome::Detached(_)));
        assert!(!stage.is_active());
        assert!(matches!(stage.sync(95_000.0), SyncOutcome::Unchanged));

        // Second go-live re-bases onto the LATER position, not back to zero.
        flag.store(true, Ordering::SeqCst);
        assert!(matches!(stage.sync(120_000.0), SyncOutcome::Attached));
        assert_eq!(stage.offset_ms(), 120_000.0);
    }

    /// A stage that starts live is anchored at 0 — the session clock is 0 at start.
    #[test]
    fn live_start_anchors_at_zero() {
        let flag = Arc::new(AtomicBool::new(true));
        let stage = SttStage::new(16_000, flag).expect("construct live stage");
        assert!(stage.is_active());
        assert_eq!(stage.offset_ms(), 0.0);
    }

    /// Detaching hands the flushed tail back rather than dropping it on the floor.
    /// Feeding silence means the tail is legitimately empty here, so this asserts the
    /// CONTRACT (a `Detached` carrying a vector the caller can dispatch) — the tail's
    /// contents are covered end-to-end by the `pipeline_integration` test, which feeds
    /// real speech. Do NOT add an `assert!(tail.iter().all(…))` here: on an empty vector
    /// that is vacuously true and asserts nothing.
    #[test]
    fn detach_hands_back_the_flushed_tail() {
        let flag = Arc::new(AtomicBool::new(true));
        let mut stage = SttStage::new(16_000, flag.clone()).expect("construct live stage");
        let _ = stage.process(&vec![0.0f32; 16_000]);
        flag.store(false, Ordering::SeqCst);
        assert!(
            matches!(stage.sync(10_000.0), SyncOutcome::Detached(_)),
            "detach must report the flushed tail, not Unchanged"
        );
        assert!(!stage.is_active(), "the VAD is dropped after the flush");
    }
}
