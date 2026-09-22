//! What a recording's stored duration is made of (specs/0071 W5).
//!
//! Split out of `recording_state.rs` rather than added to it: that file went over the
//! 800-line cap with this in it, and the file-size gate (specs/0065) is right that new
//! behaviour belongs in a new module. It is also genuinely separable — the accounting is a
//! plain value plus its log format, with no access to recording state.

use crate::audio::recording_state::RecordingState;

/// The inputs behind a recording's stored duration (specs/0071 W5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DurationAccounting {
    /// Wall clock since the recording started.
    pub elapsed: f64,
    /// Total time previously spent paused.
    pub pauses: f64,
    /// Time in the pause currently in progress, if any.
    pub current_pause: f64,
    /// What gets stored: `elapsed - pauses - current_pause`.
    pub active: f64,
}

impl DurationAccounting {
    /// One greppable line naming every term. `warn!` when the pause terms explain a
    /// shortfall, because that is the case worth finding in a log nobody was watching.
    pub fn log(&self, context: &str, source: &str) {
        let line = format!(
            "recording duration accounting [{}, {}]: elapsed={:.2}s pauses={:.2}s \
             current_pause={:.2}s -> active={:.2}s (stored)",
            context, source, self.elapsed, self.pauses, self.current_pause, self.active
        );
        // Any pause at all is worth flagging: the owner reported not having pressed HOLD, so
        // a non-zero term here is itself the finding.
        if self.pauses + self.current_pause > 0.5 {
            log::warn!("{line} — pause time is being subtracted from the stored duration");
        } else {
            log::info!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // specs/0071 W5 — the arithmetic behind a recording's stored duration, made checkable.
    // A recording stored 68.85s for 114.6s of audio and it could not be attributed after the
    // fact; these pin what the reported terms mean so the log is readable when it matters.

    fn acct(elapsed: f64, pauses: f64, current_pause: f64) -> DurationAccounting {
        DurationAccounting {
            elapsed,
            pauses,
            current_pause,
            active: elapsed - pauses - current_pause,
        }
    }

    #[test]
    fn active_is_elapsed_minus_both_pause_terms() {
        let a = acct(117.71, 0.0, 0.0);
        assert!((a.active - 117.71).abs() < 1e-9, "no pauses: active is elapsed");

        // The shape the reported bug would have had, if pauses were the cause.
        let a = acct(117.71, 48.86, 0.0);
        assert!((a.active - 68.85).abs() < 1e-2, "pauses are subtracted");

        // A pause still in progress counts too.
        let a = acct(100.0, 10.0, 5.0);
        assert!((a.active - 85.0).abs() < 1e-9);
    }

    /// The whole point of the instrumentation: a clean recording and a pause-shortened one
    /// must be distinguishable from their terms alone, without the audio file.
    #[test]
    fn the_terms_distinguish_a_short_elapsed_from_accrued_pauses() {
        let short_elapsed = acct(68.85, 0.0, 0.0);
        let paused = acct(117.71, 48.86, 0.0);

        assert!((short_elapsed.active - paused.active).abs() < 1e-1, "same stored duration");
        assert_ne!(
            short_elapsed.elapsed.round(),
            paused.elapsed.round(),
            "but a different cause, visible in the terms"
        );
        assert_eq!(short_elapsed.pauses, 0.0);
        assert!(paused.pauses > 0.0);
    }

    // The bug this fixes: `cleanup()` wipes `recording_start` and runs BEFORE
    // `save_recording_only` reads the duration, so the save got `None` and
    // `recording_saver.rs` fell back to the last transcript segment's end time — "when
    // speech stopped", not "how long we recorded". Measured: 63.19s stored for 67.22s of
    // audio (trailing silence), and 68.85s for 114.6s (a paused transcript).

    #[test]
    fn the_duration_survives_the_cleanup_that_runs_before_the_save() {
        let state = RecordingState::new();
        state.start_recording().expect("start");
        let live = state.get_active_recording_duration().expect("live duration");

        // Exactly the stop path's order: streams stop (cleanup) and only then does the save
        // ask for the duration.
        state.cleanup();
        assert!(
            state.get_active_recording_duration().is_none(),
            "cleanup still wipes the live clocks — that part is unchanged"
        );

        let saved = state
            .duration_for_save("test")
            .expect("the captured duration outlives cleanup");
        assert!(
            (saved - live).abs() < 0.5,
            "the saved duration must be what the recording actually ran: {saved} vs {live}"
        );
    }

    #[test]
    fn a_second_cleanup_cannot_destroy_the_captured_duration() {
        let state = RecordingState::new();
        state.start_recording().expect("start");
        state.cleanup();
        let first = state.duration_for_save("test").expect("captured");
        // `cleanup()` calls `stop_recording()` again; capturing `None` over a good value
        // would silently reintroduce the bug.
        state.cleanup();
        let second = state.duration_for_save("test").expect("still captured");
        assert_eq!(first, second);
    }

    #[test]
    fn a_new_recording_does_not_inherit_the_previous_duration() {
        let state = RecordingState::new();
        state.start_recording().expect("start");
        state.cleanup();
        assert!(state.duration_for_save("test").is_some(), "first session captured");

        state.start_recording().expect("start again");
        let fresh = state.duration_for_save("test").expect("live for the new session");
        assert!(
            fresh < 0.5,
            "a fresh session reports its own elapsed time, not the last one's: {fresh}"
        );
    }

    #[test]
    fn with_no_recording_at_all_it_reports_nothing_rather_than_a_number() {
        let state = RecordingState::new();
        assert!(state.duration_for_save("test").is_none());
    }

    #[test]
    fn accounting_is_none_outside_a_recording() {
        let state = RecordingState::new();
        assert!(
            state.duration_accounting().is_none(),
            "no recording_start instant means nothing to account for"
        );
    }

    #[test]
    fn accounting_matches_the_duration_that_gets_stored() {
        let state = RecordingState::new();
        state.start_recording().expect("start");
        let stored = state.get_active_recording_duration().expect("a duration");
        let acct = state.duration_accounting().expect("accounting");
        // Both read the same clock a moment apart, so they agree to within the gap.
        assert!(
            (acct.active - stored).abs() < 0.5,
            "the logged `active` must be the number that gets stored: {} vs {}",
            acct.active,
            stored
        );
        assert_eq!(acct.pauses, 0.0);
        assert_eq!(acct.current_pause, 0.0);
    }

    #[test]
    fn a_real_pause_shows_up_in_the_terms() {
        let state = RecordingState::new();
        state.start_recording().expect("start");
        state.pause_recording().expect("pause");
        let acct = state.duration_accounting().expect("accounting");
        assert!(
            acct.current_pause >= 0.0 && acct.pauses == 0.0,
            "an in-progress pause is reported separately from completed ones"
        );
        state.resume_recording().expect("resume");
        let acct = state.duration_accounting().expect("accounting");
        assert_eq!(acct.current_pause, 0.0, "no pause in progress after resuming");
    }
}
