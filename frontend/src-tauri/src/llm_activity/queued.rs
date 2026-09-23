//! Queued background work (specs/0074 W3).
//!
//! The registry used to know two states: running and finished. A prep pass that planned six
//! briefs therefore showed one at a time and nothing before it. A task can now be *queued*
//! first — planned, visible as "Waiting" — and [`QueuedHandle::start`] moves it to running
//! under the same id. A queued task dropped without starting (skipped, cancelled, found to be
//! up to date after all) disappears silently: it was never work, so it is never history.

use super::registry::{LlmTaskRegistry, Origin, RunningTask, TaskHandle, TaskKind, TaskOutcome};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedTask {
    pub id: u64,
    pub kind: TaskKind,
    pub label: String,
    pub meeting_id: Option<String>,
}

impl LlmTaskRegistry {
    /// Queue a task. `None` if `(kind, meeting_id)` is already queued or running — the dedupe
    /// is the single-flight between the background pass and a manual trigger.
    pub fn enqueue_for(
        self: &Arc<Self>,
        kind: TaskKind,
        origin: Origin,
        label: impl Into<String>,
        meeting_id: Option<String>,
    ) -> Option<QueuedHandle> {
        let mut inner = self.lock();
        let busy = inner
            .queued
            .iter()
            .map(|(_, t)| (t.kind, &t.meeting_id))
            .chain(inner.running.iter().map(|(_, t)| (t.kind, &t.meeting_id)))
            .any(|(k, m)| k == kind && *m == meeting_id);
        if busy {
            return None;
        }
        inner.next_id += 1;
        let id = inner.next_id;
        let label = label.into();
        let task = QueuedTask {
            id,
            kind,
            label,
            meeting_id,
        };
        inner.queued.push((origin, task));
        drop(inner);
        self.notify();
        Some(QueuedHandle {
            registry: Arc::clone(self),
            id,
            started: false,
        })
    }

    /// The queue's "Clear finished": drop success and skip records. Failures stay until they
    /// are dismissed or retried, so the badge is untouched.
    pub fn clear_finished(&self) {
        let mut inner = self.lock();
        inner
            .history
            .retain(|r| matches!(r.outcome, TaskOutcome::Failed { .. }));
        drop(inner);
        self.notify();
    }

    /// Silently drop the queued or running entry for `(kind, meeting_id)` — for a run that was
    /// just cancelled to make way for a replacement. Its handle's later finish finds nothing
    /// and records nothing, so a superseded run is neither a failure nor a duplicate.
    pub fn forget(&self, kind: TaskKind, meeting_id: &str) {
        let matches =
            |k: TaskKind, m: &Option<String>| k == kind && m.as_deref() == Some(meeting_id);
        let mut inner = self.lock();
        inner
            .queued
            .retain(|(_, t)| !matches(t.kind, &t.meeting_id));
        inner
            .running
            .retain(|(_, t)| !matches(t.kind, &t.meeting_id));
        drop(inner);
        self.notify();
    }

    fn unqueue(&self, id: u64) -> Option<(Origin, QueuedTask)> {
        let mut inner = self.lock();
        let pos = inner.queued.iter().position(|(_, t)| t.id == id)?;
        Some(inner.queued.remove(pos))
    }
}

/// RAII handle for a queued task. Unlike [`TaskHandle`], dropping it is not a failure.
pub struct QueuedHandle {
    registry: Arc<LlmTaskRegistry>,
    id: u64,
    started: bool,
}

impl QueuedHandle {
    /// Move queued → running under the same id, in one lock so a concurrent enqueue never
    /// sees the task in neither list.
    pub fn start(mut self) -> TaskHandle {
        self.started = true;
        let mut inner = self.registry.lock();
        if let Some(pos) = inner.queued.iter().position(|(_, t)| t.id == self.id) {
            let (origin, q) = inner.queued.remove(pos);
            let task = RunningTask {
                id: q.id,
                kind: q.kind,
                label: q.label,
                note: None,
                meeting_id: q.meeting_id,
            };
            inner.running.push((origin, task));
        }
        drop(inner);
        self.registry.notify();
        TaskHandle {
            registry: Arc::clone(&self.registry),
            id: self.id,
            finished: false,
        }
    }
}

