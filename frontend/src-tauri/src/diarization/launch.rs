//! Diarization run lifecycle: claim the per-meeting run slot, spawn the pass,
//! report it on the background-activity queue, and emit the terminal event.
//!
//! This is split out of [`crate::diarization::pipeline`] because the run
//! lifecycle (slot guard, spawn, queue reporting, terminal emit) is a separable
//! concern from the pass itself (model load, decode, cluster, persist) — and
//! `pipeline.rs` is size-capped (specs/0042 WS6 file-size ratchet) with no
//! headroom left for new code.

use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::pipeline::{registry_finish, registry_try_begin, run, EVENT_ERROR};

/// Run offline diarization for a saved meeting on a background task, emitting
/// `diarization-{progress,complete,error}`. Returns immediately after spawning.
///
/// WS3.1 (specs/0029): at most ONE live run per meeting. Returns `true` when a new
/// run was started, `false` when one is already in flight (the caller should attach
/// to the running pass — its progress arrives on the shared events — rather than
/// treat this as a failure).
///
/// Gating on the opt-in setting (and on models-present, for an auto-run) lives at
/// the call site (P1-C); this entry point always runs when called.
pub fn diarize_meeting<R: Runtime>(app: AppHandle<R>, meeting_id: String) -> bool {
    if !registry_try_begin(&meeting_id) {
        log::info!(
            "diarization already running for meeting {meeting_id}; not starting a second run"
        );
        return false;
    }
    tauri::async_runtime::spawn(async move {
        // specs/0063 W3 — the queue is where the user finds out what the machine is busy
        // with, and a diarization pass is the most CPU-hungry thing it does. Registered
        // inside the spawn so a refused duplicate run never creates a phantom row.
        let task = app
            .try_state::<crate::llm_activity::LlmActivityState>()
            .map(|state| std::sync::Arc::clone(&state.0))
            .map(|registry| {
                registry.start_for(
                    crate::llm_activity::registry::TaskKind::Diarization,
                    crate::llm_activity::registry::Origin::Background,
                    format!("Identifying speakers — {meeting_id}"),
                    Some(meeting_id.clone()),
                )
            });

        let outcome = run(app.clone(), meeting_id.clone()).await;
        if let Some(t) = task {
            t.finish(outcome.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
        }
        if let Err(e) = outcome {
            log::warn!("diarization failed for {meeting_id}: {e:#}");
            // Release the run slot BEFORE emitting so a status query racing the
            // event never sees a stale "running". (The success path does the same
            // inside `run`, just before its `diarization-complete` emit.)
            registry_finish(&meeting_id, "error", 0);
            let _ = app.emit(
                EVENT_ERROR,
                serde_json::json!({ "meeting_id": meeting_id, "error": format!("{e:#}") }),
            );
        }
    });
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_diarization_task_is_registered_as_background_work_for_the_meeting() {
        let reg = crate::llm_activity::registry::LlmTaskRegistry::new();
        let reg = std::sync::Arc::new(reg);
        let handle = std::sync::Arc::clone(&reg).start_for(
            crate::llm_activity::registry::TaskKind::Diarization,
            crate::llm_activity::registry::Origin::Background,
            "Identifying speakers — Pricing sync",
            Some("m1".to_string()),
        );
        let view = reg.view();
        assert_eq!(view.running.len(), 1);
        assert_eq!(
            view.running[0].kind,
            crate::llm_activity::registry::TaskKind::Diarization
        );
        assert_eq!(view.running[0].meeting_id.as_deref(), Some("m1"));
        handle.finish(Ok(()));
        assert!(reg.view().running.is_empty());
        assert!(!reg.view().has_failure);
    }
}
