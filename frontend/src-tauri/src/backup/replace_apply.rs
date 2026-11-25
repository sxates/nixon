//! Replace restore, launch-time half (specs/0092, "Import: Replace" step 4).
//!
//! Runs before the DB opens. Every step records or re-checks what it did, so a launch that
//! finds the marker again after a crash can run the whole sequence again safely. Progress is
//! kept in `<staging>/apply-state.json`:
//! - `placed` is written right after the folders are renamed (before any DB write) so a re-run
//!   can still rewrite their `folder_path`;
//! - `swap_ts` is written before the first file is moved aside; once set, steps 1-3 never run
//!   again and the swap is completed from whatever is on disk.
//!
//! Any failure puts everything back (folders, old DB, replaced settings) and drops the marker.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{ConnectOptions, Connection, SqliteConnection};

use super::files::SETTINGS_ALLOWLIST;
use super::relocate::{place_folders, undo, Placed};
use super::replace::{
    write_atomic, Marker, RestoreOutcome, MARKER_NAME, RECORDINGS_STAGING_NAME, RESULT_NAME,
    STAGED_DB, STAGING_DIR_NAME,
};
use super::{BackupError, MACHINE_BOUND_TABLES};

const DB_NAME: &str = "meeting_minutes.sqlite";
const DB_FILES: [&str; 3] = ["", "-wal", "-shm"];
const STATE_NAME: &str = "apply-state.json";
const OUTGOING_SETTINGS: &str = "outgoing-settings";
const RECORDING_PREFS: &str = "recording_preferences.json";
const REPLACED_MARK: &str = ".replaced-";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlacedRec {
    meeting_id: String,
    path: String,
    original: String,
}

impl From<&Placed> for PlacedRec {
    fn from(p: &Placed) -> Self {
        Self {
            meeting_id: p.meeting_id.clone(),
            path: p.path.to_string_lossy().into_owned(),
            original: p.original.clone(),
        }
    }
}

