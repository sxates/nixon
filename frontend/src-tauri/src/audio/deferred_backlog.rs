//! Deferred-backlog query (low-power-mode spec §5). The debounce/prompt/
//! processing loop lives in the frontend; this command answers "which
//! meetings still need processing and still have audio on disk?".

use crate::audio::constants::AUDIO_EXTENSIONS;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Manager, Runtime};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredMeeting {
    pub id: String,
    pub title: String,
    pub folder_path: String,
    pub transcript_count: i64,
}

/// True when the meeting folder still contains at least one media file.
fn folder_has_audio(folder: &Path) -> bool {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries.flatten().any(|e| {
                let p = e.path();
                p.is_file()
                    && p.extension()
                        .map(|ext| {
                            AUDIO_EXTENSIONS.contains(&ext.to_string_lossy().to_lowercase().as_str())
                        })
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// spec 0051 final review (Finding 2): `try_state`, not `state()`.
///
/// Task 5 hoisted `DeferredBacklogProvider` out of the `showOnboarding` ternary, so
/// `useDeferredBacklog`'s mount effect now fires ~5s after EVERY launch — including
/// throughout first-launch onboarding, when `AppState` is unmanaged for the whole
/// window (`database::setup::initialize_database_on_startup` only emits
/// `first-launch-detected`; `app.manage(AppState)` happens later, from a
/// frontend-triggered command). `state()` panics there, and a panicking command never
/// sends its IPC response — the frontend's `try/catch` never fires and the promise
/// hangs forever. The same window exists on a normal launch whenever DB init outruns
/// the 5s debounce.
///
/// Unmanaged ⇒ `Ok(vec![])`: no database means no meetings, so an empty backlog is the
/// truthful answer, and the frontend's next trigger (power change / mount after
/// onboarding reloads the window) retries. Mirrors the sibling fix in
/// `processing_reconcile::spawn_startup_reconciliation`.
#[tauri::command]
pub async fn api_list_deferred_meetings<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<DeferredMeeting>, String> {
    let Some(pool) = app
        .try_state::<AppState>()
        .map(|s| s.db_manager.pool().clone())
    else {
        log::debug!(
            "deferred-meeting list skipped: database not initialized yet (first-launch \
             onboarding window — no meetings can exist yet)"
        );
        return Ok(Vec::new());
    };
    let rows = MeetingsRepository::list_deferred_candidates(&pool)
        .await
        .map_err(|e| format!("Failed to list deferred meetings: {e}"))?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let folder = r.folder_path?;
            folder_has_audio(Path::new(&folder)).then_some(DeferredMeeting {
                id: r.id,
                title: r.title,
                folder_path: folder,
                transcript_count: r.transcript_count,
            })
        })
        .collect())
}
