//! specs/0092 task 7: scheduled backups (due check, pruning, skip-when-unchanged, abort).

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use app_lib::backup::schedule::{is_due, prune, run_once, RunResult};
use app_lib::backup::settings::{self, BackupSettings};
use app_lib::backup::BackupError;
use chrono::{DateTime, Local, TimeZone};
use common::fresh_db;

fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
    Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
}

fn enabled(folder: &Path) -> BackupSettings {
    BackupSettings {
        enabled: true,
        folder: Some(folder.to_path_buf()),
        ..Default::default()
    }
}

fn never() -> Arc<dyn Fn() -> bool + Send + Sync> {
    Arc::new(|| false)
}

fn zips(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn due_once_per_local_day() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = enabled(dir.path());
    assert!(is_due(at(2026, 11, 24, 9, 0), &s));

    s.last_success_at = Some(at(2026, 11, 23, 23, 59).to_rfc3339());
    assert!(is_due(at(2026, 11, 24, 0, 1), &s));

    s.last_success_at = Some(at(2026, 11, 24, 0, 5).to_rfc3339());
    assert!(!is_due(at(2026, 11, 24, 23, 0), &s));

    let mut off = enabled(dir.path());
    off.enabled = false;
    assert!(!is_due(at(2026, 11, 24, 9, 0), &off));
    let mut nofolder = enabled(dir.path());
    nofolder.folder = None;
    assert!(!is_due(at(2026, 11, 24, 9, 0), &nofolder));
}

#[test]
fn prune_keeps_newest_and_ignores_lookalikes() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path();
    for d in 10..19 {
        fs::write(f.join(format!("Nixon Backup 2026-11-{d} 0930.zip")), b"z").unwrap();
    }
    let others = [
        "Nixon Backup 2026-11-24 0930 (1).zip",
        "Nixon Backup 2026-11-24 0930 (before restore).zip",
        "notes.zip",
        "Nixon Backup 2026-11-24 0930.zip.partial",
    ];
    for o in others {
        fs::write(f.join(o), b"x").unwrap();
    }
    let old = "Nixon Backup 2026-11-01 0930.zip.partial";
    let lookalike = "Nixon Backup foo.zip.partial";
    for name in [old, lookalike] {
        fs::write(f.join(name), b"x").unwrap();
        let file = fs::File::options().write(true).open(f.join(name)).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(2 * 24 * 3600))
            .unwrap();
    }

    let deleted = prune(f, 7).unwrap();
    assert_eq!(deleted.len(), 3);
    let left = zips(f);
    assert!(!left.contains(&"Nixon Backup 2026-11-10 0930.zip".to_string()));
    assert!(!left.contains(&"Nixon Backup 2026-11-11 0930.zip".to_string()));
    assert!(!left.contains(&old.to_string()));
    assert!(left.contains(&"Nixon Backup 2026-11-12 0930.zip".to_string()));
    for o in others {
        assert!(left.contains(&o.to_string()), "{o} must survive");
    }
    assert!(left.contains(&lookalike.to_string()));
    assert_eq!(left.len(), 7 + others.len() + 1);
}

#[tokio::test]
async fn run_skips_when_unchanged() {
    let (dir, db) = fresh_db().await;
    let dest = tempfile::tempdir().unwrap();
    settings::save(dir.path(), &enabled(dest.path())).unwrap();
    let now = at(2026, 11, 24, 9, 0);

    let first = run_once(db.pool(), dir.path(), now, never(), true, vec![]).await.unwrap();
    assert!(matches!(first, RunResult::Written(_)));
    // Not due again today without force.
    let same_day = run_once(db.pool(), dir.path(), now, never(), false, vec![]).await.unwrap();
    assert!(matches!(same_day, RunResult::NotDue));
    // Next day, nothing changed and the backup is still in the folder: skipped.
    let next = at(2026, 11, 25, 9, 0);
    let second = run_once(db.pool(), dir.path(), next, never(), false, vec![]).await.unwrap();
    assert!(matches!(second, RunResult::Skipped));
    assert_eq!(zips(dest.path()).len(), 1);
    let s = settings::load(dir.path());
    assert!(s.last_fingerprint.is_some() && s.last_error.is_none());
}

