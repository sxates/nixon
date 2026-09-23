//! `plan_move` against a fake filesystem (specs/0073 task 11).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::plan::*;
use super::roots::MoveRoots;

#[derive(Default)]
struct FakeFs {
    /// folder → meeting id in its metadata (None = pre-0037, no id)
    folders: BTreeMap<PathBuf, Option<String>>,
    /// other paths that exist (loose files)
    files: BTreeSet<PathBuf>,
    /// folders on another volume than the target
    other_volume: BTreeSet<PathBuf>,
    free: u64,
}

impl FakeFs {
    fn folder(mut self, path: &str, id: Option<&str>) -> Self {
        self.folders.insert(path.into(), id.map(str::to_string));
        self
    }
    fn file(mut self, path: &str) -> Self {
        self.files.insert(path.into());
        self
    }
}

impl PlanFs for FakeFs {
    fn canonical(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }
    fn exists(&self, path: &Path) -> bool {
        self.folders.contains_key(path) || self.files.contains(path)
    }
    fn is_owned(&self, path: &Path, meeting_id: &str, known: &[PathBuf]) -> bool {
        match self.folders.get(path) {
            Some(Some(id)) => id == meeting_id,
            Some(None) => known.iter().any(|r| path.starts_with(r) && path != r),
            None => false,
        }
    }
    fn meeting_id_of(&self, path: &Path) -> Option<String> {
        self.folders.get(path).cloned().flatten()
    }
    fn bytes(&self, _: &Path) -> u64 {
        100
    }
    fn same_volume(&self, path: &Path, _: &Path) -> bool {
        !self.other_volume.contains(path)
    }
    fn entries(&self, root: &Path) -> Vec<PathBuf> {
        self.folders
            .keys()
            .chain(self.files.iter())
            .filter(|p| p.parent() == Some(root))
            .cloned()
            .collect()
    }
    fn free_bytes(&self, _: &Path) -> u64 {
        self.free
    }
}

fn roots() -> MoveRoots {
    MoveRoots {
        target: "/new".into(),
        sources: vec!["/old".into()],
        known: vec!["/new".into(), "/old".into(), "/prod".into()],
        protected: vec!["/prod".into()],
        removable: vec!["/old".into()],
    }
}

fn row(id: &str, folder: Option<&str>) -> PlanRow {
    PlanRow {
        meeting_id: id.into(),
        title: format!("Meeting {id}"),
        folder_path: folder.map(str::to_string),
    }
}

#[test]
fn meetings_already_under_the_target_are_left_out() {
    let fs = FakeFs::default()
        .folder("/new/A", Some("a"))
        .folder("/old/B", Some("b"));
    let plan = plan_move(
        &[row("a", Some("/new/A")), row("b", Some("/old/B"))],
        &[],
        &roots(),
        &fs,
    );
    assert_eq!(plan.meetings, 1);
    assert_eq!(plan.units[0].src, PathBuf::from("/old/B"));
    assert_eq!(plan.units[0].dst, PathBuf::from("/new/B"));
}

#[test]
fn a_shared_folder_moves_once_and_updates_every_row() {
    // A continued meeting: two rows, one folder whose metadata names the first.
    let fs = FakeFs::default().folder("/old/Sync", Some("a"));
    let plan = plan_move(
        &[row("a", Some("/old/Sync")), row("b", Some("/old/Sync"))],
        &[],
        &roots(),
        &fs,
    );
    assert_eq!(plan.folders, 1);
    assert_eq!(plan.meetings, 2);
    assert_eq!(plan.units[0].update_ids, vec!["a", "b"]);
    assert_eq!(plan.not_owned, 0);
}

#[test]
fn a_name_collision_gets_a_numbered_suffix() {
    let fs = FakeFs::default()
        .folder("/old/Sync", Some("a"))
        .folder("/new/Sync", Some("other"))
        .folder("/elsewhere/Sync", Some("b"));
    let plan = plan_move(
        &[
            row("a", Some("/old/Sync")),
            row("b", Some("/elsewhere/Sync")),
        ],
        &[],
        &roots(),
        &fs,
    );
    let dsts: BTreeSet<_> = plan.units.iter().map(|u| u.dst.clone()).collect();
    assert_eq!(
        dsts,
        BTreeSet::from([
            PathBuf::from("/new/Sync (2)"),
            PathBuf::from("/new/Sync (3)")
        ])
    );
}