impl Drop for QueuedHandle {
    fn drop(&mut self) {
        if !self.started && self.registry.unqueue(self.id).is_some() {
            self.registry.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Arc<LlmTaskRegistry> {
        Arc::new(LlmTaskRegistry::new())
    }

    fn prep(r: &Arc<LlmTaskRegistry>, m: &str) -> Option<QueuedHandle> {
        r.enqueue_for(
            TaskKind::PrepBrief,
            Origin::Background,
            format!("Prep {m}"),
            Some(m.into()),
        )
    }

    #[test]
    fn enqueue_for_dedupes_by_kind_and_meeting() {
        let r = reg();
        let first = prep(&r, "m1").expect("first enqueue");
        assert!(prep(&r, "m1").is_none(), "same kind + meeting while queued");
        assert!(
            prep(&r, "m2").is_some(),
            "another meeting is not a duplicate"
        );
        let other_kind = r.enqueue_for(
            TaskKind::ActionItems,
            Origin::Background,
            "x",
            Some("m1".into()),
        );
        assert!(other_kind.is_some(), "another kind is not a duplicate");
        let _running = first.start();
        assert!(
            prep(&r, "m1").is_none(),
            "same kind + meeting while running"
        );
    }

    #[test]
    fn a_running_task_from_start_for_also_blocks_the_enqueue() {
        let r = reg();
        let _t = Arc::clone(&r).start_for(
            TaskKind::PrepBrief,
            Origin::Background,
            "p",
            Some("m1".into()),
        );
        assert!(prep(&r, "m1").is_none());
    }

    #[test]
    fn start_moves_queued_to_running_under_the_same_id() {
        let r = reg();
        let h = prep(&r, "m1").unwrap();
        let v = r.view();
        assert_eq!((v.queued.len(), v.running.len()), (1, 0));
        let id = v.queued[0].id;
        let t = h.start();
        let v = r.view();
        assert!(v.queued.is_empty());
        assert_eq!(v.running[0].id, id);
        assert_eq!(v.running[0].meeting_id.as_deref(), Some("m1"));
        t.finish(Ok(()));
        assert_eq!(r.view().history[0].id, id);
    }

    #[test]
    fn dropping_a_queued_handle_is_silent() {
        let r = reg();
        drop(prep(&r, "m1").unwrap());
        let v = r.view();
        assert!(v.queued.is_empty() && v.running.is_empty() && v.history.is_empty());
        assert!(
            !v.has_failure,
            "a planned-then-skipped task is not a failure"
        );
        assert!(
            prep(&r, "m1").is_some(),
            "and it no longer blocks a new enqueue"
        );
    }

    #[test]
    fn foreground_queued_tasks_stay_out_of_the_view() {
        let r = reg();
        let _h = r.enqueue_for(TaskKind::AskAI, Origin::Foreground, "q", None);
        assert!(r.view().queued.is_empty());
    }

    #[test]
    fn clear_finished_keeps_failures_and_the_badge() {
        let r = reg();
        prep(&r, "ok").unwrap().start().finish(Ok(()));
        prep(&r, "bad").unwrap().start().finish(Err("boom".into()));
        Arc::clone(&r)
            .start_for(TaskKind::ActionItems, Origin::Background, "s", None)
            .finish_skipped("nothing");
        r.clear_finished();
        let v = r.view();
        assert_eq!(v.history.len(), 1);
        assert_eq!(v.history[0].meeting_id.as_deref(), Some("bad"));
        assert!(v.has_failure);
    }

    #[test]
    fn forget_drops_a_superseded_run_without_recording_it() {
        let r = reg();
        let t = prep(&r, "m1").unwrap().start();
        r.forget(TaskKind::PrepBrief, "m1");
        assert!(r.view().running.is_empty());
        assert!(
            prep(&r, "m1").is_some(),
            "the replacement can enqueue at once"
        );
        t.finish(Err("cancelled".into()));
        let v = r.view();
        assert!(v.history.is_empty(), "the superseded run leaves no record");
        assert!(!v.has_failure);
    }
}
