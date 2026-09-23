// Debounced "which call is live" state machine (specs/0008 P1, generalised in specs/0074 W6).
//
// Pure: no process, Core Audio or Tauri deps, so it is unit-tested deterministically. The
// monitor feeds it one raw observation per poll (`None` = no call, `Some(platform)` = the
// classifier's answer) and acts on the confirmed transitions it returns.
//
// With one platform this is exactly the pre-0074 Zoom machine: a flip needs `threshold`
// consecutive polls that agree with each other and disagree with the confirmed state.

/// What the machine wants the caller to do after a poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition<T> {
    /// No confirmed change this poll.
    None,
    /// Confirmed no call -> call on `T`.
    Started(T),
    /// Confirmed call on `T` -> no call.
    Ended(T),
    /// Confirmed call on one platform -> call on another, with no idle poll between them.
    /// The caller handles it as an `Ended` followed by a `Started`.
    Switched { ended: T, started: T },
}

/// Debounced `Option<T>` state machine.
#[derive(Debug)]
pub struct DebounceMachine<T> {
    state: Option<T>,
    /// The raw value that currently disagrees with `state`, and for how many consecutive polls.
    pending: Option<(Option<T>, u8)>,
    threshold: u8,
}

impl<T: Copy + Eq> DebounceMachine<T> {
    pub fn new(threshold: u8) -> Self {
        Self {
            state: None,
            pending: None,
            // threshold must be >= 1; a 0 threshold would flip on every poll.
            threshold: threshold.max(1),
        }
    }

    /// The confirmed state: the platform of the live call, or `None`.
    pub fn state(&self) -> Option<T> {
        self.state
    }

    /// Feed one raw observation. Returns the confirmed transition, if any.
    pub fn observe(&mut self, raw: Option<T>) -> Transition<T> {
        if raw == self.state {
            // Raw signal matches the confirmed state: reset any pending flip.
            self.pending = None;
            return Transition::None;
        }

        // Raw signal disagrees: count toward a flip, but only polls that agree with EACH
        // OTHER count as consecutive.
        let count = match self.pending {
            Some((candidate, n)) if candidate == raw => n.saturating_add(1),
            _ => 1,
        };
        if count < self.threshold {
            self.pending = Some((raw, count));
            return Transition::None;
        }

        // Confirmed flip.
        self.pending = None;
        let old = std::mem::replace(&mut self.state, raw);
        match (old, raw) {
            (None, Some(started)) => Transition::Started(started),
            (Some(ended), None) => Transition::Ended(ended),
            (Some(ended), Some(started)) => Transition::Switched { ended, started },
            // raw != state, so both cannot be None.
            (None, None) => Transition::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The pre-0074 Zoom cases, unchanged in meaning: `Some(Z)` is "a meeting helper is present".
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum P {
        Z,
        T,
    }
    use P::{T as TEAMS, Z};

    #[test]
    fn starts_idle() {
        let m = DebounceMachine::<P>::new(2);
        assert_eq!(m.state(), None);
    }

    #[test]
    fn requires_consecutive_polls_to_enter_meeting() {
        let mut m = DebounceMachine::new(2);
        assert_eq!(m.observe(Some(Z)), Transition::None);
        assert_eq!(m.state(), None);
        assert_eq!(m.observe(Some(Z)), Transition::Started(Z));
        assert_eq!(m.state(), Some(Z));
    }

    #[test]
    fn single_poll_flicker_does_not_flip() {
        let mut m = DebounceMachine::new(2);
        assert_eq!(m.observe(Some(Z)), Transition::None);
        assert_eq!(m.observe(None), Transition::None);
        assert_eq!(m.state(), None);
    }

    #[test]
    fn requires_consecutive_polls_to_end_meeting() {
        let mut m = DebounceMachine::new(2);
        m.observe(Some(Z));
        assert_eq!(m.observe(Some(Z)), Transition::Started(Z));
        assert_eq!(m.observe(None), Transition::None);
        assert_eq!(m.state(), Some(Z));
        assert_eq!(m.observe(None), Transition::Ended(Z));
        assert_eq!(m.state(), None);
    }

    #[test]
    fn blip_during_meeting_resets_pending_end() {
        let mut m = DebounceMachine::new(2);
        m.observe(Some(Z));
        assert_eq!(m.observe(Some(Z)), Transition::Started(Z));
        assert_eq!(m.observe(None), Transition::None);
        assert_eq!(m.observe(Some(Z)), Transition::None);
        assert_eq!(m.observe(None), Transition::None);
        assert_eq!(m.state(), Some(Z));
    }

    #[test]
    fn threshold_zero_is_clamped_to_one() {
        let mut m = DebounceMachine::new(0);
        assert_eq!(m.observe(Some(Z)), Transition::Started(Z));
    }

    #[test]
    fn full_cycle() {
        let mut m = DebounceMachine::new(2);
        assert_eq!(m.observe(Some(Z)), Transition::None);
        assert_eq!(m.observe(Some(Z)), Transition::Started(Z));
        assert_eq!(m.observe(Some(Z)), Transition::None); // steady in-meeting
        assert_eq!(m.observe(None), Transition::None);
        assert_eq!(m.observe(None), Transition::Ended(Z));
        assert_eq!(m.observe(None), Transition::None); // steady idle
    }

    #[test]
    fn disagreeing_candidates_do_not_add_up() {
        // One poll of Zoom and one of Teams are not two agreeing polls of anything.
        let mut m = DebounceMachine::new(2);
        assert_eq!(m.observe(Some(Z)), Transition::None);
        assert_eq!(m.observe(Some(TEAMS)), Transition::None);
        assert_eq!(m.state(), None);
        assert_eq!(m.observe(Some(TEAMS)), Transition::Started(TEAMS));
    }

    #[test]
    fn a_platform_change_ends_the_old_call_and_starts_the_new_one() {
        let mut m = DebounceMachine::new(2);
        m.observe(Some(Z));
        assert_eq!(m.observe(Some(Z)), Transition::Started(Z));
        assert_eq!(m.observe(Some(TEAMS)), Transition::None);
        assert_eq!(
            m.observe(Some(TEAMS)),
            Transition::Switched {
                ended: Z,
                started: TEAMS
            }
        );
        assert_eq!(m.state(), Some(TEAMS));
    }
}