#[tokio::test]
async fn unchanged_data_is_still_backed_up_when_the_folder_has_no_backup() {
    let (dir, db) = fresh_db().await;
    let dest = tempfile::tempdir().unwrap();
    settings::save(dir.path(), &enabled(dest.path())).unwrap();
    run_once(db.pool(), dir.path(), at(2026, 11, 24, 9, 0), never(), true, vec![]).await.unwrap();
    for z in zips(dest.path()) {
        fs::remove_file(dest.path().join(z)).unwrap(); // user deleted the files
    }
    let r = run_once(db.pool(), dir.path(), at(2026, 11, 25, 9, 0), never(), false, vec![]).await;
    assert!(matches!(r, Ok(RunResult::Written(_))));
    assert_eq!(zips(dest.path()).len(), 1);
}

#[tokio::test]
async fn back_up_now_always_writes_even_when_unchanged() {
    let (dir, db) = fresh_db().await;
    let dest = tempfile::tempdir().unwrap();
    settings::save(dir.path(), &enabled(dest.path())).unwrap();
    run_once(db.pool(), dir.path(), at(2026, 11, 24, 9, 0), never(), true, vec![]).await.unwrap();
    let r = run_once(db.pool(), dir.path(), at(2026, 11, 24, 9, 5), never(), true, vec![]).await;
    assert!(matches!(r, Ok(RunResult::Written(_))));
    assert_eq!(zips(dest.path()).len(), 2);
}

#[test]
fn changing_the_folder_forgets_the_last_backup() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = enabled(dir.path());
    s.last_fingerprint = Some("abc".into());
    s.last_success_at = Some("2026-11-24T09:00:00+00:00".into());
    s.apply_ui(true, Some("/other".into()), false);
    assert!(s.last_fingerprint.is_none() && s.last_success_at.is_none());
    assert_eq!(s.folder.as_deref(), Some(Path::new("/other")));

    // Same folder: results are kept.
    s.last_fingerprint = Some("abc".into());
    s.apply_ui(false, Some("/other".into()), true);
    assert_eq!(s.last_fingerprint.as_deref(), Some("abc"));
    assert!(!s.enabled && s.include_audio);
}

#[tokio::test]
async fn run_aborts_when_recording_starts() {
    let (dir, db) = fresh_db().await;
    let dest = tempfile::tempdir().unwrap();
    settings::save(dir.path(), &enabled(dest.path())).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let rec: Arc<dyn Fn() -> bool + Send + Sync> =
        Arc::new(move || c.fetch_add(1, Ordering::SeqCst) >= 1);

    let r = run_once(db.pool(), dir.path(), at(2026, 11, 24, 9, 0), rec, true, vec![]).await;
    assert_eq!(r.unwrap_err(), BackupError::Cancelled);
    assert!(zips(dest.path()).is_empty(), "no zip and no .partial");
    assert!(settings::load(dir.path()).last_success_at.is_none());
}

#[tokio::test]
async fn run_records_error_for_missing_folder() {
    let (dir, db) = fresh_db().await;
    let dest = tempfile::tempdir().unwrap();
    let folder = dest.path().join("gone");
    settings::save(dir.path(), &enabled(&folder)).unwrap();

    let r = run_once(db.pool(), dir.path(), at(2026, 11, 24, 9, 0), never(), false, vec![]).await;
    assert!(r.is_err());
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("backup.json")).unwrap()).unwrap();
    assert!(json["lastError"].is_string());
    assert!(json["lastAttemptAt"].is_string());
}

#[tokio::test]
async fn force_without_folder_is_an_error() {
    let (dir, db) = fresh_db().await;
    let r = run_once(db.pool(), dir.path(), at(2026, 11, 24, 9, 0), never(), true, vec![]).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn save_failure_does_not_hide_a_written_backup() {
    let (dir, db) = fresh_db().await;
    let dest = tempfile::tempdir().unwrap();
    settings::save(dir.path(), &enabled(dest.path())).unwrap();
    // write_atomic stages through backup.json.tmp; a directory there makes every save fail.
    fs::create_dir(dir.path().join("backup.json.tmp")).unwrap();

    let r = run_once(db.pool(), dir.path(), at(2026, 11, 24, 9, 0), never(), false, vec![]).await;
    assert!(matches!(r, Ok(RunResult::Written(_))));
    assert_eq!(zips(dest.path()).len(), 1);
}
