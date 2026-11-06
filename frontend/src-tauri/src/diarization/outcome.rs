//! specs/0084 W2 — how a speaker-identification pass ended.
//!
//! [`PassOutcome`] is the one value every pass ending is routed through: the launcher
//! (`launch.rs`) builds it from the pass's result and hands it to
//! [`crate::audio::lifecycle::on_diarization_outcome`], which owns the DB side (the
//! crash-loop marker, `audio_state`, `speaker_id_failure`). Other callers that end a pass
//! (specs/0083 W2) build one the same way and route it through the same hook.

use crate::audio::lifecycle::state::SpeakerIdFailure;

use super::cancel::is_cancelled;

/// Why a pass failed: the stored failure code and the message for the log and the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassFailure {
    /// Persisted as `meetings.speaker_id_failure` when the meeting becomes `failed`.
    pub code: SpeakerIdFailure,
    /// User-actionable error text (the `anyhow` chain, `{:#}`).
    pub message: String,
}

/// How a speaker-identification pass ended.
///
/// - `Ok`: speakers were persisted. The meeting is stamped identified and reevaluated.
/// - `Failed`: a real failure. A pending, transcribed meeting becomes `failed` with the
///   failure code; the queue shows a red row with Retry and an error toast is shown.
/// - `Cancelled`: the user cancelled, or the meeting was deleted mid-pass. A quiet skip:
///   no red failure, no toast. A pending meeting becomes `failed`/`cancelled` so neither
///   the startup reconcile nor an automatic caller starts it again; nothing is undone on
///   relaunch. Any queued re-run is dropped. Like every ending, it clears the
///   `speaker_id_started_at` crash-loop marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassOutcome {
    Ok,
    Failed(PassFailure),
    Cancelled,
}

impl PassOutcome {
    /// Classify a pass's result. A [`super::cancel::Cancelled`] error anywhere in the chain
    /// is a cancel; so is any error once the meeting no longer exists (`meeting_exists` =
    /// false), since a deleted meeting must end its pass quietly.
    pub fn from_result(result: &anyhow::Result<()>, meeting_exists: bool) -> Self {
        match result {
            Ok(()) => Self::Ok,
            Err(e) if is_cancelled(e) || !meeting_exists => Self::Cancelled,
            Err(e) => Self::Failed(PassFailure {
                code: SpeakerIdFailure::Error,
                message: format!("{e:#}"),
            }),
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{anyhow, Context};

    #[test]
    fn classifies_ok_cancel_and_failure() {
        assert_eq!(PassOutcome::from_result(&Ok(()), true), PassOutcome::Ok);
        let cancelled: anyhow::Result<()> =
            Err(anyhow!(super::super::cancel::Cancelled)).context("persist");
        assert_eq!(
            PassOutcome::from_result(&cancelled, true),
            PassOutcome::Cancelled
        );
        let failed: anyhow::Result<()> = Err(anyhow!("no models"));
        assert_eq!(
            PassOutcome::from_result(&failed, true),
            PassOutcome::Failed(PassFailure {
                code: SpeakerIdFailure::Error,
                message: "no models".into()
            })
        );
        assert_eq!(
            PassOutcome::from_result(&failed, false),
            PassOutcome::Cancelled,
            "a deleted meeting's pass ends quietly"
        );
    }
}
