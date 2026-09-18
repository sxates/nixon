//! Retry a failed background task by its registry id, dispatched by kind (specs/0063 W3).
//!
//! Distinct from [`crate::llm_activity::commands::api_llm_activity_retry`], which is
//! prep-brief-only and addressed by meeting id. A footer-queue row only knows its task
//! id — not what kind of work produced it — so this module resolves the kind and
//! re-dispatches accordingly.

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
/// Takes the record out of the registry BEFORE dispatching — the row must clear
/// immediately, and if the retry itself fails it registers a fresh failure of its own,
/// which is correct behaviour rather than mutating the old record. A double-clicked
/// Retry finds nothing on the second call and returns a clean error, never panics or
/// double-runs.
pub async fn retry_task<R: Runtime>(app: &AppHandle<R>, task_id: u64) -> Result<(), String> {
    let registry = app
        .try_state::<LlmActivityState>()
        .ok_or_else(|| "Background task tracking is unavailable".to_string())?
        .0
        .clone();

    let record = registry
        .take_record(task_id)
        .ok_or_else(|| "That task is no longer in the queue.".to_string())?;

    if !is_retryable(record.kind) {
        return Err(format!("{:?} tasks can't be retried", record.kind));
    }

    // Every dispatch below addresses a meeting; a record without one cannot be retried.
    let meeting_id = record
        .meeting_id
        .clone()
        .ok_or_else(|| "That task has no meeting to retry against.".to_string())?;

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
            crate::diarization::launch::diarize_meeting(app.clone(), meeting_id);
            Ok(())
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
}
