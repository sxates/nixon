//! Startup pre-pass for a crash between stop and save (specs/0083 W2a task 10).
//!
//! The save no longer waits on a webview, so the window is small, but a quit or crash can
//! still land between the folder being finalized and the transcript rows being written.
//! Such a meeting has a recording folder whose `metadata.json` says `completed`, ZERO
//! transcript rows, and a non-empty `transcripts.json`. Its transcript is imported through
//! the WS6.7-guarded attach (`save_transcripts_for_meeting`, which refuses a populated
//! meeting), so it can never duplicate rows. It starts no pass: the reconcile's
//! `list_unfinished` → `finish_processing` that runs next handles the meeting's audio.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::audio::recording_recovery::load_folder_transcript_segments;
use crate::database::repositories::{
    meeting::MeetingsRepository, transcript::TranscriptsRepository,
};

#[derive(Deserialize)]
struct FolderMeta {
    #[serde(default)]
    meeting_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Folders under `roots` that are `completed`, linked to one of `wanted`, and hold a
/// `transcripts.json`. Filesystem only.
fn completed_folders(roots: &[PathBuf], wanted: &HashSet<String>) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for folder in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            if !folder.join("transcripts.json").is_file() {
                continue;
            }
            let Some(meta) = std::fs::read(folder.join("metadata.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<FolderMeta>(&b).ok())
            else {
                continue;
            };
            let Some(id) = meta.meeting_id.filter(|id| wanted.contains(id)) else {
                continue;
            };
            if meta.status.as_deref() == Some("completed") && !out.iter().any(|(i, _)| *i == id) {
                out.push((id, folder));
            }
        }
    }
    out
}

/// Import every stranded session's transcript. Returns how many meetings got rows.
pub async fn import_unsaved_sessions(pool: &SqlitePool, roots: &[PathBuf]) -> usize {
    let empty: Vec<String> = match sqlx::query_scalar(
        "SELECT id FROM meetings m WHERE NOT EXISTS \
         (SELECT 1 FROM transcripts t WHERE t.meeting_id = m.id)",
    )
    .fetch_all(pool)
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            log::warn!("Wrap-up reconcile: couldn't list meetings without transcripts: {e}");
            return 0;
        }
    };
    if empty.is_empty() {
        return 0;
    }
    let wanted: HashSet<String> = empty.into_iter().collect();
    let roots = roots.to_vec();
    let found = tokio::task::spawn_blocking(move || completed_folders(&roots, &wanted))
        .await
        .unwrap_or_default();
    let mut imported = 0;
    for (id, folder) in found {
        if import_one(pool, &id, &folder).await {
            imported += 1;
        }
    }
    imported
}

async fn import_one(pool: &SqlitePool, meeting_id: &str, folder: &Path) -> bool {
    let segments = load_folder_transcript_segments(folder);
    if segments.is_empty() {
        return false;
    }
    let title = match MeetingsRepository::get_meeting_metadata(pool, meeting_id).await {
        Ok(Some(m)) => m.title,
        _ => return false,
    };
    let folder_str = folder.to_string_lossy().into_owned();
    match TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        meeting_id,
        &title,
        &segments,
        Some(folder_str),
    )
    .await
    {
        Ok(true) => {
            log::info!(
                "Wrap-up reconcile: imported {} unsaved segment(s) into meeting {meeting_id}",
                segments.len()
            );
            true
        }
        Ok(false) => false,
        Err(e) => {
            log::warn!("Wrap-up reconcile: couldn't import {meeting_id}: {e}");
            false
        }
    }
}

/// The launch pre-pass: after 0084's crash-loop breaker, before `list_unfinished`.
pub async fn startup_pre_pass<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    let roots = crate::audio::recording_preferences::known_recording_roots();
    import_unsaved_sessions(state.db_manager.pool(), &roots).await;
}

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod tests;
