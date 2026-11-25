//! Tauri commands for backup and restore (specs/0092, "Tauri IPC"). Registered in `registry.rs`.
//!
//! Every job takes the one lock in [`super::job`]. Export, extraction and merge do synchronous
//! file I/O inside async fns, so each job body runs on a blocking thread (`blocking`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use super::add::import_add;
use super::archive::read_manifest;
use super::export::{export_to, ExportOptions};
use super::job::{self, blocking, EmitProgress, Finished, JobGuard, JobKind, JobStatus, EVENT_FINISHED};
use super::library::{self, BackupFile};
use super::manifest::{build_schema_version, validate};
use super::merge::MergeCounts;
use super::replace::{stage, RESULT_NAME};
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

/// The folder backups go to: the one chosen in Settings, else the recordings folder.
fn backup_folder() -> PathBuf {
    settings::load(&crate::app_paths::app_data_dir())
        .folder_or(&crate::audio::recording_preferences::recordings_root())
}

/// "Back up now": a backup made by hand, written into the backup folder (kept until deleted),
/// with or without audio per the Include audio switch. Returns the file's path.
#[tauri::command]
pub async fn api_backup_export<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<Option<String>, BackupError> {
    let guard = job::try_begin(JobKind::Export)?;
    let app_data_dir = crate::app_paths::app_data_dir();
    let include_audio = settings::load(&app_data_dir).include_audio;
    let dest = backup_folder();
    let pool = state.db_manager.pool().clone();
    let progress = EmitProgress::new(app.clone());
    let token = guard.token().clone();
    let opts = ExportOptions {
        include_audio,
        app_data_dir,
        abort_if: None,
        allowed_roots: crate::audio::recording_preferences::known_recording_roots(),
    };
    let result = blocking(async move {
        let summary = export_to(&pool, &dest, &opts, &progress, &token).await?;
        Ok(summary.path.to_string_lossy().into_owned())
    })
    .await;
    drop(guard);
    emit_finished(&app, JobKind::Export, &result.as_ref().map(|_| ()).map_err(Clone::clone), result.as_ref().ok().cloned());
    result.map(Some)
}

/// The backups in the backup folder, newest first.
#[tauri::command]
pub async fn api_backup_list() -> Vec<BackupFile> {
    tauri::async_runtime::spawn_blocking(|| library::list(&backup_folder()))
        .await
        .unwrap_or_default()
}

/// Delete one backup from the backup folder (only a backup file directly inside it).
#[tauri::command]
pub fn api_backup_delete(path: String) -> Result<(), BackupError> {
    library::delete(&backup_folder(), Path::new(&path))
}

/// Show one backup from the backup folder in Finder.
#[tauri::command]
pub fn api_backup_reveal(path: String) -> Result<(), BackupError> {
    library::reveal(&backup_folder(), Path::new(&path))
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

/// The settings, with `folder` always filled in (the recordings folder when none is chosen).
#[tauri::command]
pub fn api_backup_get_settings() -> BackupSettings {
    let mut s = settings::load(&crate::app_paths::app_data_dir());
    s.folder = Some(backup_folder());
    s
}

#[tauri::command]
pub fn api_backup_set_settings(enabled: bool, include_audio: bool) -> Result<BackupSettings, BackupError> {
    let dir = crate::app_paths::app_data_dir();
    settings::update(&dir, |s| s.apply_ui(enabled, include_audio))?;
    Ok(api_backup_get_settings())
}

/// Choose where backups go. Existing backups stay where they are.
#[tauri::command]
pub fn api_backup_set_folder(folder: String) -> Result<BackupSettings, BackupError> {
    let dir = crate::app_paths::app_data_dir();
    settings::update(&dir, |s| s.set_folder(Some(folder)))?;
    Ok(api_backup_get_settings())
}

#[tauri::command]
pub async fn api_backup_pick_folder<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, BackupError> {
    let picked = tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| BackupError::Io(format!("folder picker failed: {e}")))?;
    Ok(picked.map(|p| p.to_string()))
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
        let m: ImportMode = serde_json::from_str(r#""add""#).unwrap();
        assert_eq!(m, ImportMode::Add);
    }
}
