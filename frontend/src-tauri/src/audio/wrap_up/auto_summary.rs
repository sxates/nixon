//! The auto-summary gate at the end of a wrap-up (specs/0083 W2a task 8).
//!
//! Ported from `autoSummarySkipReason` / `autoSummarySkipToast`
//! (`frontend/src/lib/auto-summary.ts`), which ran on `/meeting-details?source=recording`.
//! The Rust gate always runs straight off a recording, so the `not-from-recording` arm is
//! gone. Two inputs changed with the move (specs/0083 §W2a stage 5):
//! - `has_summary` is new: a summary that already exists is never overwritten. The page
//!   skipped the whole gate in that case, silently, so it wins first.
//! - `has_notes` replaces `audioAwaitingTranscription`: the backend summary path grounds on
//!   notes alone when there is no transcript (`start_summary_generation_for_meeting`), but it
//!   never transcribes first, which was what the page's audio probe was for.
//!
//! The test table is `auto-summary.test.ts`, adapted only for those two inputs.

use serde::Serialize;

use super::stop_action::Toast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkipReason {
    /// A summary already exists (resumed meeting, or the user started one). Silent.
    SummaryExists,
    /// `processing_mode='defer'`: the deferred-meeting machinery owns this meeting.
    Deferred,
    /// The backlog is already processing it, or the meeting is marked `live`. Silent.
    DeferredProcessing,
    Disabled,
    EmptyTranscript,
    NoModel,
    /// The gate's inputs couldn't be read.
    ModelCheckFailed,
    /// Every gate passed but the summary couldn't be started.
    StartFailed,
}

#[derive(Debug, Clone, Default)]
pub struct GateInput {
    pub has_summary: bool,
    pub processing_mode: Option<String>,
    /// The deferred backlog accepted this meeting at stop (`process-now` handoff).
    pub in_backlog: bool,
    pub enabled: bool,
    pub transcript_count: i64,
    pub has_notes: bool,
    pub has_model: bool,
}

/// The reason to skip, or `None` to start the summary. First match wins.
pub fn skip_reason(input: &GateInput) -> Option<SkipReason> {
    if input.has_summary {
        return Some(SkipReason::SummaryExists);
    }
    match input.processing_mode.as_deref() {
        Some("defer") if input.in_backlog => return Some(SkipReason::DeferredProcessing),
        Some("defer") => return Some(SkipReason::Deferred),
        Some("live") => return Some(SkipReason::DeferredProcessing),
        _ => {}
    }
    if !input.enabled {
        return Some(SkipReason::Disabled);
    }
    if input.transcript_count <= 0 && !input.has_notes {
        return Some(SkipReason::EmptyTranscript);
    }
    if !input.has_model {
        return Some(SkipReason::NoModel);
    }
    None
}

