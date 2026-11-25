//! Tauri commands for backup and restore (specs/0092, "Tauri IPC"). Registered in `registry.rs`.
//!
//! Every job takes the one lock in [`super::job`]. Export, extraction and merge do synchronous
//! file I/O inside async fns, so each job body runs on a blocking thread (`blocking`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Local;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use super::add::import_add;
use super::archive::read_manifest;
use super::export::{default_file_name, export_to_file, ExportOptions};
use super::job::{self, blocking, EmitProgress, Finished, JobGuard, JobKind, JobStatus, EVENT_FINISHED};
use super::manifest::{build_schema_version, validate};
use super::merge::MergeCounts;
use super::replace::{stage, RESULT_NAME};
use super::schedule::{run_once, RunResult};
use super::settings::{self, BackupSettings};
use super::BackupError;
use crate::state::AppState;

fn emit_finished<R: Runtime>(app: &AppHandle<R>, kind: JobKind, result: &Result<(), BackupError>, path: Option<String>) {
    let payload = match result {
        Ok(()) => Finished::ok(kind, path),
        Err(e) => Finished::err(kind, e),
    };
    let _ = app.emit(EVENT_FINISHED, payload);
}

#[tauri::command]
pub async fn api_backup_export<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    include_audio: bool,
) -> Result<Option<String>, BackupError> {
    let dialog_app = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .set_file_name(default_file_name())
            .add_filter("Zip", &["zip"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| BackupError::Io(format!("save dialog failed: {e}")))?;
    let Some(picked) = picked else { return Ok(None) };
    let dest = picked
        .into_path()
        .map_err(|e| BackupError::Io(format!("bad destination: {e}")))?;

    let guard = job::try_begin(JobKind::Export)?;
    let pool = state.db_manager.pool().clone();
    let progress = EmitProgress::new(app.clone());
    let token = guard.token().clone();
    let opts = ExportOptions {
        include_audio,
        app_data_dir: crate::app_paths::app_data_dir(),
        abort_if: None,
        allowed_roots: crate::audio::recording_preferences::known_recording_roots(),
    };
    let result = blocking(async move {
        let summary = export_to_file(&pool, &dest, &opts, &progress, &token).await?;
        Ok(summary.path.to_string_lossy().into_owned())
    })
    .await;
    drop(guard);
    emit_finished(&app, JobKind::Export, &result.as_ref().map(|_| ()).map_err(Clone::clone), result.as_ref().ok().cloned());
    result.map(Some)
}

#[tauri::command]
pub fn api_backup_cancel() -> bool {
    job::cancel_current()
}

#[tauri::command]
pub fn api_backup_status() -> Option<JobStatus> {
    job::status()
}

#[tauri::command]
pub async fn api_backup_pick_file<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, BackupError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Nixon backup", &["zip"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| BackupError::Io(format!("file picker failed: {e}")))?;
    Ok(picked.map(|p| p.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCounts {
    pub meetings: i64,
    pub people: i64,
    pub topics: i64,
    pub action_items: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub created_at: String,
    pub app_version: String,
    pub includes_audio: bool,
    pub counts: PreviewCounts,
    pub bytes: u64,
    /// Meetings already on this Mac, so the UI can pick the default import mode.
    pub local_meetings: u32,
}

#[tauri::command]
pub async fn api_backup_preview(
    state: State<'_, AppState>,
    path: String,
) -> Result<BackupPreview, BackupError> {
    let zip = PathBuf::from(path);
    let (manifest, bytes) = {
        let zip = zip.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<_, BackupError> {
            let manifest = read_manifest(&zip)?;
            validate(&manifest, build_schema_version())?;
            Ok((manifest, std::fs::metadata(&zip)?.len()))
        })
        .await
        .map_err(|e| BackupError::Io(format!("preview failed: {e}")))??
    };
    let local: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings")
        .fetch_one(state.db_manager.pool())
        .await?;
    Ok(BackupPreview {
        created_at: manifest.created_at,
        app_version: manifest.app_version,
        includes_audio: manifest.includes_audio,
        counts: PreviewCounts {
            meetings: manifest.counts.meetings,
            people: manifest.counts.people,
            topics: manifest.counts.topics,
            action_items: manifest.counts.action_items,
        },
        bytes,
        local_meetings: local.max(0) as u32,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportMode {
    Replace,
    Add,
}

/// Add returns the counts. Replace stages, closes the database and restarts (never returns on
/// success).
#[tauri::command]
pub async fn api_backup_import<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    path: String,
    mode: ImportMode,
) -> Result<Option<MergeCounts>, BackupError> {
    let guard = job::try_begin(JobKind::Import)?;
    // Checked after the lock is taken (nothing else can start a job) and again after staging:
    // a recording that began meanwhile must not be cut by the restart.
    if mode == ImportMode::Replace && crate::audio::recording_commands::is_recording().await {
        return Err(BackupError::RecordingActive);
    }
    let pool = state.db_manager.pool().clone();
    let zip = PathBuf::from(&path);
    let app_data = crate::app_paths::app_data_dir();
    let recordings_root = crate::audio::recording_preferences::recordings_root();
    let progress = EmitProgress::new(app.clone());
    let token = guard.token().clone();

    match mode {
        ImportMode::Add => {
            let result = blocking(async move {
                import_add(&pool, &zip, &app_data, &recordings_root, &progress, &token).await
            })
            .await;
            drop(guard);
            emit_finished(&app, JobKind::Import, &result.as_ref().map(|_| ()).map_err(Clone::clone), Some(path));
            result.map(Some)
        }
        ImportMode::Replace => {
            let result = blocking(async move {
                stage(&pool, &zip, &app_data, &recordings_root, &progress, &token).await
            })
            .await;
            if let Err(e) = result {
                drop(guard);
                emit_finished(&app, JobKind::Import, &Err(e.clone()), Some(path));
                return Err(e);
            }
            if crate::audio::recording_commands::is_recording().await {
                super::replace::cancel_pending(&crate::app_paths::app_data_dir());
                drop(guard);
                let e = BackupError::RecordingActive;
                emit_finished(&app, JobKind::Import, &Err(e.clone()), Some(path));
                return Err(e);
            }
            restart_into_restore(&app, &state, guard).await
        }
    }
}

/// Checkpoint and close the database, save the window state, relaunch. The restore is applied
/// at the next launch before the database opens.
async fn restart_into_restore<R: Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, AppState>,
    guard: JobGuard,
) -> Result<Option<MergeCounts>, BackupError> {
    if let Err(e) = state.db_manager.cleanup().await {
        // The marker is written; the apply step opens the database file directly, so a failed
        // checkpoint is logged, not fatal.
        log::warn!("backup restore: database cleanup before restart failed: {e}");
    }
    crate::window_state::save(app);
    drop(guard);
    app.restart()
}

#[tauri::command]
pub fn api_backup_get_settings() -> BackupSettings {
    settings::load(&crate::app_paths::app_data_dir())
}

#[tauri::command]
pub fn api_backup_set_settings(
    enabled: bool,
    folder: Option<String>,
    include_audio: bool,
) -> Result<BackupSettings, BackupError> {
    let dir = crate::app_paths::app_data_dir();
    settings::update(&dir, |s| s.apply_ui(enabled, folder, include_audio))
}

#[tauri::command]
pub async fn api_backup_pick_folder<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, BackupError> {
    let picked = tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| BackupError::Io(format!("folder picker failed: {e}")))?;
    Ok(picked.map(|p| p.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunOutcome {
    Written,
    Skipped,
    NotDue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResultDto {
    pub result: RunOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[tauri::command]
pub async fn api_backup_run_now(state: State<'_, AppState>) -> Result<RunResultDto, BackupError> {
    let guard = job::try_begin(JobKind::Scheduled)?;
    let pool = state.db_manager.pool().clone();
    let app_data = crate::app_paths::app_data_dir();
    let roots = crate::audio::recording_preferences::known_recording_roots();
    let result = blocking(async move {
        run_once(
            &pool,
            &app_data,
            Local::now(),
            Arc::new(crate::audio::recording_commands::is_recording_now),
            true,
            roots,
        )
        .await
    })
    .await;
    drop(guard);
    Ok(match result? {
        RunResult::Written(sum) => RunResultDto {
            result: RunOutcome::Written,
            path: Some(sum.path.to_string_lossy().into_owned()),
            bytes: Some(sum.bytes),
        },
        RunResult::Skipped => RunResultDto { result: RunOutcome::Skipped, path: None, bytes: None },
        RunResult::NotDue => RunResultDto { result: RunOutcome::NotDue, path: None, bytes: None },
    })
}

/// What `restore-result.json` holds, as the UI wants it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub ok: bool,
    pub meetings: u32,
    #[serde(alias = "created_at")]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn take_restore_result(app_data_dir: &Path) -> Option<RestoreResult> {
    let file = app_data_dir.join(RESULT_NAME);
    let bytes = std::fs::read(&file).ok()?;
    // Read once: the toast must not come back on the next launch, even if the file is junk.
    let _ = std::fs::remove_file(&file);
    serde_json::from_slice(&bytes).ok()
}

#[tauri::command]
pub fn api_backup_last_restore_result() -> Option<RestoreResult> {
    take_restore_result(&crate::app_paths::app_data_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_result_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(RESULT_NAME),
            br#"{"ok":false,"meetings":3,"created_at":"2026-11-24T10:00:00Z","error":"boom"}"#,
        )
        .unwrap();
        let r = take_restore_result(dir.path()).expect("result");
        assert_eq!(
            r,
            RestoreResult {
                ok: false,
                meetings: 3,
                created_at: "2026-11-24T10:00:00Z".into(),
                error: Some("boom".into()),
            }
        );
        assert!(take_restore_result(dir.path()).is_none());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["createdAt"], "2026-11-24T10:00:00Z");
    }

    #[test]
    fn dto_shapes_are_camel_case() {
        let dto = RunResultDto { result: RunOutcome::NotDue, path: None, bytes: None };
        assert_eq!(serde_json::to_string(&dto).unwrap(), r#"{"result":"notDue"}"#);
        let m: ImportMode = serde_json::from_str(r#""add""#).unwrap();
        assert_eq!(m, ImportMode::Add);
    }
}