impl From<&PlacedRec> for Placed {
    fn from(p: &PlacedRec) -> Self {
        Self {
            meeting_id: p.meeting_id.clone(),
            path: PathBuf::from(&p.path),
            original: p.original.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    /// This Mac's `save_folder` / `save_folder_user_chosen`, read before anything changed.
    outgoing_save_folder: Option<String>,
    outgoing_user_chosen: bool,
    placed: Option<Vec<PlacedRec>>,
    swap_ts: Option<String>,
}

impl State {
    fn load(staging: &Path) -> Result<Option<State>, BackupError> {
        match fs::read(staging.join(STATE_NAME)) {
            Ok(b) => serde_json::from_slice(&b)
                .map(Some)
                .map_err(|e| BackupError::Io(format!("apply state: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, staging: &Path) -> Result<(), BackupError> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| BackupError::Io(format!("apply state: {e}")))?;
        write_atomic(&staging.join(STATE_NAME), &json)
    }

    fn placed_list(&self) -> Vec<Placed> {
        self.placed
            .as_ref()
            .map(|v| v.iter().map(Placed::from).collect())
            .unwrap_or_default()
    }
}

/// `(save_folder, save_folder_user_chosen)` from this Mac's current preferences file.
fn read_outgoing_prefs(app_data_dir: &Path) -> (Option<String>, bool) {
    let value: Option<serde_json::Value> = fs::read(app_data_dir.join(RECORDING_PREFS))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let prefs = value.as_ref().and_then(|v| v.get("preferences"));
    let folder = prefs
        .and_then(|p| p.get("save_folder"))
        .and_then(|f| f.as_str())
        .map(str::to_string);
    let chosen = prefs
        .and_then(|p| p.get("save_folder_user_chosen"))
        .and_then(|c| c.as_bool())
        .unwrap_or(false);
    (folder, chosen)
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// A standalone connection with foreign keys off and no WAL (the staged DB stays one file).
async fn open_staged(path: &Path) -> Result<SqliteConnection, BackupError> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Delete)
        .foreign_keys(false)
        .busy_timeout(std::time::Duration::from_secs(10))
        .disable_statement_logging();
    Ok(opts.connect().await?)
}

async fn table_columns(
    conn: &mut SqliteConnection,
    table: &str,
    schema: &str,
) -> Result<Vec<String>, BackupError> {
    Ok(
        sqlx::query_scalar("SELECT name FROM pragma_table_info(?, ?)")
            .bind(table)
            .bind(schema)
            .fetch_all(&mut *conn)
            .await?,
    )
}

/// Step 1: this Mac's rows win in every machine-bound table.
async fn carry_over(staged: &Path, current: &Path) -> Result<(), BackupError> {
    if !current.is_file() {
        return Ok(());
    }
    let mut conn = open_staged(staged).await?;
    let result = carry_over_on(&mut conn, current).await;
    let closed = conn.close().await;
    result?;
    closed?;
    Ok(())
}

async fn carry_over_on(conn: &mut SqliteConnection, current: &Path) -> Result<(), BackupError> {
    sqlx::query("ATTACH DATABASE ? AS cur")
        .bind(current.to_string_lossy().into_owned())
        .execute(&mut *conn)
        .await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
    let copied = copy_tables(conn).await;
    if copied.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
        return copied;
    }
    sqlx::query("COMMIT").execute(&mut *conn).await?;
    sqlx::query("DETACH DATABASE cur").execute(&mut *conn).await?;
    Ok(())
}

async fn copy_tables(conn: &mut SqliteConnection) -> Result<(), BackupError> {
    for table in MACHINE_BOUND_TABLES.iter().filter(|t| **t != "_sqlx_migrations") {
        let mine = table_columns(conn, table, "main").await?;
        let theirs = table_columns(conn, table, "cur").await?;
        let shared: Vec<String> = mine.into_iter().filter(|c| theirs.contains(c)).collect();
        if shared.is_empty() {
            continue; // not in both
        }
        let cols = shared.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
        let t = quote(table);
        sqlx::query(&format!("DELETE FROM main.{t}"))
            .execute(&mut *conn)
            .await?;
        sqlx::query(&format!(
            "INSERT INTO main.{t} ({cols}) SELECT {cols} FROM cur.{t}"
        ))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Step 3: placed meetings point at their new folder; the rest keep a path only if it exists.
async fn rewrite_folder_paths(staged: &Path, placed: &[Placed]) -> Result<(), BackupError> {
    let mut conn = open_staged(staged).await?;
    let result = rewrite_on(&mut conn, placed).await;
    let closed = conn.close().await;
    result?;
    closed?;
    Ok(())
}

async fn rewrite_on(conn: &mut SqliteConnection, placed: &[Placed]) -> Result<(), BackupError> {
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
    let done = rewrite_rows(conn, placed).await;
    if done.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
        return done;
    }
    sqlx::query("COMMIT").execute(&mut *conn).await?;
    Ok(())
}

async fn rewrite_rows(conn: &mut SqliteConnection, placed: &[Placed]) -> Result<(), BackupError> {
    let mut ids = HashSet::new();
    for p in placed {
        sqlx::query("UPDATE meetings SET folder_path = ? WHERE id = ?")
            .bind(p.path.to_string_lossy().into_owned())
            .bind(&p.meeting_id)
            .execute(&mut *conn)
            .await?;
        ids.insert(p.meeting_id.clone());
    }
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, folder_path FROM meetings WHERE folder_path IS NOT NULL")
            .fetch_all(&mut *conn)
            .await?;
    for (id, folder) in rows {
        if ids.contains(&id) || Path::new(&folder).is_dir() {
            continue;
        }
        sqlx::query("UPDATE meetings SET folder_path = NULL WHERE id = ?")
            .bind(&id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

fn aside_path(app_data_dir: &Path, ext: &str, ts: &str) -> PathBuf {
    app_data_dir.join(format!("{DB_NAME}{ext}{REPLACED_MARK}{ts}"))
}

/// Step 4: current DB (+WAL/SHM) aside, staged DB in. Safe to repeat after a partial run.
fn swap_db(app_data_dir: &Path, staged: &Path, ts: &str) -> Result<(), BackupError> {
    if !staged.is_file() {
        return Ok(()); // already moved in
    }
    for ext in DB_FILES {
        let cur = app_data_dir.join(format!("{DB_NAME}{ext}"));
        let aside = aside_path(app_data_dir, ext, ts);
        if cur.exists() && !aside.exists() {
            fs::rename(&cur, &aside)?;
        }
    }
    for ext in DB_FILES.iter().skip(1) {
        // A staged sidecar would belong to the staged DB; keep it with its file.
        let side = staged.with_file_name(format!("{DB_NAME}{ext}"));
        if side.exists() {
            fs::rename(&side, app_data_dir.join(format!("{DB_NAME}{ext}")))?;
        }
    }
    fs::rename(staged, app_data_dir.join(DB_NAME))?;
    Ok(())
}

/// The settings files a backup may overwrite: `(path relative to the staged settings dir)`.
fn staged_settings(settings: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = SETTINGS_ALLOWLIST
        .iter()
        .map(PathBuf::from)
        .filter(|p| settings.join(p).is_file())
        .collect();
    fn walk(dir: &Path, rel: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let r = rel.join(e.file_name());
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path(), &r, out),
                Ok(t) if t.is_file() => out.push(r),
                _ => {}
            }
        }
    }
    walk(&settings.join("templates"), Path::new("templates"), &mut out);
    out
}

/// The restored `recording_preferences.json`: the backup's file, but this Mac's save folder.
fn merged_recording_prefs(backup: &[u8], st: &State) -> Result<Vec<u8>, BackupError> {
    let mut value: serde_json::Value = serde_json::from_slice(backup)
        .map_err(|e| BackupError::Io(format!("recording preferences: {e}")))?;
    if !value.is_object() {
        value = serde_json::json!({});
    }
    let root = value.as_object_mut().expect("object");
    let prefs = root
        .entry("preferences")
        .or_insert_with(|| serde_json::json!({}));
    if !prefs.is_object() {
        *prefs = serde_json::json!({});
    }
    let prefs = prefs.as_object_mut().expect("object");
    match &st.outgoing_save_folder {
        Some(f) => {
            prefs.insert("save_folder".into(), serde_json::json!(f));
        }
        None => {
            prefs.remove("save_folder");
        }
    }
    prefs.insert(
        "save_folder_user_chosen".into(),
        serde_json::json!(st.outgoing_user_chosen),
    );
    prefs.insert("previous_save_folders".into(), serde_json::json!([]));
    serde_json::to_vec_pretty(&value).map_err(|e| BackupError::Io(format!("recording prefs: {e}")))
}

/// Step 5: copy allowlisted settings over (keeping the outgoing ones for a rollback).
fn apply_settings(app_data_dir: &Path, staging: &Path, st: &State) -> Result<(), BackupError> {
    let settings = staging.join("settings");
    let keep = staging.join(OUTGOING_SETTINGS);
    for rel in staged_settings(&settings) {
        let dest = app_data_dir.join(&rel);
        let saved = keep.join(&rel);
        if dest.is_file() && !saved.exists() {
            if let Some(parent) = saved.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&dest, &saved)?;
        }
        let bytes = fs::read(settings.join(&rel))?;
        let bytes = if rel == Path::new(RECORDING_PREFS) {
            merged_recording_prefs(&bytes, st)?
        } else {
            bytes
        };
        write_atomic(&dest, &bytes)?;
    }
    Ok(())
}

fn restore_settings(app_data_dir: &Path, staging: &Path) {
    let keep = staging.join(OUTGOING_SETTINGS);
    for rel in staged_settings(&keep) {
        let _ = fs::copy(keep.join(&rel), app_data_dir.join(&rel));
    }
    // staged_settings only knows allowlisted names, which is exactly what was saved.
}

/// Undo steps 1-5 as far as they got. Best effort; nothing here may fail the launch.
fn rollback(app_data_dir: &Path, marker: &Marker) {
    let staging = PathBuf::from(&marker.staging_dir);
    if let Ok(Some(st)) = State::load(&staging) {
        if let Some(ts) = &st.swap_ts {
            for ext in DB_FILES {
                let cur = app_data_dir.join(format!("{DB_NAME}{ext}"));
                let aside = aside_path(app_data_dir, ext, ts);
                if !aside.exists() {
                    continue;
                }
                if cur.exists() {
                    let staged = staging.join(STAGED_DB);
                    if ext.is_empty() && !staged.exists() {
                        let _ = fs::create_dir_all(staged.parent().unwrap_or(&staging));
                        let _ = fs::rename(&cur, &staged); // keep the restored DB for diagnosis
                    } else {
                        let _ = fs::remove_file(&cur);
                    }
                }
                let _ = fs::rename(&aside, &cur);
            }
        }
        if let Err(e) = undo(&st.placed_list(), Path::new(&marker.recordings_staging)) {
            log::error!("restore rollback: could not move recording folders back: {e}");
        }
        restore_settings(app_data_dir, &staging);
    }
}

fn write_result(
    app_data_dir: &Path,
    ok: bool,
    meetings: u32,
    created_at: &str,
    error: Option<String>,
) {
    let mut v = serde_json::json!({ "ok": ok, "meetings": meetings, "created_at": created_at });
    if let Some(e) = error {
        v["error"] = serde_json::json!(e);
    }
    let bytes = serde_json::to_vec_pretty(&v).unwrap_or_default();
    if let Err(e) = write_atomic(&app_data_dir.join(RESULT_NAME), &bytes) {
        log::error!("could not write {RESULT_NAME}: {e}");
    }
}

fn remove_marker(app_data_dir: &Path) {
    let _ = fs::remove_file(app_data_dir.join(MARKER_NAME));
}

/// Delete a staging dir, but only one that has the name we gave it.
fn remove_staging(dir: &Path, expected_name: &str) {
    if dir.file_name().is_some_and(|n| n == expected_name) {
        let _ = fs::remove_dir_all(dir);
    }
}

enum Flow {
    Stopped,
    Done,
}

async fn run_steps(
    app_data_dir: &Path,
    marker: &Marker,
    stop_after: u8,
) -> Result<Flow, BackupError> {
    let staging = PathBuf::from(&marker.staging_dir);
    let rec_staging = PathBuf::from(&marker.recordings_staging);
    let root = PathBuf::from(&marker.recordings_root);
    let staged_db = staging.join(STAGED_DB);
    let current_db = app_data_dir.join(DB_NAME);

    let mut st = match State::load(&staging)? {
        Some(s) => s,
        None => {
            let (folder, chosen) = read_outgoing_prefs(app_data_dir);
            let s = State {
                outgoing_save_folder: folder,
                outgoing_user_chosen: chosen,
                placed: None,
                swap_ts: None,
            };
            s.save(&staging)?;
            s
        }
    };

    if st.swap_ts.is_none() {
        if !staged_db.is_file() {
            return Err(BackupError::Io("the staged database is missing".into()));
        }
        // 1. this Mac's machine-bound rows
        carry_over(&staged_db, &current_db).await?;
        if stop_after == 1 {
            return Ok(Flow::Stopped);
        }
        // 2. recording folders into the root
        if st.placed.is_none() {
            let placed = place_folders(&rec_staging, &root, &marker.manifest.recordings)?;
            st.placed = Some(placed.iter().map(PlacedRec::from).collect());
            if let Err(e) = st.save(&staging) {
                let _ = undo(&placed, &rec_staging);
                return Err(e);
            }
        }
        if stop_after == 2 {
            return Ok(Flow::Stopped);
        }
        // 3. folder_path rewrite
        rewrite_folder_paths(&staged_db, &st.placed_list()).await?;
        if stop_after == 3 {
            return Ok(Flow::Stopped);
        }
        st.swap_ts = Some(chrono::Utc::now().format("%Y%m%dT%H%M%S%3f").to_string());
        st.save(&staging)?;
    }
    // 4. swap the databases
    let ts = st.swap_ts.clone().unwrap_or_default();
    swap_db(app_data_dir, &staged_db, &ts)?;
    if stop_after == 4 {
        return Ok(Flow::Stopped);
    }
    // 5. settings
    apply_settings(app_data_dir, &staging, &st)?;
    if stop_after == 5 {
        return Ok(Flow::Stopped);
    }
    Ok(Flow::Done)
}

/// Apply a staged Replace restore. `Ok(None)` when there is nothing to do.
pub async fn apply_pending(app_data_dir: &Path) -> Result<Option<RestoreOutcome>, BackupError> {
    apply_until(app_data_dir, u8::MAX).await
}

/// Test hook: stop cleanly after step `stop_after` (1-5), leaving the marker as a crash would.
#[doc(hidden)]
pub async fn apply_until(
    app_data_dir: &Path,
    stop_after: u8,
) -> Result<Option<RestoreOutcome>, BackupError> {
    let marker_path = app_data_dir.join(MARKER_NAME);
    if !marker_path.is_file() {
        return Ok(None);
    }
    let marker: Marker = match fs::read(&marker_path)
        .map_err(BackupError::from)
        .and_then(|b| {
            serde_json::from_slice(&b).map_err(|e| BackupError::Io(format!("restore marker: {e}")))
        }) {
        Ok(m) => m,
        Err(e) => {
            remove_marker(app_data_dir);
            write_result(app_data_dir, false, 0, "", Some(e.to_string()));
            return Err(e);
        }
    };
    let staging = PathBuf::from(&marker.staging_dir);
    if !staging.is_dir() {
        // A previous launch already finished and cleaned up.
        remove_marker(app_data_dir);
        return Ok(None);
    }

    match run_steps(app_data_dir, &marker, stop_after).await {
        Ok(Flow::Stopped) => Ok(None),
        Ok(Flow::Done) => {
            let meetings = marker.manifest.counts.meetings.max(0) as u32;
            let from = marker.manifest.created_at.clone();
            write_result(app_data_dir, true, meetings, &from, None);
            remove_marker(app_data_dir);
            remove_staging(&staging, STAGING_DIR_NAME);
            remove_staging(Path::new(&marker.recordings_staging), RECORDINGS_STAGING_NAME);
            Ok(Some(RestoreOutcome { meetings, from }))
        }
        Err(e) => {
            rollback(app_data_dir, &marker);
            remove_marker(app_data_dir);
            write_result(
                app_data_dir,
                false,
                0,
                &marker.manifest.created_at,
                Some(e.to_string()),
            );
            Err(e)
        }
    }
}

/// After the DB opened: the files moved aside by a restore are no longer needed.
pub fn cleanup_replaced(app_data_dir: &Path) {
    let Ok(entries) = fs::read_dir(app_data_dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(DB_NAME) && name.contains(REPLACED_MARK) {
            let _ = fs::remove_file(e.path());
        }
    }
}