/// The toast copy for a skip, or `None` when the skip stays silent.
pub fn skip_toast(reason: SkipReason) -> Option<Toast> {
    let (title, description) = match reason {
        SkipReason::SummaryExists | SkipReason::DeferredProcessing => return None,
        SkipReason::Deferred => (
            "Processing deferred",
            "This meeting still needs processing. Use \"Process now\" on the transcript panel, or it will run automatically when back on power.",
        ),
        SkipReason::Disabled => (
            "Auto-summary is off",
            "Turn on \"Summarize automatically when a meeting ends\" in Settings → Recording.",
        ),
        SkipReason::EmptyTranscript => (
            "Auto-summary skipped: transcript is empty",
            "Nothing was transcribed for this meeting.",
        ),
        SkipReason::NoModel => (
            "Auto-summary skipped: no AI model configured",
            "Choose a summary model in Settings to enable auto-summaries.",
        ),
        SkipReason::ModelCheckFailed => (
            "Auto-summary skipped",
            "Could not check your summary model configuration.",
        ),
        SkipReason::StartFailed => (
            "Couldn't start the summary",
            "Open the meeting and generate the summary from there.",
        ),
    };
    Some(Toast {
        title,
        description: Some(description),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing() -> GateInput {
        GateInput {
            enabled: true,
            transcript_count: 12,
            has_model: true,
            ..Default::default()
        }
    }

    fn with(f: impl FnOnce(&mut GateInput)) -> GateInput {
        let mut i = passing();
        f(&mut i);
        i
    }

    #[test]
    fn proceeds_when_all_gates_pass() {
        assert_eq!(skip_reason(&passing()), None);
    }

    #[test]
    fn skips_when_the_toggle_is_off() {
        assert_eq!(
            skip_reason(&with(|i| i.enabled = false)),
            Some(SkipReason::Disabled)
        );
    }

    #[test]
    fn skips_when_the_transcript_is_empty() {
        assert_eq!(
            skip_reason(&with(|i| i.transcript_count = 0)),
            Some(SkipReason::EmptyTranscript)
        );
    }

    #[test]
    fn proceeds_on_an_empty_transcript_when_there_are_notes() {
        assert_eq!(
            skip_reason(&with(|i| {
                i.transcript_count = 0;
                i.has_notes = true;
            })),
            None
        );
    }

    #[test]
    fn skips_when_no_model_is_configured() {
        assert_eq!(
            skip_reason(&with(|i| i.has_model = false)),
            Some(SkipReason::NoModel)
        );
    }

    #[test]
    fn skips_a_deferred_meeting() {
        assert_eq!(
            skip_reason(&with(|i| {
                i.transcript_count = 0;
                i.processing_mode = Some("defer".into());
            })),
            Some(SkipReason::Deferred)
        );
    }

    #[test]
    fn skips_silently_a_live_marked_meeting() {
        assert_eq!(
            skip_reason(&with(|i| {
                i.transcript_count = 5;
                i.processing_mode = Some("live".into());
            })),
            Some(SkipReason::DeferredProcessing)
        );
    }

    #[test]
    fn skips_silently_when_the_backlog_has_the_defer_marked_meeting() {
        let r = skip_reason(&with(|i| {
            i.processing_mode = Some("defer".into());
            i.in_backlog = true;
        }));
        assert_eq!(r, Some(SkipReason::DeferredProcessing));
        assert_eq!(skip_toast(r.unwrap()), None);
    }

    #[test]
    fn still_toasts_processing_deferred_when_not_in_the_backlog() {
        let r = skip_reason(&with(|i| i.processing_mode = Some("defer".into())));
        assert_eq!(r, Some(SkipReason::Deferred));
        assert!(skip_toast(r.unwrap()).is_some());
    }

    #[test]
    fn deferral_outranks_the_toggle_and_transcript_and_model_gates() {
        assert_eq!(
            skip_reason(&with(|i| {
                i.enabled = false;
                i.transcript_count = 0;
                i.has_model = false;
                i.processing_mode = Some("defer".into());
            })),
            Some(SkipReason::Deferred)
        );
    }

    #[test]
    fn an_absent_processing_mode_changes_nothing() {
        assert_eq!(skip_reason(&with(|i| i.processing_mode = None)), None);
    }

    #[test]
    fn an_existing_summary_outranks_everything_and_is_silent() {
        let r = skip_reason(&with(|i| {
            i.has_summary = true;
            i.enabled = false;
            i.processing_mode = Some("defer".into());
        }));
        assert_eq!(r, Some(SkipReason::SummaryExists));
        assert_eq!(skip_toast(r.unwrap()), None);
    }

    #[test]
    fn prioritizes_the_disabled_toggle_over_transcript_and_model_state() {
        assert_eq!(
            skip_reason(&with(|i| {
                i.enabled = false;
                i.transcript_count = 0;
                i.has_model = false;
            })),
            Some(SkipReason::Disabled)
        );
    }

    #[test]
    fn prioritizes_empty_transcript_over_missing_model() {
        assert_eq!(
            skip_reason(&with(|i| {
                i.transcript_count = 0;
                i.has_model = false;
            })),
            Some(SkipReason::EmptyTranscript)
        );
    }

    #[test]
    fn deferred_copy_does_not_claim_low_power_mode() {
        let t = skip_toast(SkipReason::Deferred).unwrap();
        assert!(t.title.contains("deferred"));
        let d = t.description.unwrap();
        assert!(!d.to_lowercase().contains("low power") && !d.to_lowercase().contains("low-power"));
        assert!(d.contains("Process now"));
    }

    #[test]
    fn disabled_copy_says_where_the_toggle_lives() {
        let t = skip_toast(SkipReason::Disabled).unwrap();
        assert_eq!(t.title, "Auto-summary is off");
        assert!(t
            .description
            .unwrap()
            .contains("Summarize automatically when a meeting ends"));
    }

    #[test]
    fn every_non_silent_reason_has_copy() {
        for r in [
            SkipReason::Disabled,
            SkipReason::EmptyTranscript,
            SkipReason::NoModel,
            SkipReason::ModelCheckFailed,
            SkipReason::StartFailed,
        ] {
            assert!(!skip_toast(r).unwrap().title.is_empty());
        }
    }

    #[test]
    fn reasons_serialize_in_the_frontends_spelling() {
        assert_eq!(
            serde_json::to_value(SkipReason::DeferredProcessing).unwrap(),
            "deferred-processing"
        );
        assert_eq!(
            serde_json::to_value(SkipReason::EmptyTranscript).unwrap(),
            "empty-transcript"
        );
    }
}
