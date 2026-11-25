//! Add-what's-missing import, orchestration half (specs/0092). Never edits what is already here.
//!
//! Staging lives in `<app data>/add-staging/` and `<recordings root>/.nixon-add-staging/`
//! (cleared first, removed at the end, success or not). Recording folders are placed and the
//! new meetings' `folder_path`s rewritten inside the merge transaction, before `COMMIT`; any
//! failure rolls back and puts the folders back.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use sqlx::{SqliteConnection, SqlitePool};
use tokio_util::sync::CancellationToken;

use super::archive::{extract, read_manifest};
use super::manifest::{build_schema_version, validate, Manifest};
use super::merge::{self, MergeCounts};
use super::progress::Progress;
use super::relocate::{place_folders, undo, Placed};
use super::replace::{migrate_staged, MARKER_NAME, STAGED_DB};
use super::BackupError;

pub const ADD_STAGING_DIR_NAME: &str = "add-staging";
pub const ADD_RECORDINGS_STAGING_NAME: &str = ".nixon-add-staging";
/// Where the second extraction (recordings of the new meetings) writes its copy of `db/`.
const SECOND_PASS_DIR: &str = "recordings-pass";

fn remove_dir_if_exists(p: &Path) -> Result<(), BackupError> {
    match fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Add everything in the backup that this Mac does not have. See the module docs.
pub async fn import_add(
    pool: &SqlitePool,
    zip: &Path,
    app_data_dir: &Path,
    recordings_root: &Path,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<MergeCounts, BackupError> {
    let manifest = read_manifest(zip)?;
    validate(&manifest, build_schema_version())?;
    if app_data_dir.join(MARKER_NAME).exists() {
        return Err(BackupError::Busy);
    }
    let staging = app_data_dir.join(ADD_STAGING_DIR_NAME);
    let rec_staging = recordings_root.join(ADD_RECORDINGS_STAGING_NAME);
    let result = run(pool, zip, &manifest, &staging, &rec_staging, recordings_root, progress, cancel).await;
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&rec_staging);
    result
}

#[allow(clippy::too_many_arguments)]
async fn run(
    pool: &SqlitePool,
    zip: &Path,
    manifest: &Manifest,
    staging: &Path,
    rec_staging: &Path,
    recordings_root: &Path,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<MergeCounts, BackupError> {
    remove_dir_if_exists(staging)?;
    remove_dir_if_exists(rec_staging)?;
    fs::create_dir_all(staging)?;
    fs::create_dir_all(recordings_root)?;

    // Pass 1: the database (no recording folders).
    extract(zip, staging, rec_staging, Some(&HashSet::new()), progress, cancel)?;
    let staged_db = staging.join(STAGED_DB);
    if !staged_db.is_file() {
        return Err(BackupError::NotABackup);
    }
    migrate_staged(&staged_db).await?;

    // Pass 2, outside the write lock: recording folders of the meetings that look new. The
    // merge recomputes the real set under the lock and only places folders for those.
    let candidates = candidate_meetings(pool, &staged_db).await?;
    let wanted: HashSet<String> = manifest
        .recordings
        .iter()
        .filter(|r| candidates.contains(&r.meeting_id))
        .map(|r| r.meeting_id.clone())
        .collect();
    if !wanted.is_empty() {
        extract(zip, &staging.join(SECOND_PASS_DIR), rec_staging, Some(&wanted), progress, cancel)?;
    }
    if cancel.is_cancelled() {
        return Err(BackupError::Cancelled);
    }

    let mut conn = pool.acquire().await?;
    merge::begin(&mut conn, &staged_db).await?;
    let counts = match merge::insert_missing(&mut conn).await {
        Ok(c) => c,
        Err(e) => {
            let _ = merge::end(&mut conn, false).await;
            return Err(e);
        }
    };
    let placed = match place_and_rewrite(&mut conn, &counts, manifest, rec_staging, recordings_root).await {
        Ok(p) => p,
        Err(e) => {
            let _ = merge::end(&mut conn, false).await;
            return Err(e);
        }
    };
    if cancel.is_cancelled() {
        let _ = merge::end(&mut conn, false).await;
        let _ = undo(&placed, rec_staging);
        return Err(BackupError::Cancelled);
    }
    match merge::end(&mut conn, true).await {
        // Committed. If DETACH failed the connection still has `backup` attached: close it
        // instead of returning it to the pool. Never undo anything after a commit.
        Ok(true) => {}
        Ok(false) => {
            log::warn!("backup import: DETACH failed after commit; closing the connection");
            drop(conn.detach());
        }
        Err(e) => {
            let _ = undo(&placed, rec_staging);
            return Err(e);
        }
    }
    Ok(counts)
}

/// Backup meeting ids this Mac does not have (read before the write lock is taken).
async fn candidate_meetings(
    pool: &SqlitePool,
    staged_db: &Path,
) -> Result<HashSet<String>, BackupError> {
    let here: HashSet<String> = sqlx::query_scalar("SELECT id FROM meetings")
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    let mut conn = <SqliteConnection as sqlx::Connection>::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(staged_db)
            .read_only(true)
            .create_if_missing(false),
    )
    .await?;
    let theirs: Result<Vec<String>, _> =
        sqlx::query_scalar("SELECT id FROM meetings").fetch_all(&mut conn).await;
    let _ = sqlx::Connection::close(conn).await;
    Ok(theirs?.into_iter().filter(|id| !here.contains(id)).collect())
}

/// Move the new meetings' folders into the root and point their `folder_path` at them (NULL
/// when the backup carried no folder: the old Mac's path means nothing here). On failure the
/// folders are put back; the caller rolls back the transaction.
async fn place_and_rewrite(
    conn: &mut SqliteConnection,
    counts: &MergeCounts,
    manifest: &Manifest,
    rec_staging: &Path,
    root: &Path,
) -> Result<Vec<Placed>, BackupError> {
    if counts.new_meeting_ids.is_empty() {
        return Ok(Vec::new());
    }
    let new_ids: HashSet<&str> = counts.new_meeting_ids.iter().map(String::as_str).collect();
    let refs: Vec<_> = manifest
        .recordings
        .iter()
        .filter(|r| new_ids.contains(r.meeting_id.as_str()))
        .cloned()
        .collect();
    let placed = place_folders(rec_staging, root, &refs)?;
    let rewrite = async {
        for id in &counts.new_meeting_ids {
            let path = placed
                .iter()
                .find(|p| &p.meeting_id == id)
                .map(|p| p.path.to_string_lossy().into_owned());
            sqlx::query("UPDATE main.meetings SET folder_path = ? WHERE id = ?")
                .bind(path)
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }
        Ok::<(), BackupError>(())
    }
    .await;
    if let Err(e) = rewrite {
        let _ = undo(&placed, rec_staging);
        return Err(e);
    }
    Ok(placed)
}
