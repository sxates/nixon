//! The filesystem half of the mover: the real [`PlanFs`], the synced copy, the verify, and
//! the guarded removals (source folder, emptied old roots, stale staging folders).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use super::plan::{PlanFs, STAGING_PREFIX};
use super::roots::MoveRoots;
use crate::audio::meeting_folder::{canonical_or_lexical, folder_meeting_id, is_meeting_folder_in};

/// Finder's folder-view file. The only entry that doesn't keep an old root alive.
const DS_STORE: &str = ".DS_Store";

/// `errno` for a rename across filesystems (the same value on macOS and Linux).
const EXDEV: i32 = 18;

/// Is `e` the "can't rename across volumes" error?
pub fn is_cross_device(e: &io::Error) -> bool {
    e.raw_os_error() == Some(EXDEV)
}

/// The nearest existing ancestor of `path` (itself when it exists).
fn nearest_existing(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|p| p.exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(unix)]
fn device_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(nearest_existing(path))
        .ok()
        .map(|m| m.dev())
}

#[cfg(not(unix))]
fn device_of(_path: &Path) -> Option<u64> {
    None
}

/// Is `path` the root of a volume (`/`, a mount point)?
fn is_volume_root(path: &Path) -> bool {
    match path.parent() {
        None => true,
        Some(parent) => match (device_of(path), device_of(parent)) {
            (Some(own), Some(up)) => own != up,
            _ => false,
        },
    }
}

/// Every file under `root`, relative path → size. `.DS_Store` is ignored (Finder may add
/// one to either side at any time).
pub fn tree_sizes(root: &Path) -> io::Result<BTreeMap<PathBuf, u64>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, u64>) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(base, &path, out)?;
            } else if entry.file_name() != DS_STORE {
                let rel = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
                out.insert(rel, std::fs::metadata(&path)?.len());
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

/// The real filesystem.
pub struct RealFs;

impl PlanFs for RealFs {
    fn canonical(&self, path: &Path) -> PathBuf {
        canonical_or_lexical(path)
    }
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn is_owned(&self, path: &Path, meeting_id: &str, known_roots: &[PathBuf]) -> bool {
        is_meeting_folder_in(path, meeting_id, known_roots)
    }
    fn meeting_id_of(&self, path: &Path) -> Option<String> {
        folder_meeting_id(path)
    }
    fn bytes(&self, path: &Path) -> u64 {
        tree_sizes(path)
            .map(|m| m.values().sum())
            .unwrap_or_default()
    }
    fn same_volume(&self, path: &Path, target: &Path) -> bool {
        match (device_of(path), device_of(target)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
    fn entries(&self, root: &Path) -> Vec<PathBuf> {
        let Ok(read) = std::fs::read_dir(root) else {
            return Vec::new();
        };
        read.flatten()
            .filter(|e| e.file_name() != DS_STORE)
            .map(|e| canonical_or_lexical(&e.path()))
            .collect()
    }
    fn free_bytes(&self, target: &Path) -> u64 {
        crate::onboarding_disk::free_space_for(&nearest_existing(target))
    }
}

/// Copy `src` into `dst` (created), syncing every file to disk. `on_file` gets each file's
/// size as it lands, for progress.
pub fn copy_synced(src: &Path, dst: &Path, on_file: &dyn Fn(u64)) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_synced(&from, &to, on_file)?;
        } else {
            let n = std::fs::copy(&from, &to)?;
            std::fs::File::open(&to)?.sync_all()?;
            on_file(n);
        }
    }
    Ok(())
}

/// Is `copy` a complete copy of `src`? Same relative file set, same byte sizes, and (when
/// the source has one) a `metadata.json` that parses. Returns the first difference.
pub fn verify_copy(src: &Path, copy: &Path) -> Result<(), String> {
    let want = tree_sizes(src).map_err(|e| format!("couldn't read the original: {e}"))?;
    let got = tree_sizes(copy).map_err(|e| format!("couldn't read the copy: {e}"))?;
    for (rel, size) in &want {
        match got.get(rel) {
            None => return Err(format!("{} is missing from the copy", rel.display())),
            Some(s) if s != size => {
                return Err(format!(
                    "{} is {s} bytes in the copy but {size} in the original",
                    rel.display()
                ))
            }
            _ => {}
        }
    }
    if let Some(extra) = got.keys().find(|k| !want.contains_key(*k)) {
        return Err(format!(
            "{} is in the copy but not the original",
            extra.display()
        ));
    }
    if want.contains_key(Path::new("metadata.json")) {
        let bytes = std::fs::read(copy.join("metadata.json"))
            .map_err(|e| format!("couldn't read the copied metadata.json: {e}"))?;
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| format!("the copied metadata.json doesn't parse: {e}"))?;
    }
    Ok(())
}

/// May the mover `remove_dir_all` this source folder? Never a root (known, protected,
/// target), an ancestor of the target, the home folder or a volume root, and never
/// anything inside a protected root.
pub fn safe_to_remove_source(src: &Path, roots: &MoveRoots) -> bool {
    let src = canonical_or_lexical(src);
    let home = dirs::home_dir().map(|h| canonical_or_lexical(&h));
    src.is_dir()
        && !roots.is_protected(&src)
        && !roots.known.contains(&src)
        && !roots.target.starts_with(&src)
        && home.as_ref() != Some(&src)
        && !is_volume_root(&src)
}

/// Remove every removable root the run emptied (owner decision Q2). "Empty" ignores
/// `.DS_Store`; the root itself goes with a non-recursive `remove_dir`, so a file the user
/// drops in at the wrong moment makes the removal fail safe. Only roots the run moved a
/// folder out of are considered, and never a protected one. Returns the removed roots.
pub fn remove_emptied_roots(moved_from: &[PathBuf], roots: &MoveRoots) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for root in &roots.removable {
        if roots.is_protected(root) || *root == roots.target || roots.target.starts_with(root) {
            continue;
        }
        if !moved_from.iter().any(|p| canonical_or_lexical(p) == *root) {
            continue;
        }
        let Ok(read) = std::fs::read_dir(root) else {
            continue;
        };
        let names: Vec<_> = read.flatten().map(|e| e.file_name()).collect();
        if names.iter().any(|n| n != DS_STORE) {
            log::info!(
                "recordings move: keeping {} (it still holds other files)",
                root.display()
            );
            continue;
        }
        let _ = std::fs::remove_file(root.join(DS_STORE));
        match std::fs::remove_dir(root) {
            Ok(()) => {
                log::info!(
                    "recordings move: removed the emptied folder {}",
                    root.display()
                );
                removed.push(root.clone());
            }
            Err(e) => log::warn!("recordings move: kept {}: {e}", root.display()),
        }
    }
    removed
}

/// Remove leftover staging folders directly under `root`. Safe only when no move is
/// running: a staging folder is always a copy whose source still exists.
pub fn sweep_staging(root: &Path) {
    let Ok(read) = std::fs::read_dir(root) else {
        return;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(STAGING_PREFIX) && entry.path().is_dir() {
            match std::fs::remove_dir_all(entry.path()) {
                Ok(()) => log::info!("recordings move: removed a leftover copy {name}"),
                Err(e) => log::warn!("recordings move: couldn't remove {name}: {e}"),
            }
        }
    }
}
