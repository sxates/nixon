//! Export: snapshot + settings + meeting folders into one `.zip` (specs/0092, "Export").

use std::collections::HashSet;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

use super::archive::{AbortIf, ArchiveWriter, MANIFEST_NAME};
use super::files::{folder_entries, settings_entries};
use super::manifest::{Manifest, RecordingRef, FORMAT};
use super::progress::{Phase, Progress};
use super::snapshot::snapshot_db;
use super::BackupError;
use crate::audio::folder_lease::{self, LeaseHolder};
use crate::audio::lifecycle::is_media_file;

pub struct ExportOptions {
    pub include_audio: bool,
    pub app_data_dir: PathBuf,
    /// Checked between entries; true aborts with `Cancelled` (the scheduler passes
    /// "a recording started").
    pub abort_if: Option<AbortIf>,
    /// Only meeting folders under one of these roots are exported. Empty means no restriction.
    pub allowed_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ExportSummary {
    pub path: PathBuf,
    pub bytes: u64,
    pub manifest: Manifest,
}

/// `Nixon Backup YYYY-MM-DD HHMM` in local time (no extension).
pub(crate) fn default_stem() -> String {
    format!("{MANUAL_PREFIX} {}", chrono::Local::now().format("%Y-%m-%d %H%M"))
}

/// File-name prefixes: a backup made by hand, and one the daily schedule made (only those are
/// pruned).
pub const MANUAL_PREFIX: &str = "Nixon Backup";
pub const AUTO_PREFIX: &str = "Nixon Auto Backup";

/// Export into `dest_dir` as `Nixon Backup YYYY-MM-DD HHMM.zip` (suffixed ` 2`, ` 3`… if taken).
pub async fn export_to(
    pool: &SqlitePool,
    dest_dir: &Path,
    opts: &ExportOptions,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<ExportSummary, BackupError> {
    export_named(pool, dest_dir, &default_stem(), opts, progress, cancel).await
}

/// Export into `dest_dir` as `Nixon Auto Backup YYYY-MM-DD HHMM.zip` (the daily schedule).
pub async fn export_auto_to(
    pool: &SqlitePool,
    dest_dir: &Path,
    opts: &ExportOptions,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<ExportSummary, BackupError> {
    let stem = format!("{AUTO_PREFIX} {}", chrono::Local::now().format("%Y-%m-%d %H%M"));
    export_named(pool, dest_dir, &stem, opts, progress, cancel).await
}

async fn export_named(
    pool: &SqlitePool,
    dest_dir: &Path,
    stem: &str,
    opts: &ExportOptions,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<ExportSummary, BackupError> {
    std::fs::create_dir_all(dest_dir)?;
    let mut dest = dest_dir.join(format!("{stem}.zip"));
    let mut n = 2;
    while dest.exists() {
        dest = dest_dir.join(format!("{stem} {n}.zip"));
        n += 1;
    }
    export_to_file(pool, &dest, opts, progress, cancel).await
}

/// Export to an explicit path (the save dialog). Written as `<name>.partial`, then renamed.
pub async fn export_to_file(
    pool: &SqlitePool,
    dest_file: &Path,
    opts: &ExportOptions,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<ExportSummary, BackupError> {
    let mut partial_name = dest_file.file_name().unwrap_or_default().to_os_string();
    partial_name.push(".partial");
    let partial = dest_file.with_file_name(partial_name);
    let snapshot = opts
        .app_data_dir
        .join(format!(".backup-snapshot-{}.sqlite", uuid::Uuid::new_v4()));

    let result = run(pool, dest_file, &partial, &snapshot, opts, progress, cancel).await;
    let _ = std::fs::remove_file(&snapshot);
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// One meeting folder to archive.
struct Planned {
    meeting_id: String,
    source: PathBuf,
    name: String,
}

fn under_allowed(path: &Path, roots: &[PathBuf]) -> bool {
    if roots.is_empty() {
        return true;
    }
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    roots.iter().any(|r| {
        let root = std::fs::canonicalize(r).unwrap_or_else(|_| r.clone());
        real.starts_with(root)
    })
}

fn usable_folder(path: &Path, roots: &[PathBuf]) -> bool {
    path.is_dir() && under_allowed(path, roots)
}

/// Meetings whose folder exists and is inside a known root, with unique in-archive names.
fn plan_folders(recordings: &[(String, Option<String>)], roots: &[PathBuf]) -> Vec<Planned> {
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (id, folder) in recordings {
        let Some(folder) = folder else { continue };
        let source = PathBuf::from(folder);
        if !usable_folder(&source, roots) {
            continue;
        }
        let Some(last) = source.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let mut name = last.clone();
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = format!("{last} ({n})");
            n += 1;
        }
        out.push(Planned {
            meeting_id: id.clone(),
            source,
            name,
        });
    }
    out
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

async fn run(
    pool: &SqlitePool,
    dest_file: &Path,
    partial: &Path,
    snapshot: &Path,
    opts: &ExportOptions,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<ExportSummary, BackupError> {
    let cancelled = || {
        cancel.is_cancelled() || opts.abort_if.as_ref().is_some_and(|f| f())
    };
    if cancelled() {
        return Err(BackupError::Cancelled);
    }
    // Create the destination first so an unwritable folder fails before any snapshot work.
    let out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(partial)?;

    progress.report(Phase::Snapshot, 0, 1);
    let info = snapshot_db(pool, snapshot).await?;
    progress.report(Phase::Snapshot, 1, 1);
    if cancelled() {
        return Err(BackupError::Cancelled);
    }

    let settings = settings_entries(&opts.app_data_dir);
    let planned = plan_folders(&info.recordings, &opts.allowed_roots);

    let mut total = file_size(snapshot);
    total += settings.iter().map(|(p, _)| file_size(p)).sum::<u64>();
    for p in &planned {
        total += folder_entries(&p.source, opts.include_audio)
            .iter()
            .map(|f| file_size(f))
            .sum::<u64>();
    }

    let mut w = ArchiveWriter::new(out, total, progress, cancel, opts.abort_if.as_ref());
    w.add_file(snapshot, "db/meeting_minutes.sqlite", false)?;
    for (src, name) in &settings {
        w.add_file(src, &format!("settings/{name}"), false)?;
    }

    let mut recordings = Vec::new();
    for p in &planned {
        // Wait in short slices so cancel / abort_if are honored while another job holds it.
        let _lease = loop {
            if cancelled() {
                return Err(BackupError::Cancelled);
            }
            if let Ok(l) = folder_lease::acquire_within(
                &p.meeting_id,
                LeaseHolder::Backup,
                std::time::Duration::from_millis(200),
            )
            .await
            {
                break l;
            }
        };
        // The folder may have moved while we waited; trust the live path if it is usable.
        let live: Option<String> =
            sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
                .bind(&p.meeting_id)
                .fetch_optional(pool)
                .await?
                .flatten();
        let source = match live.map(PathBuf::from) {
            Some(l) if l != p.source && usable_folder(&l, &opts.allowed_roots) => l,
            _ => p.source.clone(),
        };
        if !source.is_dir() {
            continue;
        }
        for f in folder_entries(&source, opts.include_audio) {
            let Some(fname) = f.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            w.add_file(&f, &format!("recordings/{}/{fname}", p.name), is_media_file(&f))?;
        }
        recordings.push(RecordingRef {
            meeting_id: p.meeting_id.clone(),
            folder: p.name.clone(),
        });
    }

    let manifest = Manifest {
        format: FORMAT,
        app: "Nixon".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: info.schema_version,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        includes_audio: opts.include_audio,
        counts: info.counts,
        recordings,
    };
    let json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| BackupError::Io(format!("manifest: {e}")))?;
    w.add_bytes(MANIFEST_NAME, &json)?;

    let file = w.finish()?;
    let _ = file.sync_all();
    drop(file);
    std::fs::rename(partial, dest_file)?;
    let bytes = file_size(dest_file);
    Ok(ExportSummary {
        path: dest_file.to_path_buf(),
        bytes,
        manifest,
    })
}
