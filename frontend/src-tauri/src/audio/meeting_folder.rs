//! specs/0073 W1 — what counts as a meeting's recording folder, wherever it lives.
//!
//! Before 0073 the app trusted "is under the CURRENT recordings root" as proof that a path
//! was a meeting folder. That stopped being true the moment the root changed: a meeting left
//! in the old root could no longer be deleted (its folder was skipped as "outside the
//! recordings root") or recovered. [`is_meeting_folder`] replaces that test. It is the
//! ownership check used by delete, and later by the recordings mover.

use std::path::{Path, PathBuf};

/// Canonicalize when possible (defeats `..` and symlinked roots); keep the lexical path for
/// a location that doesn't exist.
pub fn canonical_or_lexical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Is `path` the root of a volume (`/`, `/Volumes/X`, any mount point)?
fn is_volume_root(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return true; // "/"
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(path), std::fs::metadata(parent)) {
            (Ok(own), Ok(up)) => own.dev() != up.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        false
    }
}

/// The `meeting_id` recorded in a folder's `metadata.json` (written since specs/0037).
/// `None` when there is no metadata, it doesn't parse, or it carries no id.
pub fn folder_meeting_id(folder: &Path) -> Option<String> {
    let bytes = std::fs::read(folder.join("metadata.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("meeting_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Is `path` the recording folder of meeting `meeting_id`, given the recordings roots Nixon
/// knows about? True iff all of:
///
/// - `path` is a directory,
/// - it is not one of `known_roots`, the home directory, `/`, or a volume root,
/// - and either its `metadata.json` names `meeting_id`, or (a folder from before
///   specs/0037, with no id in its metadata) it lies inside one of `known_roots`.
///
/// A folder whose metadata names a DIFFERENT meeting is never this meeting's, wherever it is.
pub fn is_meeting_folder_in(path: &Path, meeting_id: &str, known_roots: &[PathBuf]) -> bool {
    if !path.is_dir() {
        return false;
    }
    let path = canonical_or_lexical(path);
    let roots: Vec<PathBuf> = known_roots
        .iter()
        .map(|r| canonical_or_lexical(r))
        .collect();
    let home = dirs::home_dir().map(|h| canonical_or_lexical(&h));

    if roots.contains(&path) || home.as_ref() == Some(&path) || is_volume_root(&path) {
        return false;
    }
    match folder_meeting_id(&path) {
        Some(id) => id == meeting_id,
        None => roots.iter().any(|root| path.starts_with(root)),
    }
}

/// [`is_meeting_folder_in`] against every recordings root Nixon knows about.
pub fn is_meeting_folder(path: &Path, meeting_id: &str) -> bool {
    is_meeting_folder_in(
        path,
        meeting_id,
        &super::recording_preferences::known_recording_roots(),
    )
}

/// What [`remove_meeting_folder_in`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum RemoveOutcome {
    Removed,
    AlreadyGone,
    /// The path failed the ownership check and was left alone.
    NotOwned,
    Failed(String),
}

/// Remove meeting `meeting_id`'s recording folder at `folder_path`, but only if it passes
/// [`is_meeting_folder_in`]. Never runs `remove_dir_all` on an arbitrary stored path.
pub fn remove_meeting_folder_in(
    folder_path: &Path,
    meeting_id: &str,
    known_roots: &[PathBuf],
) -> RemoveOutcome {
    if !folder_path.exists() {
        return RemoveOutcome::AlreadyGone;
    }
    if !is_meeting_folder_in(folder_path, meeting_id, known_roots) {
        return RemoveOutcome::NotOwned;
    }
    match std::fs::remove_dir_all(canonical_or_lexical(folder_path)) {
        Ok(()) => RemoveOutcome::Removed,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => RemoveOutcome::AlreadyGone,
        Err(e) => RemoveOutcome::Failed(e.to_string()),
    }
}

/// Recursively copy `src` into `dst`, creating `dst` and every subdirectory.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn folder_with_id(parent: &Path, name: &str, id: Option<&str>) -> PathBuf {
        let folder = parent.join(name);
        fs::create_dir_all(&folder).unwrap();
        let meta = match id {
            Some(id) => format!(r#"{{"meeting_id":"{id}","status":"completed"}}"#),
            None => r#"{"status":"completed"}"#.to_string(),
        };
        fs::write(folder.join("metadata.json"), meta).unwrap();
        folder
    }

    #[test]
    fn a_folder_naming_the_meeting_is_owned_even_outside_every_root() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let folder = folder_with_id(elsewhere.path(), "Sync", Some("m-1"));
        assert!(is_meeting_folder_in(
            &folder,
            "m-1",
            &[root.path().to_path_buf()]
        ));
    }

    #[test]
    fn a_folder_naming_another_meeting_is_never_owned() {
        let root = tempfile::tempdir().unwrap();
        let folder = folder_with_id(root.path(), "Sync", Some("m-other"));
        assert!(!is_meeting_folder_in(
            &folder,
            "m-1",
            &[root.path().to_path_buf()]
        ));
    }

    #[test]
    fn a_pre_0037_folder_is_owned_only_inside_a_known_root() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let inside = folder_with_id(root.path(), "Old", None);
        let outside = folder_with_id(elsewhere.path(), "Old", None);
        let roots = [root.path().to_path_buf()];
        assert!(is_meeting_folder_in(&inside, "m-1", &roots));
        assert!(!is_meeting_folder_in(&outside, "m-1", &roots));
    }

    #[test]
    fn a_root_home_and_slash_are_never_meeting_folders() {
        let root = tempfile::tempdir().unwrap();
        // Even a root that happens to carry a matching metadata.json.
        fs::write(root.path().join("metadata.json"), r#"{"meeting_id":"m-1"}"#).unwrap();
        let roots = [root.path().to_path_buf()];
        assert!(!is_meeting_folder_in(root.path(), "m-1", &roots));
        assert!(!is_meeting_folder_in(Path::new("/"), "m-1", &roots));
        let home = dirs::home_dir().unwrap();
        assert!(!is_meeting_folder_in(&home, "m-1", std::slice::from_ref(&home)));
        assert!(!is_meeting_folder_in(&home, "m-1", &[]));
    }

    #[test]
    fn a_missing_path_or_a_file_is_not_a_meeting_folder() {
        let root = tempfile::tempdir().unwrap();
        let roots = [root.path().to_path_buf()];
        assert!(!is_meeting_folder_in(
            &root.path().join("gone"),
            "m-1",
            &roots
        ));
        let file = root.path().join("audio.mp4");
        fs::write(&file, b"x").unwrap();
        assert!(!is_meeting_folder_in(&file, "m-1", &roots));
    }

    #[test]
    fn traversal_out_of_a_root_is_resolved_before_the_check() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = folder_with_id(elsewhere.path(), "Old", None);
        let sneaky = root
            .path()
            .join("..")
            .join(elsewhere.path().file_name().unwrap())
            .join("Old");
        assert!(sneaky.is_dir());
        assert!(!is_meeting_folder_in(
            &sneaky,
            "m-1",
            &[root.path().to_path_buf()]
        ));
        assert!(outside.is_dir());
    }

    #[test]
    fn remove_deletes_an_owned_folder_and_refuses_anything_else() {
        let root = tempfile::tempdir().unwrap();
        let roots = [root.path().to_path_buf()];
        let mine = folder_with_id(root.path(), "Mine", Some("m-1"));
        let theirs = folder_with_id(root.path(), "Theirs", Some("m-2"));

        assert_eq!(
            remove_meeting_folder_in(&theirs, "m-1", &roots),
            RemoveOutcome::NotOwned
        );
        assert!(theirs.is_dir());
        assert_eq!(
            remove_meeting_folder_in(root.path(), "m-1", &roots),
            RemoveOutcome::NotOwned
        );
        assert_eq!(
            remove_meeting_folder_in(&mine, "m-1", &roots),
            RemoveOutcome::Removed
        );
        assert!(!mine.exists());
        assert_eq!(
            remove_meeting_folder_in(&mine, "m-1", &roots),
            RemoveOutcome::AlreadyGone
        );
    }

    #[test]
    fn copy_dir_recursive_copies_nested_files() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("a/b")).unwrap();
        fs::write(src.path().join("top.txt"), b"1").unwrap();
        fs::write(src.path().join("a/b/deep.bin"), b"22").unwrap();
        let out = dst.path().join("copy");
        copy_dir_recursive(src.path(), &out).unwrap();
        assert_eq!(fs::read(out.join("top.txt")).unwrap(), b"1");
        assert_eq!(fs::read(out.join("a/b/deep.bin")).unwrap(), b"22");
    }
}
