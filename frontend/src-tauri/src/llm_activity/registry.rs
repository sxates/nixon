//! In-memory registry of background LLM work (specs/0052).
//!
//! Tracks TASKS, not individual LLM calls: `generate_summary` is the single choke point for
//! every provider call, but it is too low-level to name a unit of work — one summary is many
//! calls. Jobs declare a task; the calls inside report progress against it.
//!
//! Every operation is infallible by construction. A poisoned lock must never break
//! generation (the specs/0034 extraction contract).

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Finished outcomes retained for the popover. In-memory only: cleared on restart, and
/// deliberately not persisted so prompt/error text never lands on disk.
pub const HISTORY_CAP: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskKind {
    PrepBrief,
    MeetingSummary,
    ActionItems,
    NoteEnhancement,
    AskAI,
    Rollup,
    /// specs/0063 W3 — an offline diarization pass. Not an LLM task, but the queue is the
    /// one place the user looks to find out what the machine is busy with.
    Diarization,
}

/// `Background` tasks surface in the sidebar indicator. `Foreground` tasks are recorded for
/// observability but already have their own progress UI (ChunkProgressDisplay, the deferred
/// backlog pill, Ask AI's inline progress), so surfacing them would double up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Background,
    Foreground,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningTask {
    pub id: u64,
    pub kind: TaskKind,
    pub label: String,
    pub note: Option<String>,
    /// The meeting this task acts on, when it has one. Required for `PrepBrief` so the
    /// popover's Retry can address the right `meeting_briefs` row.
    pub meeting_id: Option<String>,
}

/// Terminal outcome of a finished task. Distinct from a plain `error: Option<String>`
/// so a deliberate skip (nothing was wrong; there was simply nothing to do) can be told
/// apart from a failure — a skip is not an error and must not render as one.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TaskOutcome {
    Success,
    Failed { error: String },
    /// specs/0053 W3: work deliberately not done, with the reason — e.g. the Auto
    /// outline found no commitments, so action-item extraction was skipped.
    Skipped { reason: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: u64,
    pub kind: TaskKind,
    pub label: String,
    /// `None` on success or skip; the error message on failure. Kept alongside
    /// [`outcome`](Self::outcome) for existing consumers that only care about
    /// pass/fail; `outcome` is the source of truth for rendering.
    pub error: Option<String>,
    /// Carried over from [`RunningTask`] so a failed prep brief stays retryable.
    pub meeting_id: Option<String>,
    pub outcome: TaskOutcome,
    /// Carried over from the running task's origin, but NOT serialized to the frontend
    /// (`view()` deliberately still returns only what it already did — a wider change
    /// than this fix needs). Used only by [`LlmTaskRegistry::take_record`]'s `has_failure`
    /// recompute, so a foreground failure (already shown inline; specs/0052) can never
    /// re-light the badge when a background failure elsewhere is taken.
    #[serde(skip)]
    pub origin: Origin,
}

/// What the frontend renders. Only background work is included.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlmActivityView {
    pub running: Vec<RunningTask>,
    /// Newest first, capped at [`HISTORY_CAP`].
    pub history: Vec<TaskRecord>,
    /// Sticky until dismissed — the whole point of the feature.
    pub has_failure: bool,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    running: Vec<(Origin, RunningTask)>,
    history: VecDeque<TaskRecord>,
    has_failure: bool,
}

#[derive(Default)]
pub struct LlmTaskRegistry {
    inner: Mutex<Inner>,
    /// Set once at startup so transitions can notify the frontend. Optional because the
    /// registry is constructed before the app handle exists (and in tests there is none).
    app: Mutex<Option<tauri::AppHandle>>,
}

/// Tauri-managed wrapper so the registry can be shared with command handlers.
pub struct LlmActivityState(pub Arc<LlmTaskRegistry>);

