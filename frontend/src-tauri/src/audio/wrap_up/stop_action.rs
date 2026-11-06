//! Stop-time processing-mode decisions (specs/0083 W2a task 6).
//!
//! A line-for-line port of `stopAction` / `shouldAutoDiarizeAtStop`
//! (`frontend/src/lib/processing-mode.ts`) and `stopFollowUp`
//! (`frontend/src/lib/stop-processing-handoff.ts`). The test tables below are copied from
//! `processing-mode.test.ts` and `stop-processing-handoff.test.ts`; keep them in step.

use serde::Serialize;

/// What the stop path does with this meeting's processing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopAction {
    /// Was deferred at some point but ends overridden live: hand the meeting to the deferred
    /// backlog for the full uniform pass (transcription + diarization) right away.
    ProcessNow,
    /// Ends still deferred, or the user deferred mid-meeting: mark it so the backlog picks it
    /// up and retention doesn't sweep it. Nothing else runs at stop (1.10 feedback).
    MarkDefer,
    /// A normal live meeting with no deferred involvement.
    None,
}

/// - `started_deferred`: the session STARTED with live transcription off.
/// - `ever_deferred`: the session was deferred at ANY point (a live→defer→live session has
///   `started_deferred = false` but `ever_deferred = true`).
/// - `override_mode`: the meeting's `processing_mode` override (`live` | `defer` | none).
pub fn stop_action(
    started_deferred: bool,
    ever_deferred: bool,
    override_mode: Option<&str>,
) -> StopAction {
    let was_deferred = started_deferred || ever_deferred;
    if override_mode == Some("live") && was_deferred {
        return StopAction::ProcessNow;
    }
    if (was_deferred && override_mode != Some("live")) || override_mode == Some("defer") {
        return StopAction::MarkDefer;
    }
    StopAction::None
}

/// Run the stop-time diarization only for a normal live meeting.
pub fn should_auto_diarize_at_stop(action: StopAction) -> bool {
    action == StopAction::None
}

/// Whether the deferred backlog took the meeting. `None` = no answer (treated as refused).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffOutcome {
    Accepted,
    Refused,
}

/// A user-facing toast the frontend shows when visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Toast {
    pub title: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopFollowUp {
    /// Finish the audio at stop (`lifecycle::finish_processing`).
    pub auto_diarize: bool,
    /// Leave `processing_mode='defer'` in place so a later pass can retry.
    pub defer_marker: bool,
    pub toast: Option<Toast>,
}

pub const HANDOFF_FAILED_TOAST: Toast = Toast {
    title: "Couldn't start processing this meeting",
    description: Some(
        "Your recording is saved. Nixon will retry the next time it starts, or use \"Process now\" on the transcript.",
    ),
};

/// `handoff` is `None` for any action other than `ProcessNow` (no handoff was attempted).
/// For `ProcessNow` a `None` handoff means no answer, which counts as a failure.
pub fn stop_follow_up(action: StopAction, handoff: Option<HandoffOutcome>) -> StopFollowUp {
    if action != StopAction::ProcessNow {
        return StopFollowUp {
            auto_diarize: should_auto_diarize_at_stop(action),
            defer_marker: action == StopAction::MarkDefer,
            toast: None,
        };
    }
    // The marker stays either way: on success the backlog clears it as its last step; on
    // failure it is what makes the meeting recoverable.
    if handoff == Some(HandoffOutcome::Accepted) {
        return StopFollowUp {
            auto_diarize: false,
            defer_marker: true,
            toast: None,
        };
    }
    StopFollowUp {
        auto_diarize: true,
        defer_marker: true,
        toast: Some(HANDOFF_FAILED_TOAST),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // processing-mode.test.ts — `stopAction(startedDeferred, everDeferred, overrideMode)`.
    #[test]
    fn stop_action_table() {
        // processes immediately when a deferred-started session was overridden live
        assert_eq!(
            stop_action(true, true, Some("live")),
            StopAction::ProcessNow
        );
        // marks a still-deferred session for backlog
        assert_eq!(stop_action(true, true, None), StopAction::MarkDefer);
        // marks a live session the user deferred mid-meeting
        assert_eq!(
            stop_action(false, true, Some("defer")),
            StopAction::MarkDefer
        );
        // does nothing for a normal live meeting that was never deferred
        assert_eq!(stop_action(false, false, None), StopAction::None);
        // live-started, deferred mid-way, then overridden back to live → repair pass
        assert_eq!(
            stop_action(false, true, Some("live")),
            StopAction::ProcessNow
        );
        // live-started, deferred mid-way and NOT overridden back → mark-defer
        assert_eq!(stop_action(false, true, None), StopAction::MarkDefer);
    }

    #[test]
    fn should_auto_diarize_at_stop_table() {
        assert!(should_auto_diarize_at_stop(StopAction::None));
        assert!(!should_auto_diarize_at_stop(StopAction::ProcessNow));
        assert!(!should_auto_diarize_at_stop(StopAction::MarkDefer));
    }

    // stop-processing-handoff.test.ts
    #[test]
    fn leaves_a_plain_live_meeting_on_the_normal_auto_diarize_path() {
        assert_eq!(
            stop_follow_up(StopAction::None, None),
            StopFollowUp {
                auto_diarize: true,
                defer_marker: false,
                toast: None
            }
        );
    }

    #[test]
    fn marks_a_still_deferred_meeting_and_runs_nothing_at_stop() {
        assert_eq!(
            stop_follow_up(StopAction::MarkDefer, None),
            StopFollowUp {
                auto_diarize: false,
                defer_marker: true,
                toast: None
            }
        );
    }

    #[test]
    fn stays_quiet_when_the_process_now_handoff_was_accepted() {
        assert_eq!(
            stop_follow_up(StopAction::ProcessNow, Some(HandoffOutcome::Accepted)),
            StopFollowUp {
                auto_diarize: false,
                defer_marker: true,
                toast: None
            }
        );
    }

    #[test]
    fn falls_back_to_auto_diarize_and_warns_when_the_handoff_is_refused() {
        let r = stop_follow_up(StopAction::ProcessNow, Some(HandoffOutcome::Refused));
        assert!(r.auto_diarize);
        assert!(r.defer_marker);
        assert_eq!(r.toast, Some(HANDOFF_FAILED_TOAST));
    }

    #[test]
    fn treats_a_missing_handoff_result_for_process_now_as_a_failure() {
        let r = stop_follow_up(StopAction::ProcessNow, None);
        assert!(r.auto_diarize);
        assert!(r.toast.is_some());
        assert_eq!(
            r,
            stop_follow_up(StopAction::ProcessNow, Some(HandoffOutcome::Refused))
        );
    }

    #[test]
    fn always_keeps_the_defer_marker_for_a_process_now_meeting() {
        for h in [
            Some(HandoffOutcome::Accepted),
            Some(HandoffOutcome::Refused),
            None,
        ] {
            assert!(stop_follow_up(StopAction::ProcessNow, h).defer_marker);
        }
    }
}
