//! IPC for the LLM activity indicator (specs/0052).

use crate::database::repositories::meeting_brief::MeetingBriefsRepository;
use crate::llm_activity::{LlmActivityState, LlmActivityView};
use crate::state::AppState;
use tauri::{AppHandle, Emitter, Manager, State};

/// Emitted on every task transition with the full view. The frontend also pulls a snapshot
/// on mount, so a missed event self-heals on the next transition.
pub const LLM_ACTIVITY_EVENT: &str = "llm-activity-changed";

/// Emit the current view. Best-effort: a failed emit must never break generation.
pub fn emit_activity(app: &AppHandle, view: &LlmActivityView) {
    let _ = app.emit(LLM_ACTIVITY_EVENT, view);
}

/// Current activity — pulled once on mount so the indicator is correct even if the frontend
/// mounted after a transition it never saw.
#[tauri::command]
pub async fn api_llm_activity_snapshot(
    state: State<'_, LlmActivityState>,
) -> Result<LlmActivityView, String> {
    Ok(state.0.view())
}

/// Acknowledge the sticky failure badge. History is retained — dismissing says "I've seen
/// it", not "forget it happened".
#[tauri::command]
pub async fn api_llm_activity_dismiss(
    app: AppHandle,
    state: State<'_, LlmActivityState>,
) -> Result<(), String> {
    state.0.dismiss();
    emit_activity(&app, &state.0.view());
    Ok(())
}

/// Clear a prep brief's failure counter and force one regeneration pass.
///
/// Retry is prep-brief-only: prep is the one background task with a durable, addressable
/// artifact (a `meeting_briefs` row keyed by meeting) and a give-up rule to clear.
#[tauri::command]
pub async fn api_llm_activity_retry(app: AppHandle, meeting_id: String) -> Result<(), String> {
    let pool = app
        .try_state::<AppState>()
        .ok_or_else(|| "App state unavailable".to_string())?
        .db_manager
        .pool()
        .clone();

    MeetingBriefsRepository::reset_failures(&pool, &meeting_id)
        .await
        .map_err(|e| format!("Could not clear the failure count: {e}"))?;

    crate::aggregation::prep_jobs::run_prep_pass(&app).await;
    Ok(())
}

/// Dismiss ONE failed background task, addressed by its registry id (specs/0063 W3 Task 6).
///
/// Distinct from [`api_llm_activity_dismiss`], which acknowledges the WHOLE failure history
/// at once and keeps every record. This one calls `LlmTaskRegistry::take_record`, which
/// removes exactly that record and recomputes `has_failure` from what remains — so
/// dismissing one queue row never makes an unrelated row's failure vanish too. A `task_id`
/// that is already gone (a double-click, a race with Retry) is not an error: the row the
/// user was looking at is already gone from the queue, which is what they wanted.
#[tauri::command]
pub async fn api_llm_activity_dismiss_task(
    app: AppHandle,
    state: State<'_, LlmActivityState>,
    task_id: u64,
) -> Result<(), String> {
    state.0.take_record(task_id);
    emit_activity(&app, &state.0.view());
    Ok(())
}

/// Retry one failed background task, addressed by its registry id (specs/0063 W3).
///
/// Distinct from [`api_llm_activity_retry`], which is prep-brief-only and addressed by
/// meeting. The queue row knows its task id, not what kind of work produced it, so the
/// dispatch happens in [`crate::llm_activity::retry`].
#[tauri::command]
pub async fn api_llm_activity_retry_task(app: AppHandle, task_id: u64) -> Result<(), String> {
    crate::llm_activity::retry::retry_task(&app, task_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend listens on this exact string; changing it silently breaks the indicator.
    #[test]
    fn the_event_name_is_stable() {
        assert_eq!(LLM_ACTIVITY_EVENT, "llm-activity-changed");
    }
}