impl LlmTaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A poisoned mutex must not take generation down with it — recover the guard.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Attach the app handle so transitions emit `llm-activity-changed`.
    pub fn attach(&self, app: tauri::AppHandle) {
        *self.app.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);
    }

    /// Emit the current view. Must be called with the inner lock RELEASED — `view()`
    /// re-locks, so holding the guard here would deadlock.
    fn notify(&self) {
        let handle = self.app.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(app) = handle {
            crate::llm_activity::commands::emit_activity(&app, &self.view());
        }
    }

    pub fn start(
        self: &Arc<Self>,
        kind: TaskKind,
        origin: Origin,
        label: impl Into<String>,
    ) -> TaskHandle {
        self.start_for(kind, origin, label, None)
    }

    /// [`start`](Self::start) with the meeting this task acts on — use for `PrepBrief` so
    /// the popover's Retry has something to address.
    pub fn start_for(
        self: &Arc<Self>,
        kind: TaskKind,
        origin: Origin,
        label: impl Into<String>,
        meeting_id: Option<String>,
    ) -> TaskHandle {
        let mut inner = self.lock();
        inner.next_id += 1;
        let id = inner.next_id;
        inner.running.push((
            origin,
            RunningTask {
                id,
                kind,
                label: label.into(),
                note: None,
                meeting_id,
            },
        ));
        drop(inner);
        self.notify();
        TaskHandle {
            registry: Arc::clone(self),
            id,
            finished: false,
        }
    }

    pub fn view(&self) -> LlmActivityView {
        let inner = self.lock();
        LlmActivityView {
            running: inner
                .running
                .iter()
                .filter(|(origin, _)| *origin == Origin::Background)
                .map(|(_, t)| t.clone())
                .collect(),
            history: inner.history.iter().cloned().collect(),
            has_failure: inner.has_failure,
        }
    }

    /// Every running task regardless of origin — for tests and diagnostics.
    pub fn all_running_count(&self) -> usize {
        self.lock().running.len()
    }

    /// Acknowledge failures. History is retained; only the badge clears.
    pub fn dismiss(&self) {
        self.lock().has_failure = false;
        self.notify();
    }

    /// The kind of the history record at `id`, without removing it — lets a caller decide
    /// whether it is even going to act (e.g. `retry_task` checking retryability) before
    /// paying the cost of `take_record`, so a decision to refuse never destroys the record
    /// it refused to touch (specs/0063 W3 Task 6).
    pub fn peek_kind(&self, id: u64) -> Option<TaskKind> {
        self.lock().history.iter().find(|r| r.id == id).map(|r| r.kind)
    }

    /// The meeting id of the history record at `id`, without removing it — same
    /// `peek`-before-`take_record` shape as [`peek_kind`](Self::peek_kind), so a caller
    /// can validate every precondition (retryable kind AND a meeting id present) before
    /// destroying the record (specs/0063 W3 fix round, M4). The outer `Option` is
    /// "record found at all"; the inner one is the record's own optional `meeting_id`.
    pub fn peek_meeting_id(&self, id: u64) -> Option<Option<String>> {
        self.lock()
            .history
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.meeting_id.clone())
    }

    /// Remove a finished record and hand it back — used by Retry (specs/0063 W3), which must
    /// clear the row and the lamp it is retrying, not leave them behind looking untouched.
    /// `has_failure` is recomputed from what remains rather than simply cleared: retrying one
    /// of two failures must not silently acknowledge the other.
    pub fn take_record(&self, id: u64) -> Option<TaskRecord> {
        let mut inner = self.lock();
        let pos = inner.history.iter().position(|r| r.id == id)?;
        let record = inner.history.remove(pos)?;
        // specs/0063 W3 fix round (I3): only a BACKGROUND failure may re-light the badge.
        // A `Foreground` failure (e.g. Ask AI) already has its own inline error UI — see
        // the `Origin` doc comment above — so it must never resurrect `has_failure` when
        // an unrelated background record is taken out from under it.
        inner.has_failure = inner.history.iter().any(|r| {
            r.origin == Origin::Background && matches!(r.outcome, TaskOutcome::Failed { .. })
        });
        drop(inner);
        self.notify();
        Some(record)
    }

    fn set_note(&self, id: u64, note: String) {
        let mut inner = self.lock();
        if let Some((_, task)) = inner.running.iter_mut().find(|(_, t)| t.id == id) {
            task.note = Some(note);
        }
        drop(inner);
        self.notify();
    }

    fn complete(&self, id: u64, error: Option<String>) {
        let outcome = match &error {
            Some(e) => TaskOutcome::Failed { error: e.clone() },
            None => TaskOutcome::Success,
        };
        let raises_failure = error.is_some();
        self.finish_task(id, error, outcome, raises_failure);
    }

    /// specs/0053 W3: record a deliberate skip. Distinct from [`complete`](Self::complete)
    /// with `None` (success — the work ran and produced nothing) and from a failure — a
    /// skip must never raise the sticky badge or render as a red row.
    fn complete_skipped(&self, id: u64, reason: String) {
        self.finish_task(id, None, TaskOutcome::Skipped { reason }, false);
    }

    fn finish_task(
        &self,
        id: u64,
        error: Option<String>,
        outcome: TaskOutcome,
        raises_failure: bool,
    ) {
        let mut inner = self.lock();
        let Some(pos) = inner.running.iter().position(|(_, t)| t.id == id) else {
            return;
        };
        let (origin, task) = inner.running.remove(pos);
        if raises_failure && origin == Origin::Background {
            inner.has_failure = true;
        }
        inner.history.push_front(TaskRecord {
            id: task.id,
            kind: task.kind,
            label: task.label,
            error,
            meeting_id: task.meeting_id,
            outcome,
            origin,
        });
        while inner.history.len() > HISTORY_CAP {
            inner.history.pop_back();
        }
        drop(inner);
        self.notify();
    }
}