#[test]
fn an_earlier_copy_of_the_same_meeting_is_reused_not_suffixed() {
    let fs = FakeFs::default()
        .folder("/old/Sync", Some("a"))
        .folder("/new/Sync", Some("a"));
    let plan = plan_move(&[row("a", Some("/old/Sync"))], &[], &roots(), &fs);
    assert_eq!(plan.units[0].dst, PathBuf::from("/new/Sync"));
}

#[test]
fn every_left_out_meeting_is_classified() {
    let fs = FakeFs::default()
        .folder("/elsewhere/E", Some("e"))
        .folder("/old/Theirs", Some("someone-else"))
        .folder("/prod/P", Some("p"))
        .folder("/old/Loose", None)
        .file("/old/notes.txt")
        .file("/old/.DS_Store-is-filtered-by-real-fs-only")
        .folder("/prod/ProdLoose", None);
    let plan = plan_move(
        &[
            row("e", Some("/elsewhere/E")),
            row("gone", Some("/old/Gone")),
            row("n", Some("/old/Theirs")),
            row("p", Some("/prod/P")),
            row("null", None),
        ],
        &[],
        &roots(),
        &fs,
    );
    assert_eq!(plan.meetings, 1);
    assert_eq!(plan.elsewhere, 1);
    assert!(plan.units[0].elsewhere);
    assert_eq!(plan.missing, 1);
    assert_eq!(plan.not_owned, 1);
    assert_eq!(plan.protected, 1);
    // /old/Loose, /old/notes.txt and the extra file; never anything in /prod.
    assert_eq!(plan.unreferenced, 3);
}

#[test]
fn interrupted_recordings_move_without_a_row_update() {
    let fs = FakeFs::default()
        .folder("/old/Crashed", Some("c"))
        .folder("/prod/CrashedProd", Some("d"));
    let plan = plan_move(
        &[row("c", None), row("d", None)],
        &[
            InterruptedFolder {
                meeting_id: "c".into(),
                folder: "/old/Crashed".into(),
            },
            InterruptedFolder {
                meeting_id: "d".into(),
                folder: "/prod/CrashedProd".into(),
            },
        ],
        &roots(),
        &fs,
    );
    assert_eq!(
        plan.units.len(),
        1,
        "never an interrupted folder in a protected root"
    );
    assert_eq!(plan.units[0].lease_ids, vec!["c"]);
    assert!(plan.units[0].update_ids.is_empty());
    assert_eq!(plan.units[0].title, "Meeting c");
    assert_eq!(plan.unreferenced, 0, "an interrupted folder isn't loose");
}

#[test]
fn free_space_is_needed_only_for_cross_volume_folders() {
    let mut fs = FakeFs::default()
        .folder("/old/A", Some("a"))
        .folder("/old/B", Some("b"));
    fs.free = 0;
    let rows = [row("a", Some("/old/A")), row("b", Some("/old/B"))];
    let plan = plan_move(&rows, &[], &roots(), &fs);
    assert!(plan.enough_space, "a rename needs no space");
    assert_eq!(plan.same_volume_count, 2);

    fs.other_volume.insert("/old/B".into());
    let plan = plan_move(&rows, &[], &roots(), &fs);
    assert_eq!(plan.cross_volume_count, 1);
    assert_eq!(plan.cross_volume_bytes, 100);
    assert!(!plan.enough_space);

    fs.free = 100 + SPACE_MARGIN_BYTES;
    assert!(plan_move(&rows, &[], &roots(), &fs).enough_space);
}

#[test]
fn staging_sits_next_to_the_destination() {
    assert_eq!(
        staging_path(Path::new("/new/Weekly sync")),
        PathBuf::from("/new/.nixon-moving-Weekly sync")
    );
}
