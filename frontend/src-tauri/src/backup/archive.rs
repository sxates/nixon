//! Writing and reading the backup zip (specs/0092, ADR-0021).

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::manifest::Manifest;
use super::progress::{Phase, Progress};
use super::BackupError;

pub const MANIFEST_NAME: &str = "manifest.json";
const CHUNK: usize = 256 * 1024;
const FOUR_GIB: u64 = 4 * 1024 * 1024 * 1024;

pub type AbortIf = Arc<dyn Fn() -> bool + Send + Sync>;

/// Streams entries into a zip, reporting `Writing` progress per chunk and checking for
/// cancellation between chunks.
pub struct ArchiveWriter<'a> {
    zip: ZipWriter<File>,
    done: u64,
    total: u64,
    progress: &'a dyn Progress,
    cancel: &'a CancellationToken,
    abort_if: Option<&'a AbortIf>,
}

impl<'a> ArchiveWriter<'a> {
    pub fn new(
        file: File,
        total: u64,
        progress: &'a dyn Progress,
        cancel: &'a CancellationToken,
        abort_if: Option<&'a AbortIf>,
    ) -> Self {
        Self {
            zip: ZipWriter::new(file),
            done: 0,
            total,
            progress,
            cancel,
            abort_if,
        }
    }

    pub fn check(&self) -> Result<(), BackupError> {
        if self.cancel.is_cancelled() || self.abort_if.is_some_and(|f| f()) {
            return Err(BackupError::Cancelled);
        }
        Ok(())
    }

    /// Add a small in-memory entry (the manifest).
    pub fn add_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), BackupError> {
        self.check()?;
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        self.zip.start_file(name, opts)?;
        self.zip.write_all(bytes)?;
        Ok(())
    }

    /// Add a file from disk. `media` entries are stored, everything else deflated.
    pub fn add_file(&mut self, src: &Path, name: &str, media: bool) -> Result<(), BackupError> {
        self.check()?;
        let mut f = File::open(src)?;
        let len = f.metadata()?.len();
        let method = if media {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        let mut opts = SimpleFileOptions::default()
            .compression_method(method)
            .unix_permissions(0o644);
        if len >= FOUR_GIB {
            opts = opts.large_file(true);
        }
        self.zip.start_file(name, opts)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            self.zip.write_all(&buf[..n])?;
            self.done += n as u64;
            self.progress
                .report(Phase::Writing, self.done, self.total);
            self.check()?;
        }
        Ok(())
    }

    /// Write the central directory and hand back the file.
    pub fn finish(self) -> Result<File, BackupError> {
        Ok(self.zip.finish()?)
    }
}

/// Read and parse `manifest.json`. `NotABackup` if it is missing or not a manifest.
pub fn read_manifest(zip: &Path) -> Result<Manifest, BackupError> {
    let mut archive = ZipArchive::new(File::open(zip)?).map_err(|_| BackupError::NotABackup)?;
    let entry = archive
        .by_name(MANIFEST_NAME)
        .map_err(|_| BackupError::NotABackup)?;
    // A manifest is tiny; refuse to slurp anything absurd.
    let mut buf = Vec::new();
    entry.take(16 * 1024 * 1024).read_to_end(&mut buf)?;
    serde_json::from_slice(&buf).map_err(|_| BackupError::NotABackup)
}

/// A safe relative path for a zip entry name, or `UnsafeEntry`.
fn safe_relative(name: &str) -> Result<PathBuf, BackupError> {
    let path = Path::new(name);
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => out.push(p),
            _ => return Err(BackupError::UnsafeEntry(name.to_string())),
        }
    }
    if out.as_os_str().is_empty() || name.contains('\\') {
        return Err(BackupError::UnsafeEntry(name.to_string()));
    }
    Ok(out)
}

/// Unpack `db/…` and `settings/…` under `db_settings_dir` and `recordings/<folder>/…` under
/// `recordings_staging/<folder>/…`. With `only_meetings`, only those meetings' folders (per
/// the manifest) are unpacked. Every entry name is validated before anything is written.
pub fn extract(
    zip: &Path,
    db_settings_dir: &Path,
    recordings_staging: &Path,
    only_meetings: Option<&HashSet<String>>,
    progress: &dyn Progress,
    cancel: &CancellationToken,
) -> Result<(), BackupError> {
    let manifest = read_manifest(zip)?;
    let wanted_folders: Option<HashSet<&str>> = only_meetings.map(|set| {
        manifest
            .recordings
            .iter()
            .filter(|r| set.contains(&r.meeting_id))
            .map(|r| r.folder.as_str())
            .collect()
    });
    let mut archive = ZipArchive::new(File::open(zip)?)?;

    // Pass 1: validate every name and pick the entries to write.
    let mut jobs: Vec<(usize, PathBuf, u64)> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let rel = safe_relative(entry.name())?;
        if entry.is_dir() {
            continue;
        }
        let mut comps = rel.components();
        let top = comps.next().map(|c| c.as_os_str().to_string_lossy().into_owned());
        let dest = match top.as_deref() {
            Some("db") | Some("settings") => db_settings_dir.join(&rel),
            Some("recordings") => {
                let Some(folder) = comps.next() else {
                    return Err(BackupError::UnsafeEntry(entry.name().to_string()));
                };
                let folder = folder.as_os_str().to_string_lossy();
                if let Some(w) = &wanted_folders {
                    if !w.contains(folder.as_ref()) {
                        continue;
                    }
                }
                recordings_staging.join(rel.strip_prefix("recordings").unwrap())
            }
            _ => continue,
        };
        jobs.push((i, dest, entry.size()));
    }

    let total: u64 = jobs.iter().map(|j| j.2).sum();
    let mut done = 0u64;
    let mut buf = vec![0u8; CHUNK];
    for (i, dest, _) in jobs {
        if cancel.is_cancelled() {
            return Err(BackupError::Cancelled);
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut entry = archive.by_index(i)?;
        let mut out = File::create(&dest)?;
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            progress.report(Phase::Extracting, done, total);
            if cancel.is_cancelled() {
                return Err(BackupError::Cancelled);
            }
        }
    }
    Ok(())
}
