//! specs/0092 task 4: placing staged recording folders into the recordings root.

use std::fs;
use std::path::Path;

use app_lib::backup::manifest::RecordingRef;
use app_lib::backup::relocate::{place_folders, undo};
use app_lib::backup::BackupError;

fn stage(staging: &Path, folder: &str, file: &str, body: &str) {
    let dir = staging.join(folder);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(file), body).unwrap();
}

fn rref(id: &str, folder: &str) -> RecordingRef {
    RecordingRef {
        meeting_id: id.into(),
        folder: folder.into(),
    }
}

#[test]
fn places_and_suffixes_on_collision() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("A")).unwrap();
    fs::write(root.join("A/old.txt"), "old").unwrap();
    fs::create_dir_all(root.join("A (2)")).unwrap();
    stage(&staging, "A", "new.txt", "new");
    stage(&staging, "B", "b.txt", "b");

    let placed = place_folders(&staging, &root, &[rref("m1", "A"), rref("m2", "B")]).unwrap();

    assert_eq!(placed.len(), 2);
    assert_eq!(placed[0].meeting_id, "m1");
    assert_eq!(placed[0].path, root.join("A (3)"));
    assert_eq!(fs::read_to_string(root.join("A (3)/new.txt")).unwrap(), "new");
    assert_eq!(fs::read_to_string(root.join("A/old.txt")).unwrap(), "old");
    assert_eq!(placed[1].path, root.join("B"));
    assert!(!staging.exists(), "empty staging is removed");
}

#[test]
fn undo_restores_staging() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("A")).unwrap();
    stage(&staging, "A", "new.txt", "new");
    stage(&staging, "B", "b.txt", "b");
    let placed = place_folders(&staging, &root, &[rref("m1", "A"), rref("m2", "B")]).unwrap();
    assert_eq!(placed[0].path, root.join("A (2)"));

    // place_folders removed the empty staging dir; undo must recreate it.
    undo(&placed, &staging).unwrap();

    assert_eq!(fs::read_to_string(staging.join("A/new.txt")).unwrap(), "new");
    assert_eq!(fs::read_to_string(staging.join("B/b.txt")).unwrap(), "b");
    assert!(!root.join("A (2)").exists());
    assert!(!root.join("B").exists());
    assert!(root.join("A").exists());
}

#[test]
fn missing_staged_folder_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    let root = tmp.path().join("root");
    stage(&staging, "B", "b.txt", "b");

    let placed = place_folders(&staging, &root, &[rref("m1", "A"), rref("m2", "B")]).unwrap();

    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].meeting_id, "m2");
    assert!(root.join("B/b.txt").exists());
}

#[test]
fn mid_loop_failure_undoes_placed_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    let root = tmp.path().join("root");
    stage(&staging, "A", "a.txt", "a");
    fs::create_dir_all(&root).unwrap();

    // Second ref is a path-escaping name; it must error, and A must be rolled back.
    let err = place_folders(&staging, &root, &[rref("m1", "A"), rref("m2", "../x")]).unwrap_err();
    assert!(matches!(err, BackupError::Io(_)), "{err:?}");
    assert!(staging.join("A/a.txt").exists());
    assert!(!root.join("A").exists());
}

#[test]
fn gives_up_after_1000_collisions() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    let root = tmp.path().join("root");
    stage(&staging, "A", "a.txt", "a");
    fs::create_dir_all(root.join("A")).unwrap();
    for n in 2..=1000 {
        fs::create_dir_all(root.join(format!("A ({n})"))).unwrap();
    }
    let err = place_folders(&staging, &root, &[rref("m1", "A")]).unwrap_err();
    assert!(matches!(err, BackupError::Io(_)), "{err:?}");
    assert!(staging.join("A/a.txt").exists());
}
