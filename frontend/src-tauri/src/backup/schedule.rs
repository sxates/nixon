//! Daily scheduled backups (specs/0092, "Scheduled backups").

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use super::export::{export_auto_to, ExportOptions, ExportSummary};
use super::files::settings_entries;
use super::job::{self, blocking, Finished, JobKind, EVENT_FINISHED};
use super::progress::NoProgress;
use super::settings::{self, BackupSettings};
use super::snapshot::{logical_fingerprint, snapshot_db};
use super::BackupError;

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(120);
const TICK: Duration = Duration::from_secs(3600);
const DEAD_PARTIAL_AGE: Duration = Duration::from_secs(24 * 3600);
/// Automatic backups kept in the folder; backups made by hand are never pruned.
pub const AUTO_KEEP: u32 = 5;

#[derive(Debug)]
pub enum RunResult {
    /// Nothing changed since the last backup; counted as today's success.
    Skipped,
    Written(ExportSummary),
    NotDue,
}

/// Enabled, and no success yet on `now`'s local date.
pub fn is_due(now: DateTime<Local>, s: &BackupSettings) -> bool {
    if !s.enabled {
        return false;
    }
    match s
        .last_success_at
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
    {
        Some(t) => t.with_timezone(&Local).date_naive() != now.date_naive(),
        None => true,
    }
}

/// `Nixon Auto Backup YYYY-MM-DD HHMM.zip`, exactly (no suffixed copies): the only files
/// pruning deletes.
fn is_auto_backup_name(name: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^Nixon Auto Backup \d{4}-\d{2}-\d{2} \d{4}\.zip$").unwrap()
    })
    .is_match(name)
}

/// Dead leftovers of our own exports, manual or automatic:
/// `Nixon [Auto ]Backup YYYY-MM-DD HHMM[ N].zip.partial`.
fn is_partial_name(name: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^Nixon (Auto )?Backup \d{4}-\d{2}-\d{2} \d{4}( \d+)?\.zip\.partial$")
            .unwrap()
    })
    .is_match(name)
}

/// Delete automatic backups beyond the newest `keep` (by name = time) and `.partial` files
/// older than a day. Backups made by hand and anything else are never touched. Returns what
/// was deleted.
pub fn prune(folder: &Path, keep: u32) -> Result<Vec<PathBuf>, BackupError> {
    let mut backups: Vec<(String, PathBuf)> = Vec::new();
    let mut dead: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(folder)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        if is_auto_backup_name(&name) {
            backups.push((name, entry.path()));
        } else if is_partial_name(&name) {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| SystemTime::now().duration_since(m).ok())
                .is_some_and(|age| age > DEAD_PARTIAL_AGE);
            if old {
                dead.push(entry.path());
            }
        }
    }
    backups.sort_by(|a, b| a.0.cmp(&b.0));
    let excess = backups.len().saturating_sub(keep.max(1) as usize);
    dead.extend(backups.into_iter().take(excess).map(|(_, p)| p));
    let mut deleted = Vec::new();
    for p in dead {
        if std::fs::remove_file(&p).is_ok() {
            deleted.push(p);
        }
    }
    Ok(deleted)
}

