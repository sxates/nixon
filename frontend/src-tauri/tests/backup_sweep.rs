//! specs/0092 final review I6: leftovers of interrupted jobs are swept at startup.

use std::fs;

use app_lib::backup::sweep::sweep_leftovers;

fn touch_dir(p: &std::path::Path) {
    fs::create_dir_all(p).unwrap();
    fs::write(p.join("big"), b"x").unwrap();
}

#[test]
fn removes_leftovers_and_keeps_everything_else() {
    let app = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    fs::write(app.path().join(".backup-snapshot-abc.sqlite"), b"x").unwrap();
    fs::write(app.path().join(".backup-fingerprint-abc.sqlite"), b"x").unwrap();
    fs::write(app.path().join("meeting_minutes.sqlite"), b"db").unwrap();
    fs::write(app.path().join("backup.json"), b"{}").unwrap();
    touch_dir(&app.path().join("add-staging"));
    touch_dir(&app.path().join("pre-restore"));
    touch_dir(&root.path().join(".nixon-add-staging"));
    touch_dir(&root.path().join(".nixon-restore-staging"));
    touch_dir(&root.path().join("Meeting_1"));

    sweep_leftovers(app.path(), &[root.path().to_path_buf()]);

    for gone in [".backup-snapshot-abc.sqlite", ".backup-fingerprint-abc.sqlite", "add-staging"] {
        assert!(!app.path().join(gone).exists(), "{gone}");
    }
    for kept in ["meeting_minutes.sqlite", "backup.json", "pre-restore"] {
        assert!(app.path().join(kept).exists(), "{kept}");
    }
    assert!(!root.path().join(".nixon-add-staging").exists());
    assert!(!root.path().join(".nixon-restore-staging").exists());
    assert!(root.path().join("Meeting_1/big").exists());
}

#[test]
fn does_nothing_while_a_restore_is_pending() {
    let app = tempfile::tempdir().unwrap();
    fs::write(app.path().join("restore-pending.json"), b"{}").unwrap();
    touch_dir(&app.path().join("restore-staging"));
    touch_dir(&app.path().join("add-staging"));
    sweep_leftovers(app.path(), &[]);
    assert!(app.path().join("restore-staging/big").exists());
    assert!(app.path().join("add-staging/big").exists());
}
