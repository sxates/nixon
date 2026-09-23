//! Creating (and un-creating) the folder a move or gather writes into.
//!
//! A refused change of folder must leave no trace, including the intermediate folders a
//! `create_dir_all` made. And the startup gather must never create the current folder on
//! its own when that folder lives somewhere that may just be unplugged (an external or
//! network volume's mount path): a stray local folder there would take new recordings.

use std::path::{Path, PathBuf};

/// A target folder this call created, remembered so a refusal can remove it again.
#[derive(Debug)]
pub struct CreatedTarget {
    leaf: PathBuf,
    /// The topmost folder this call created, or `None` when the target already existed.
    top: Option<PathBuf>,
}

impl CreatedTarget {
    /// Create `target` and any missing parents.
    pub fn create(target: &Path) -> Result<Self, String> {
        let top = target
            .ancestors()
            .take_while(|p| !p.exists())
            .last()
            .map(Path::to_path_buf);
        std::fs::create_dir_all(target)
            .map_err(|e| format!("Couldn't create {}: {e}", target.display()))?;
        Ok(Self {
            leaf: target.to_path_buf(),
            top,
        })
    }

    /// Remove every folder [`create`](Self::create) made, leaf first, each only while it is
    /// still empty. A folder that existed before is never touched.
    pub fn undo(&self) {
        let Some(top) = &self.top else { return };
        for dir in self.leaf.ancestors() {
            if std::fs::remove_dir(dir).is_err() || dir == top.as_path() {
                return;
            }
        }
    }
}

/// What the startup gather may do with the current recordings folder.
#[derive(Debug, PartialEq, Eq)]
pub enum StartupRoot {
    /// It exists.
    Ready,
    /// It is missing but is the local default folder, which is safe to create.
    CreateDefault,
    /// It is missing and is a folder the owner chose; its drive may be unplugged. The
    /// string is user-facing.
    Unavailable(String),
}

/// Decide, without touching the disk, whether the startup gather can use `root`.
pub fn startup_root(root: &Path, default_root: &Path) -> StartupRoot {
    if root.is_dir() {
        StartupRoot::Ready
    } else if root == default_root {
        StartupRoot::CreateDefault
    } else {
        StartupRoot::Unavailable(format!(
            "Your recordings folder ({}) isn't available. If it's on a drive, connect it and \
             restart Nixon, or choose another folder.",
            root.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_removes_every_folder_the_create_made_and_nothing_older() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a/b/c");
        let created = CreatedTarget::create(&target).unwrap();
        assert!(target.is_dir());
        created.undo();
        assert!(
            !tmp.path().join("a").exists(),
            "intermediate folders are gone"
        );
        assert!(tmp.path().exists(), "the folder that existed before stays");
    }

    #[test]
    fn undo_leaves_a_target_that_already_existed_or_is_no_longer_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let existing = tmp.path().join("kept");
        std::fs::create_dir(&existing).unwrap();
        CreatedTarget::create(&existing).unwrap().undo();
        assert!(existing.is_dir(), "an existing target is never removed");

        let target = tmp.path().join("x/y");
        let created = CreatedTarget::create(&target).unwrap();
        std::fs::write(target.join("file"), b"1").unwrap();
        created.undo();
        assert!(target.join("file").exists(), "a non-empty folder stays");
    }

    #[test]
    fn a_missing_chosen_root_is_reported_not_created() {
        let tmp = tempfile::tempdir().unwrap();
        let default_root = tmp.path().join("default");
        let chosen = tmp.path().join("Volumes/External/recordings");
        assert!(matches!(
            startup_root(&chosen, &default_root),
            StartupRoot::Unavailable(r) if r.contains("isn't available")
        ));
        assert!(!chosen.exists(), "deciding never creates the folder");
        assert_eq!(
            startup_root(&default_root, &default_root),
            StartupRoot::CreateDefault
        );
        assert_eq!(startup_root(tmp.path(), &default_root), StartupRoot::Ready);
    }
}