/// RAII handle. Dropping without [`finish`](Self::finish) records the task as failed, so a
/// job that panics or returns early can never leave a phantom "running" entry in the sidebar.
pub struct TaskHandle {
    registry: Arc<LlmTaskRegistry>,
    id: u64,
    finished: bool,
}

impl TaskHandle {
    pub fn progress(&self, note: impl Into<String>) {
        self.registry.set_note(self.id, note.into());
    }

    pub fn finish(mut self, result: Result<(), String>) {
        self.finished = true;
        self.registry.complete(self.id, result.err());
    }

    /// Terminal state for work that was deliberately not done, with the reason.
    /// Distinct from success (nothing happened) and from failure (a skip is not
    /// an error and must not surface as a red row). Takes `self` by value like
    /// [`finish`](Self::finish) and sets the same `finished` flag the `Drop`
    /// impl below relies on to detect abandoned handles.
    pub fn finish_skipped(mut self, reason: &str) {
        self.finished = true;
        self.registry.complete_skipped(self.id, reason.to_string());
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        if !self.finished {
            self.registry
                .complete(self.id, Some("Task ended without completing".to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Arc<LlmTaskRegistry> {
        Arc::new(LlmTaskRegistry::new())
    }

    #[test]
    fn a_running_task_appears_in_the_view() {
        let r = registry();
        let _task = r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1");
        let view = r.view();
        assert_eq!(view.running.len(), 1);
        assert_eq!(view.running[0].label, "Weekly 1:1");
        assert!(!view.has_failure);
    }

    #[test]
    fn a_successful_task_leaves_running_and_records_success() {
        let r = registry();
        r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1")
            .finish(Ok(()));
        let view = r.view();
        assert!(view.running.is_empty());
        assert_eq!(view.history.len(), 1);
        assert!(view.history[0].error.is_none());
        assert!(!view.has_failure, "success must not raise the sticky badge");
    }

    #[test]
    fn a_failed_task_raises_the_sticky_badge() {
        let r = registry();
        r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1")
            .finish(Err("LLM request timed out after 300 seconds".into()));
        let view = r.view();
        assert!(view.has_failure);
        assert_eq!(
            view.history[0].error.as_deref(),
            Some("LLM request timed out after 300 seconds")
        );
    }

    #[test]
    fn dismiss_clears_the_badge_but_keeps_history() {
        let r = registry();
        r.start(TaskKind::PrepBrief, Origin::Background, "x")
            .finish(Err("boom".into()));
        r.dismiss();
        let view = r.view();
        assert!(!view.has_failure);
        assert_eq!(
            view.history.len(),
            1,
            "dismiss acknowledges, it does not erase"
        );
    }

    /// A job that panics or returns early must not leave a phantom "running" task.
    #[test]
    fn dropping_a_handle_without_finishing_records_a_failure() {
        let r = registry();
        drop(r.start(TaskKind::PrepBrief, Origin::Background, "abandoned"));
        let view = r.view();
        assert!(
            view.running.is_empty(),
            "the task must not still look running"
        );
        assert!(view.has_failure);
        assert_eq!(
            view.history[0].error.as_deref(),
            Some("Task ended without completing")
        );
    }

    #[test]
    fn history_is_capped_and_keeps_the_newest() {
        let r = registry();
        for i in 0..(HISTORY_CAP + 5) {
            r.start(TaskKind::PrepBrief, Origin::Background, format!("task-{i}"))
                .finish(Ok(()));
        }
        let view = r.view();
        assert_eq!(view.history.len(), HISTORY_CAP);
        assert_eq!(view.history[0].label, format!("task-{}", HISTORY_CAP + 4));
    }

    #[test]
    fn foreground_tasks_are_recorded_but_excluded_from_the_background_view() {
        let r = registry();
        let _fg = r.start(TaskKind::AskAI, Origin::Foreground, "Ask AI");
        let _bg = r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1");
        let view = r.view();
        assert_eq!(view.running.len(), 1, "only background tasks surface");
        assert_eq!(view.running[0].label, "Weekly 1:1");
        assert_eq!(
            r.all_running_count(),
            2,
            "but foreground work is still tracked"
        );
    }

    #[test]
    fn progress_updates_the_running_note() {
        let r = registry();
        let task = r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1");
        task.progress("reading meeting 1 of 2");
        assert_eq!(
            r.view().running[0].note.as_deref(),
            Some("reading meeting 1 of 2")
        );
    }

    /// A failed FOREGROUND task must not raise the badge — the sidebar only speaks for
    /// background work, and foreground failures already surface in their own UI.
    #[test]
    fn a_failed_foreground_task_does_not_raise_the_badge() {
        let r = registry();
        r.start(TaskKind::AskAI, Origin::Foreground, "Ask AI")
            .finish(Err("boom".into()));
        assert!(!r.view().has_failure);
    }

    /// specs/0053 W3: a skip must leave running and land in history, but — unlike a
    /// failure — it must never raise the sticky badge, since a skip is healthy behaviour,
    /// not breakage.
    #[test]
    fn a_skipped_task_leaves_running_and_does_not_raise_the_badge() {
        let r = registry();
        r.start(TaskKind::ActionItems, Origin::Background, "Weekly 1:1")
            .finish_skipped("No commitments found in this meeting");
        let view = r.view();
        assert!(view.running.is_empty());
        assert_eq!(view.history.len(), 1);
        assert!(
            !view.has_failure,
            "a deliberate skip must not raise the sticky badge"
        );
        assert!(
            view.history[0].error.is_none(),
            "a skip is not an error"
        );
        match &view.history[0].outcome {
            TaskOutcome::Skipped { reason } => {
                assert_eq!(reason, "No commitments found in this meeting")
            }
            other => panic!("expected Skipped outcome, got {other:?}"),
        }
    }

    /// The success and failure paths still record the matching [`TaskOutcome`] variant,
    /// so a frontend consumer keyed on `outcome.type` (rather than `error`) also renders
    /// correctly.
    #[test]
    fn outcome_matches_success_and_failure() {
        let r = registry();
        r.start(TaskKind::PrepBrief, Origin::Background, "ok")
            .finish(Ok(()));
        r.start(TaskKind::PrepBrief, Origin::Background, "bad")
            .finish(Err("boom".into()));
        let view = r.view();
        assert!(matches!(view.history[0].outcome, TaskOutcome::Failed { .. }));
        assert!(matches!(view.history[1].outcome, TaskOutcome::Success));
    }

    #[test]
    fn take_record_removes_it_from_history_and_returns_it() {
        let reg = registry();
        let t = Arc::clone(&reg).start_for(
            TaskKind::PrepBrief,
            Origin::Background,
            "Prep — Pricing sync",
            Some("m1".into()),
        );
        t.finish(Err("provider timed out".into()));
        let id = reg.view().history[0].id;

        let taken = reg.take_record(id).expect("record should exist");
        assert_eq!(taken.kind, TaskKind::PrepBrief);
        assert_eq!(taken.meeting_id.as_deref(), Some("m1"));
        assert!(reg.view().history.is_empty(), "the record must leave history");
    }

    #[test]
    fn taking_the_last_failure_clears_the_sticky_badge() {
        let reg = registry();
        let t = Arc::clone(&reg).start_for(TaskKind::ActionItems, Origin::Background, "x", Some("m1".into()));
        t.finish(Err("boom".into()));
        assert!(reg.view().has_failure);

        let id = reg.view().history[0].id;
        reg.take_record(id);
        assert!(!reg.view().has_failure, "no failures left, so the badge must clear");
    }

    #[test]
    fn taking_one_of_two_failures_keeps_the_badge_lit() {
        let reg = registry();
        for m in ["m1", "m2"] {
            let t = Arc::clone(&reg).start_for(TaskKind::PrepBrief, Origin::Background, "x", Some(m.into()));
            t.finish(Err("boom".into()));
        }
        let id = reg.view().history[0].id;
        reg.take_record(id);
        assert!(reg.view().has_failure, "one failure remains, so the badge stays");
    }

    /// The per-record dismiss path (`api_llm_activity_dismiss_task`, fix-round 1 on
    /// specs/0063 W3 Task 6) is `take_record` under a different name — so a non-retryable
    /// kind (which the frontend renders as Dismiss, never Retry) must remove only its own
    /// record and leave an unrelated failure's badge lit, same as any other `take_record`.
    #[test]
    fn taking_a_non_retryable_kinds_record_leaves_another_failure_lit() {
        let reg = registry();
        let ask_ai = Arc::clone(&reg).start_for(TaskKind::AskAI, Origin::Background, "Ask AI", Some("m1".into()));
        ask_ai.finish(Err("boom".into()));
        let prep = Arc::clone(&reg).start_for(TaskKind::PrepBrief, Origin::Background, "Prep", Some("m2".into()));
        prep.finish(Err("boom".into()));

        let ask_ai_id = reg
            .view()
            .history
            .iter()
            .find(|r| r.kind == TaskKind::AskAI)
            .expect("the AskAI record")
            .id;

        reg.take_record(ask_ai_id);

        let remaining = reg.view();
        assert_eq!(remaining.history.len(), 1);
        assert_eq!(remaining.history[0].kind, TaskKind::PrepBrief);
        assert!(remaining.has_failure, "the other failure's badge must stay lit");
    }

    #[test]
    fn taking_an_unknown_id_is_a_no_op() {
        let reg = registry();
        assert!(reg.take_record(4242).is_none());
    }

    #[test]
    fn a_skipped_record_never_raises_the_badge_and_can_still_be_taken() {
        let reg = registry();
        let t = Arc::clone(&reg).start_for(TaskKind::ActionItems, Origin::Background, "x", None);
        t.finish_skipped("nothing to extract");
        assert!(!reg.view().has_failure);
        let id = reg.view().history[0].id;
        assert!(reg.take_record(id).is_some());
        assert!(!reg.view().has_failure);
    }

    /// A `Skipped` record left behind in history must never keep the badge lit — only
    /// `TaskOutcome::Failed` counts. This is the case the predicate could get wrong: an
    /// empty history recomputes `has_failure` to `false` under any reasonable predicate,
    /// so this test seeds a `Skipped` record that SURVIVES the take, alongside the
    /// `Failed` one that gets taken, so a predicate that (incorrectly) also counted
    /// `Skipped` would leave the badge lit and fail this test.
    #[test]
    fn a_remaining_skipped_record_does_not_keep_the_badge_lit() {
        let reg = registry();
        let skipped =
            Arc::clone(&reg).start_for(TaskKind::ActionItems, Origin::Background, "skip", None);
        skipped.finish_skipped("nothing to extract");
        let failed = Arc::clone(&reg).start_for(
            TaskKind::PrepBrief,
            Origin::Background,
            "prep",
            Some("m1".into()),
        );
        failed.finish(Err("timed out".into()));
        assert!(reg.view().has_failure);

        let failed_id = reg
            .view()
            .history
            .iter()
            .find(|r| matches!(r.outcome, TaskOutcome::Failed { .. }))
            .expect("the failed record")
            .id;
        reg.take_record(failed_id);

        let v = reg.view();
        assert_eq!(v.history.len(), 1, "the skipped record stays in history");
        assert!(!v.has_failure, "a skipped record must never keep the badge lit");
    }

    /// specs/0063 W3 fix round (I3): a FOREGROUND failure (e.g. Ask AI, already shown
    /// inline) must never re-light the badge when it is the only `Failed` record left
    /// after a background failure is taken. Before this fix, `take_record`'s recompute
    /// counted any `Failed` record regardless of origin, so retrying or dismissing the
    /// background failure below would leave `has_failure` incorrectly `true`.
    #[test]
    fn taking_the_only_background_failure_ignores_a_remaining_foreground_failure() {
        let reg = registry();
        Arc::clone(&reg)
            .start_for(TaskKind::AskAI, Origin::Foreground, "Ask AI", Some("m1".into()))
            .finish(Err("boom".into()));
        assert!(
            !reg.view().has_failure,
            "a foreground failure alone must not raise the badge"
        );

        let background = Arc::clone(&reg).start_for(
            TaskKind::MeetingSummary,
            Origin::Background,
            "Summary — Weekly 1:1",
            Some("m2".into()),
        );
        background.finish(Err("save failed".into()));
        assert!(reg.view().has_failure);

        let background_id = reg
            .view()
            .history
            .iter()
            .find(|r| r.kind == TaskKind::MeetingSummary)
            .expect("the background record")
            .id;
        reg.take_record(background_id);

        let v = reg.view();
        assert_eq!(
            v.history.len(),
            1,
            "the untouched foreground failure stays in history"
        );
        assert!(
            !v.has_failure,
            "the remaining record is foreground-only, so the badge must clear"
        );
    }
}
