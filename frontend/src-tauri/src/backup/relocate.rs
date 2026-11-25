//! Moves restored recording folders from the staging directory into the recordings root.

use std::fs;
use std::path::{Component, Path, PathBuf};

use super::manifest::RecordingRef;
use super::BackupError;

/// How many `<name> (n)` suffixes are tried before giving up.
const MAX_COLLISION_SUFFIX: u32 = 1000;

/// A recording folder moved into the root. `original` is its name under staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub meeting_id: String,
    pub path: PathBuf,
    pub original: String,
    /// The folder already on disk was kept (same meeting); nothing was moved, so undo skips it.
    pub kept: bool,
}

fn is_plain_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    )
}

/// First free `root/<name>`, `root/<name> (2)`, … `root/<name> (1000)`.
fn free_target(root: &Path, name: &str) -> Result<PathBuf, BackupError> {
    let first = root.join(name);
    if !first.exists() {
        return Ok(first);
    }
    for n in 2..=MAX_COLLISION_SUFFIX {
        let candidate = root.join(format!("{name} ({n})"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(BackupError::Io(format!(
        "no free folder name for \"{name}\" in the recordings folder"
    )))
}

/// Whether `dir/metadata.json` names `meeting_id` (the same meeting's own folder).
fn is_folder_of(dir: &Path, meeting_id: &str) -> bool {
    let Ok(text) = fs::read_to_string(dir.join("metadata.json")) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("meeting_id").and_then(|m| m.as_str()).map(|m| m == meeting_id))
        .unwrap_or(false)
}

fn place_one(staging: &Path, root: &Path, r: &RecordingRef) -> Result<Option<Placed>, BackupError> {
    if !is_plain_name(&r.folder) {
        return Err(BackupError::Io(format!(
            "invalid recording folder name \"{}\"",
            r.folder
        )));
    }
    let from = staging.join(&r.folder);
    if !from.is_dir() {
        return Ok(None);
    }
    let existing = root.join(&r.folder);
    if existing.is_dir() && is_folder_of(&existing, &r.meeting_id) {
        // Same meeting, same Mac: keep the folder (and any audio) that is already here.
        fs::remove_dir_all(&from)?;
        return Ok(Some(Placed {
            meeting_id: r.meeting_id.clone(),
            path: existing,
            original: r.folder.clone(),
            kept: true,
        }));
    }
    let to = free_target(root, &r.folder)?;
    fs::rename(&from, &to)?;
    Ok(Some(Placed {
        meeting_id: r.meeting_id.clone(),
        path: to,
        original: r.folder.clone(),
        kept: false,
    }))
}

/// Renames each staged folder into `root`, suffixing ` (n)` on a name clash. Refs whose staged
/// folder is missing are skipped. On failure, folders already placed go back to staging.
/// Removes `staging` afterwards if it is empty.
pub fn place_folders(
    staging: &Path,
    root: &Path,
    refs: &[RecordingRef],
) -> Result<Vec<Placed>, BackupError> {
    fs::create_dir_all(root)?;
    let mut placed = Vec::new();
    for r in refs {
        match place_one(staging, root, r) {
            Ok(Some(p)) => placed.push(p),
            Ok(None) => {}
            Err(e) => {
                let _ = undo(&placed, staging);
                return Err(e);
            }
        }
    }
    let _ = fs::remove_dir(staging); // only succeeds when empty
    Ok(placed)
}

/// Moves every placed folder back to `staging/<original name>`. Best effort: tries all and
/// returns the first error.
pub fn undo(placed: &[Placed], staging: &Path) -> Result<(), BackupError> {
    let mut first_err = None;
    if let Err(e) = fs::create_dir_all(staging) {
        first_err = Some(BackupError::from(e));
    }
    for p in placed.iter().filter(|p| !p.kept) {
        if let Err(e) = fs::rename(&p.path, staging.join(&p.original)) {
            first_err.get_or_insert(BackupError::from(e));
        }
    }
    first_err.map_or(Ok(()), Err)
}