/// Whether `folder` holds at least one automatic backup.
fn has_backup(folder: &Path) -> bool {
    std::fs::read_dir(folder).is_ok_and(|rd| {
        rd.flatten().any(|e| is_auto_backup_name(&e.file_name().to_string_lossy()))
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fingerprint of what a backup would contain right now (via a throwaway snapshot).
async fn current_fingerprint(
    pool: &SqlitePool,
    app_data_dir: &Path,
    include_audio: bool,
) -> Result<String, BackupError> {
    let tmp = app_data_dir.join(format!(".backup-fingerprint-{}.sqlite", uuid::Uuid::new_v4()));
    let result = async {
        let info = snapshot_db(pool, &tmp).await?;
        let folders: Vec<(PathBuf, bool)> = if include_audio {
            info.recordings
                .iter()
                .filter_map(|(_, f)| f.as_ref())
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
                .map(|p| (p, true))
                .collect()
        } else {
            Vec::new()
        };
        logical_fingerprint(&tmp, &settings_entries(app_data_dir), &folders).await
    }
    .await;
    let _ = std::fs::remove_file(&tmp);
    result.map(|fp| hex(&fp))
}

/// One scheduled-backup attempt. `force` ignores "due" and "enabled". With no folder chosen,
/// backups go to `default_folder` (the recordings folder), created if needed.
#[allow(clippy::too_many_arguments)]
pub async fn run_once(
    pool: &SqlitePool,
    app_data_dir: &Path,
    now: DateTime<Local>,
    is_recording: Arc<dyn Fn() -> bool + Send + Sync>,
    force: bool,
    allowed_roots: Vec<PathBuf>,
    default_folder: &Path,
) -> Result<RunResult, BackupError> {
    let mut s = settings::load(app_data_dir);
    if !force && !is_due(now, &s) {
        return Ok(RunResult::NotDue);
    }
    let chosen = s.folder.clone();
    let folder = s.folder_or(default_folder);
    if chosen.is_none() {
        let _ = std::fs::create_dir_all(&folder);
    }
    s.last_attempt_at = Some(now.to_rfc3339());

    let outcome =
        attempt(pool, app_data_dir, &folder, &mut s, now, is_recording, allowed_roots, force).await;
    match &outcome {
        Ok(_) => s.last_error = None,
        // A recording starting is not a failure; the next tick retries quietly.
        Err(BackupError::Cancelled) => {}
        Err(e) => s.last_error = Some(e.to_string()),
    }
    // Patch only this run's own fields into whatever backup.json holds now: the user may have
    // changed the settings while the export ran. A result for a folder (or audio choice) that
    // is no longer current is dropped. A failed save must not hide a backup that was written
    // (it would be retried hourly).
    let patched = settings::update(app_data_dir, |cur| {
        if cur.folder != chosen || cur.include_audio != s.include_audio {
            return;
        }
        cur.last_attempt_at = s.last_attempt_at.clone();
        cur.last_success_at = s.last_success_at.clone();
        cur.last_fingerprint = s.last_fingerprint.clone();
        cur.last_size = s.last_size;
        cur.last_error = s.last_error.clone();
    });
    if let Err(e) = patched {
        log::warn!("backup: could not save backup.json: {e}");
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn attempt(
    pool: &SqlitePool,
    app_data_dir: &Path,
    folder: &Path,
    s: &mut BackupSettings,
    now: DateTime<Local>,
    is_recording: Arc<dyn Fn() -> bool + Send + Sync>,
    allowed_roots: Vec<PathBuf>,
    force: bool,
) -> Result<RunResult, BackupError> {
    if !folder.is_dir() {
        return Err(BackupError::Io(format!(
            "Backup folder is unavailable: {}",
            folder.display()
        )));
    }
    let fingerprint = current_fingerprint(pool, app_data_dir, s.include_audio).await?;
    // "Unchanged" only counts while a backup is really there to point at, and never for an
    // explicit Back up now.
    if !force
        && s.last_fingerprint.as_deref() == Some(fingerprint.as_str())
        && has_backup(folder)
    {
        s.last_success_at = Some(now.to_rfc3339());
        return Ok(RunResult::Skipped);
    }
    let opts = ExportOptions {
        include_audio: s.include_audio,
        app_data_dir: app_data_dir.to_path_buf(),
        abort_if: Some(is_recording),
        allowed_roots,
    };
    let summary =
        export_auto_to(pool, folder, &opts, &NoProgress, &CancellationToken::new()).await?;
    s.last_success_at = Some(now.to_rfc3339());
    s.last_fingerprint = Some(fingerprint);
    s.last_size = Some(summary.bytes);
    // The backup is written; a prune failure must not turn it into an error.
    let _ = prune(folder, AUTO_KEEP);
    Ok(RunResult::Written(summary))
}

/// First check after 2 minutes, then hourly. Skips a tick while a recording is active.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        // Sweep the recordings-folder staging dirs, with the job lock held so a job the user
        // just started is never swept from under itself.
        if let Ok(_guard) = job::try_begin(JobKind::Scheduled) {
            super::sweep::sweep_leftovers(
                &crate::app_paths::app_data_dir(),
                &crate::audio::recording_preferences::known_recording_roots(),
            );
        }
        loop {
            tick(&app).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn tick(app: &AppHandle) {
    if crate::audio::recording_commands::is_recording().await {
        return;
    }
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    // Another job is running: skip this tick quietly; the next one retries.
    let Ok(_guard) = job::try_begin(JobKind::Scheduled) else {
        return;
    };
    let pool = state.db_manager.pool().clone();
    // Export does synchronous file I/O: keep it off the async worker threads.
    let result = blocking(async move {
        run_once(
            &pool,
            &crate::app_paths::app_data_dir(),
            Local::now(),
            Arc::new(crate::audio::recording_commands::is_recording_now),
            false,
            crate::audio::recording_preferences::known_recording_roots(),
            &crate::audio::recording_preferences::recordings_root(),
        )
        .await
    })
    .await;
    let payload = match result {
        Ok(RunResult::Written(sum)) => Finished::ok(
            JobKind::Scheduled,
            Some(sum.path.to_string_lossy().into_owned()),
        ),
        Err(BackupError::Cancelled) | Ok(_) => return,
        Err(e) => Finished::err(JobKind::Scheduled, &e),
    };
    let _ = app.emit(EVENT_FINISHED, payload);
}
