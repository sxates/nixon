//! Replace restore, staging half (specs/0092, "Import: shared front half" and "Replace").
//!
//! `stage` runs in the live app: safety copy, extraction, migration of the staged DB, marker.
//! The destructive half runs at the next launch, before the DB opens: see `replace_apply.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

use super::archive::{extract, read_manifest};
use super::export::{export_to, ExportOptions};
use super::manifest::{build_schema_version, validate, Manifest};
use super::progress::Progress;
use super::BackupError;

pub use super::replace_apply::{apply_pending, apply_until, cleanup_replaced};

pub const MARKER_NAME: &str = "restore-pending.json";
pub const RESULT_NAME: &str = "restore-result.json";
pub const STAGING_DIR_NAME: &str = "restore-staging";
pub const RECORDINGS_STAGING_NAME: &str = ".nixon-restore-staging";
/// The staged database, relative to the staging dir (where `extract` puts it).
pub const STAGED_DB: &str = "db/meeting_minutes.sqlite";

const PRE_RESTORE_DIR: &str = "pre-restore";
const SAFETY_SUFFIX: &str = " (before restore).zip";
const SAFETY_KEEP: usize = 3;

/// What `restore-pending.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub staging_dir: String,
    pub recordings_staging: String,
    pub recordings_root: String,
    pub manifest: Manifest,
    pub created_at: String,
}

/// What a finished restore reports to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestoreOutcome {
    pub meetings: u32,
    /// The backup's `created_at`.
    pub from: String,
}

/// Write `bytes` to `dest` through a sibling temp file and a rename.
pub(super) fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let mut tmp_name = dest.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = dest.with_file_name(tmp_name);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, dest)?;
    Ok(())
}

fn remove_dir_if_exists(p: &Path) -> Result<(), BackupError> {
    match fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Undo a successful [`stage`] that will not be applied: remove the marker and both staging
/// dirs it names. Best effort; a missing marker is fine.
pub fn cancel_pending(app_data_dir: &Path) {
    let marker_path = app_data_dir.join(MARKER_NAME);
    if let Ok(bytes) = fs::read(&marker_path) {
        if let Ok(m) = serde_json::from_slice::<Marker>(&bytes) {
            let _ = fs::remove_dir_all(&m.staging_dir);
            let _ = fs::remove_dir_all(&m.recordings_staging);
        }
    }
    let _ = fs::remove_file(&marker_path);
}

/// Export the current data to `pre-restore/<stem> (before restore).zip`, keep the newest 3.
async fn write_safety_copy(
    pool: &SqlitePool,
    app_data_dir: &Path,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<(), BackupError> {
    let dir = app_data_dir.join(PRE_RESTORE_DIR);
    let opts = ExportOptions {
        include_audio: false,
        app_data_dir: app_data_dir.to_path_buf(),
        abort_if: None,
        allowed_roots: Vec::new(),
    };
    let summary = export_to(pool, &dir, &opts, progress, cancel).await?;
    let stem = summary
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // `export_to` only checks for `<stem>.zip`, so never let a rename overwrite an older copy.
    let mut target = dir.join(format!("{stem}{SAFETY_SUFFIX}"));
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem} {n}{SAFETY_SUFFIX}"));
        n += 1;
    }
    fs::rename(&summary.path, &target)?;

    let mut copies: Vec<PathBuf> = fs::read_dir(&dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(SAFETY_SUFFIX))
        })
        .collect();
    copies.sort();
    let excess = copies.len().saturating_sub(SAFETY_KEEP);
    for old in &copies[..excess] {
        let _ = fs::remove_file(old);
    }
    Ok(())
}

/// Bring the staged DB's schema up to this build, as a single self-contained file.
pub(super) async fn migrate_staged(db: &Path) -> Result<(), BackupError> {
    let opts = SqliteConnectOptions::new()
        .filename(db)
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Delete)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    let result = sqlx::migrate!("./migrations").run(&pool).await;
    pool.close().await;
    result.map_err(|e| BackupError::Db(e.to_string()))
}

/// Validate the archive, write a safety copy, extract and migrate into staging, then write the
/// marker. Nothing the app uses is touched; a failure leaves no marker and no staging dirs.
pub async fn stage(
    pool: &SqlitePool,
    zip: &Path,
    app_data_dir: &Path,
    recordings_root: &Path,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<(), BackupError> {
    let manifest = read_manifest(zip)?;
    validate(&manifest, build_schema_version())?;

    // An older pending marker would point at staging we are about to clear.
    match fs::remove_file(app_data_dir.join(MARKER_NAME)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    write_safety_copy(pool, app_data_dir, progress, cancel).await?;

    let staging = app_data_dir.join(STAGING_DIR_NAME);
    let rec_staging = recordings_root.join(RECORDINGS_STAGING_NAME);
    let result = stage_inner(
        zip, &staging, &rec_staging, app_data_dir, recordings_root, manifest, progress, cancel,
    )
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir_all(&rec_staging);
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn stage_inner(
    zip: &Path,
    staging: &Path,
    rec_staging: &Path,
    app_data_dir: &Path,
    recordings_root: &Path,
    manifest: Manifest,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<(), BackupError> {
    remove_dir_if_exists(staging)?;
    remove_dir_if_exists(rec_staging)?;
    fs::create_dir_all(staging)?;
    fs::create_dir_all(recordings_root)?;
    extract(zip, staging, rec_staging, None, progress, cancel)?;
    let staged_db = staging.join(STAGED_DB);
    if !staged_db.is_file() {
        return Err(BackupError::NotABackup);
    }
    migrate_staged(&staged_db).await?;
    if cancel.is_cancelled() {
        return Err(BackupError::Cancelled);
    }
    let marker = Marker {
        staging_dir: staging.to_string_lossy().into_owned(),
        recordings_staging: rec_staging.to_string_lossy().into_owned(),
        recordings_root: recordings_root.to_string_lossy().into_owned(),
        manifest,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    let json = serde_json::to_vec_pretty(&marker)
        .map_err(|e| BackupError::Io(format!("marker: {e}")))?;
    write_atomic(&app_data_dir.join(MARKER_NAME), &json)
}
