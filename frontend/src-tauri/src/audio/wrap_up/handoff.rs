//! The `process-now` backlog handoff (spec 0051 WS2, moved to Rust by specs/0083 W2a).
//!
//! The deferred backlog still lives in the frontend (`DeferredBacklogProvider`), so the
//! wrap-up emits a `handoff` stage and waits for `api_wrap_up_handoff_result`. No answer
//! within the wait is a refusal: the defer marker is already written, so the meeting
//! degrades into the ordinary deferred case.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

use super::stop_action::HandoffOutcome;

static PENDING: LazyLock<Mutex<HashMap<String, oneshot::Sender<bool>>>> =
    LazyLock::new(Default::default);

fn pending() -> std::sync::MutexGuard<'static, HashMap<String, oneshot::Sender<bool>>> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Start waiting for `meeting_id`'s answer. Register BEFORE emitting the `handoff` stage,
/// so an instant answer is never lost.
pub fn register(meeting_id: &str) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel();
    pending().insert(meeting_id.to_string(), tx);
    rx
}

/// Deliver the frontend's answer. False when nobody was waiting (too late, or unknown).
pub fn answer(meeting_id: &str, accepted: bool) -> bool {
    match pending().remove(meeting_id) {
        Some(tx) => tx.send(accepted).is_ok(),
        None => false,
    }
}

/// Wait up to `timeout`. No answer, a dropped sender or a refusal all count as refused.
pub async fn wait(
    meeting_id: &str,
    rx: oneshot::Receiver<bool>,
    timeout: Duration,
) -> HandoffOutcome {
    let accepted = matches!(tokio::time::timeout(timeout, rx).await, Ok(Ok(true)));
    pending().remove(meeting_id);
    if accepted {
        HandoffOutcome::Accepted
    } else {
        HandoffOutcome::Refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_answer_reaches_the_waiter() {
        let rx = register("h-accept");
        assert!(answer("h-accept", true));
        assert_eq!(
            wait("h-accept", rx, Duration::from_secs(5)).await,
            HandoffOutcome::Accepted
        );
    }

    #[tokio::test]
    async fn a_refusal_or_silence_is_refused_and_a_late_answer_is_dropped() {
        let rx = register("h-refuse");
        assert!(answer("h-refuse", false));
        assert_eq!(
            wait("h-refuse", rx, Duration::from_secs(5)).await,
            HandoffOutcome::Refused
        );

        let rx = register("h-silent");
        assert_eq!(
            wait("h-silent", rx, Duration::from_millis(20)).await,
            HandoffOutcome::Refused
        );
        assert!(!answer("h-silent", true), "nobody waits after the timeout");
    }
}
