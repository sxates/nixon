//! Retry a failed background task by its registry id, dispatched by kind (specs/0063 W3).
//!
//! A footer-queue row only knows its task id — not what kind of work produced it — so this
//! module resolves the kind and re-dispatches accordingly. It is the only retry path: a
//! prep-brief-only command addressed by meeting id was removed once this landed, since it
//! had no caller and its behaviour is reproduced exactly by the `PrepBrief` arm below.

use tauri::{AppHandle, Manager, Runtime};

use crate::action_items::commands::{load_extraction_inputs, ExtractionInputs};
use crate::database::repositories::meeting_brief::MeetingBriefsRepository;
use crate::llm_activity::registry::TaskKind;
use crate::llm_activity::LlmActivityState;
use crate::state::AppState;
use crate::summary::llm_gate::{with_priority, Priority};

/// Whether a task of this kind has a retry dispatch below. Mirrored in the frontend
/// (specs/0063 W3 Task 5) to decide whether a queue row offers Retry or only Dismiss.
pub fn is_retryable(kind: TaskKind) -> bool {
    matches!(
        kind,
        TaskKind::PrepBrief
            | TaskKind::MeetingSummary
            | TaskKind::ActionItems
            | TaskKind::Diarization
    )
}

/// Retry the task recorded at `task_id`, dispatched by its kind.
///
/// Checks retryability BEFORE touching the registry, and only calls `take_record` once a
/// dispatch below is actually going to run: a non-retryable kind (or an id already gone)
/// must leave the history record exactly where it was, with a clean error back to the
/// caller — not destroy it and THEN report the error, which would strand a "failed" row
/// the user can no longer even see, let alone retry or dismiss (a reviewer flagged this
/// ordering as latent before a real per-record dismiss existed; specs/0063 W3 Task 6 made
/// it load-bearing). Once retry is confirmed possible, the record is taken out before
/// dispatching — the row must clear immediately, and if the retry itself fails it
/// registers a fresh failure of its own, which is correct behaviour rather than mutating
/// the old record. A double-clicked Retry finds nothing on the second call and returns a
/// clean error, never panics or double-runs.
pub async fn retry_task<R: Runtime>(app: &AppHandle<R>, task_id: u64) -> Result<(), String> {
    let registry = app
        .try_state::<LlmActivityState>()
        .ok_or_else(|| "Background task tracking is unavailable".to_string())?
        .0
        .clone();

    let kind = registry
        .peek_kind(task_id)
        .ok_or_else(|| "That task is no longer in the queue.".to_string())?;

    if !is_retryable(kind) {
        return Err(format!("{:?} tasks can't be retried", kind));
    }

    // specs/0063 W3 fix round (M4): every dispatch below addresses a meeting, so this must
    // be checked BEFORE `take_record` — same reasoning as the retryable-kind check above.
    // Checking it after would destroy the record for a kind that (today, unreachably from
    // the UI) has no meeting id, then report an error for a row the user can no longer see,
    // retry, or dismiss.
    let meeting_id = registry
        .peek_meeting_id(task_id)
        .ok_or_else(|| "That task is no longer in the queue.".to_string())?
        .ok_or_else(|| "That task has no meeting to retry against.".to_string())?;

    let record = registry
        .take_record(task_id)
        .ok_or_else(|| "That task is no longer in the queue.".to_string())?;

    match record.kind {
        TaskKind::PrepBrief => {
            let pool = app
                .try_state::<AppState>()
                .ok_or_else(|| "App state unavailable".to_string())?
                .db_manager
                .pool()
                .clone();
            MeetingBriefsRepository::reset_failures(&pool, &meeting_id)
                .await
                .map_err(|e| format!("Could not clear the failure count: {e}"))?;
            crate::aggregation::prep_jobs::run_prep_pass(app).await;
            Ok(())
        }
        TaskKind::MeetingSummary => {
            let pool = app
                .try_state::<AppState>()
                .ok_or_else(|| "App state unavailable".to_string())?
                .db_manager
                .pool()
                .clone();
            crate::summary::commands::start_summary_generation_for_meeting(
                app.clone(),
                pool,
                meeting_id,
            )
            .await
            .map(|_| ())
        }
        TaskKind::ActionItems => {
            let pool = app
                .try_state::<AppState>()
                .ok_or_else(|| "App state unavailable".to_string())?
                .db_manager
                .pool()
                .clone();
            let ExtractionInputs {
                summary_markdown,
                user_notes,
                provider_config,
            } = load_extraction_inputs(&pool, &meeting_id).await?;

            // specs/0063 W3: a queue Retry is a click the user is waiting on, same
            // posture as "Scan again" (Interactive), not background drain.
            with_priority(
                Priority::Interactive,
                crate::action_items::run_extraction(
                    app,
                    &pool,
                    &meeting_id,
                    &summary_markdown,
                    user_notes.as_deref(),
                    &provider_config,
                ),
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:#}"))
        }
        TaskKind::Diarization => {
            // specs/0066 W2: `diarize_meeting` returns false when a pass is ALREADY running
            // for this meeting (`launch.rs` registry guard). The first version of this arm
            // discarded that bool and reported `Ok(())` — and because `take_record` has
            // already run by here, the row and its lamp were gone too. The user was left
            // with a failure they could no longer see and a retry that never ran: the
            // literal "clicking Retry does nothing" report. Refusal is an error now.
            if crate::diarization::launch::diarize_meeting(app.clone(), meeting_id) {
                Ok(())
            } else {
                Err("Speaker identification is already running for this meeting.".to_string())
            }
        }
        TaskKind::AskAI | TaskKind::Rollup | TaskKind::NoteEnhancement => {
            unreachable!("non-retryable kinds are rejected by is_retryable above")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_kinds_are_exactly_the_ones_with_a_dispatch() {
        assert!(is_retryable(TaskKind::PrepBrief));
        assert!(is_retryable(TaskKind::MeetingSummary));
        assert!(is_retryable(TaskKind::ActionItems));
        assert!(is_retryable(TaskKind::Diarization));
        assert!(!is_retryable(TaskKind::AskAI));
        assert!(!is_retryable(TaskKind::Rollup));
        assert!(!is_retryable(TaskKind::NoteEnhancement));
    }

    /// Registry-level guarantee that `retry_task` depends on: the record is consumed
    /// (and the sticky badge recomputed) before any dispatch happens, so the row and
    /// lamp clear immediately regardless of how the retry itself turns out.
    #[test]
    fn retrying_takes_the_record_out_so_the_row_and_lamp_clear() {
        use crate::llm_activity::registry::{LlmTaskRegistry, Origin};
        use std::sync::Arc;
        let reg = Arc::new(LlmTaskRegistry::new());
        let t = Arc::clone(&reg).start_for(
            TaskKind::PrepBrief,
            Origin::Background,
            "Prep",
            Some("m1".into()),
        );
        t.finish(Err("timed out".into()));
        let id = reg.view().history[0].id;

        let record = reg.take_record(id).expect("present");
        assert!(is_retryable(record.kind));
        assert_eq!(record.meeting_id.as_deref(), Some("m1"));
        assert!(reg.view().history.is_empty());
        assert!(!reg.view().has_failure);
    }

    /// The reorder's point (fix-round 1, specs/0063 W3 Task 6): `retry_task` must check
    /// retryability BEFORE taking the record out, so a refused retry leaves the row exactly
    /// where it was — retryable via a different path, or dismissable — instead of destroying
    /// it and then reporting an error about a row the user can no longer even see.
    #[tokio::test]
    async fn retry_task_on_a_non_retryable_kind_leaves_the_record_and_errors() {
        use crate::llm_activity::registry::{LlmTaskRegistry, Origin};
        use crate::llm_activity::LlmActivityState;
        use std::sync::Arc;

        let reg = Arc::new(LlmTaskRegistry::new());
        let t = Arc::clone(&reg).start_for(
            TaskKind::AskAI,
            Origin::Background,
            "Ask AI",
            Some("m1".into()),
        );
        t.finish(Err("boom".into()));
        let id = reg.view().history[0].id;

        let app = tauri::test::mock_app();
        app.handle().manage(LlmActivityState(Arc::clone(&reg)));

        let result = retry_task(app.handle(), id).await;

        assert!(result.is_err());
        assert_eq!(
            reg.view().history.len(),
            1,
            "a refused retry must not destroy the record it refused to touch"
        );
        assert_eq!(reg.view().history[0].id, id);
    }

    /// specs/0063 W3 fix round (M4): a retryable KIND with no meeting id (unreachable from
    /// the UI today, but not impossible) must also be checked before `take_record` — same
    /// guarantee as the non-retryable-kind case above, for the other precondition.
    #[tokio::test]
    async fn retry_task_on_a_retryable_kind_with_no_meeting_id_leaves_the_record_and_errors() {
        use crate::llm_activity::registry::{LlmTaskRegistry, Origin};
        use crate::llm_activity::LlmActivityState;
        use std::sync::Arc;

        let reg = Arc::new(LlmTaskRegistry::new());
        let t = Arc::clone(&reg).start_for(TaskKind::PrepBrief, Origin::Background, "Prep", None);
        t.finish(Err("boom".into()));
        let id = reg.view().history[0].id;

        let app = tauri::test::mock_app();
        app.handle().manage(LlmActivityState(Arc::clone(&reg)));

        let result = retry_task(app.handle(), id).await;

        assert!(result.is_err());
        assert_eq!(
            reg.view().history.len(),
            1,
            "a refused retry must not destroy the record it refused to touch"
        );
        assert_eq!(reg.view().history[0].id, id);
    }
}
