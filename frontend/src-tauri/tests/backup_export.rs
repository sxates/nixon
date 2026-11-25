//! specs/0092 task 3: zip archive writer/reader and export orchestration.

mod common;

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_lib::backup::archive::{extract, read_manifest};
use app_lib::backup::export::{export_to, export_to_file, ExportOptions};
use app_lib::backup::progress::{NoProgress, Phase, Progress};
use app_lib::backup::BackupError;
use common::fresh_db;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use zip::{CompressionMethod, ZipArchive};

async fn add_meeting(pool: &SqlitePool, id: &str, folder: Option<&Path>) {
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) \
         VALUES (?, ?, 't', 't', ?)",
    )
    .bind(id)
    .bind(format!("title {id}"))
    .bind(folder.map(|p| p.to_string_lossy().into_owned()))
    .execute(pool)
    .await
    .unwrap();
}

fn make_folder(root: &Path, name: &str) -> PathBuf {
    let f = root.join(name);
    fs::create_dir_all(f.join(".checkpoints")).unwrap();
    fs::write(f.join("metadata.json"), b"{\"a\":1}").unwrap();
    fs::write(f.join("audio.opus"), vec![7u8; 4096]).unwrap();
    fs::write(f.join(".checkpoints/x.json"), b"tmp").unwrap();
    f
}

fn opts(app_data: &Path, include_audio: bool) -> ExportOptions {
    ExportOptions {
        include_audio,
        app_data_dir: app_data.to_path_buf(),
        abort_if: None,
        allowed_roots: vec![],
    }
}

fn names(zip: &Path) -> Vec<String> {
    let mut a = ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    (0..a.len())
        .map(|i| a.by_index(i).unwrap().name().to_string())
        .collect()
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: app_lib::database::manager::DatabaseManager,
    app_data: PathBuf,
    rec_root: PathBuf,
    out: PathBuf,
    base: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let (dir, db) = fresh_db().await;
    let base = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();
    fs::write(app_data.join("preferences.json"), b"{}").unwrap();
    fs::write(app_data.join("agent_access.json"), b"{}").unwrap();
    let rec_root = base.path().join("rec");
    fs::create_dir_all(&rec_root).unwrap();
    let out = base.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let f1 = make_folder(&rec_root, "Alpha_1");
    let f2 = make_folder(&rec_root, "Beta_2");
    add_meeting(db.pool(), "m1", Some(&f1)).await;
    add_meeting(db.pool(), "m2", Some(&f2)).await;
    Fixture { _dir: dir, db, app_data, rec_root, out, base }
}

fn no_snapshot_left(app_data: &Path) -> bool {
    !fs::read_dir(app_data)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".backup-snapshot-"))
}

#[tokio::test]
async fn export_roundtrip_without_audio() {
    let f = fixture().await;
    let s = export_to(f.db.pool(), &f.out, &opts(&f.app_data, false), &NoProgress, &CancellationToken::new())
        .await
        .unwrap();
    let n = names(&s.path);
    assert!(n.contains(&"manifest.json".to_string()));
    assert!(n.contains(&"db/meeting_minutes.sqlite".to_string()));
    assert!(n.contains(&"settings/preferences.json".to_string()));
    assert!(!n.iter().any(|x| x.contains("agent_access")));
    assert!(n.contains(&"recordings/Alpha_1/metadata.json".to_string()));
    assert!(n.contains(&"recordings/Beta_2/metadata.json".to_string()));
    assert!(!n.iter().any(|x| x.ends_with(".opus") || x.contains(".checkpoints")));
    let m = read_manifest(&s.path).unwrap();
    assert!(!m.includes_audio);
    assert_eq!(m.counts.meetings, 2);
    assert_eq!(m.recordings.len(), 2);
    assert_eq!(m, s.manifest);
    assert!(s.path.file_name().unwrap().to_string_lossy().starts_with("Nixon Backup "));
    assert!(no_snapshot_left(&f.app_data));

    // Round trip through extract.
    let (a, b) = (f.base.path().join("a"), f.base.path().join("b"));
    extract(&s.path, &a, &b, None, &NoProgress, &CancellationToken::new()).unwrap();
    assert!(a.join("db/meeting_minutes.sqlite").is_file());
    assert_eq!(fs::read(b.join("Alpha_1/metadata.json")).unwrap(), b"{\"a\":1}");
    let only: HashSet<String> = ["m2".to_string()].into();
    let (c, d) = (f.base.path().join("c"), f.base.path().join("d"));
    extract(&s.path, &c, &d, Some(&only), &NoProgress, &CancellationToken::new()).unwrap();
    assert!(d.join("Beta_2").is_dir() && !d.join("Alpha_1").exists());
}

#[tokio::test]
async fn export_with_audio_stores_media() {
    let f = fixture().await;
    let s = export_to(f.db.pool(), &f.out, &opts(&f.app_data, true), &NoProgress, &CancellationToken::new())
        .await
        .unwrap();
    let mut a = ZipArchive::new(fs::File::open(&s.path).unwrap()).unwrap();
    let e = a.by_name("recordings/Alpha_1/audio.opus").unwrap();
    assert_eq!(e.compression(), CompressionMethod::Stored);
    drop(e);
    let e = a.by_name("recordings/Alpha_1/metadata.json").unwrap();
    assert_eq!(e.compression(), CompressionMethod::Deflated);
    assert!(s.manifest.includes_audio);
}

#[tokio::test]
async fn export_skips_meetings_without_folders() {
    let f = fixture().await;
    let elsewhere = f.base.path().join("elsewhere");
    make_folder(&elsewhere, "Gamma_3");
    add_meeting(f.db.pool(), "m3", None).await;
    add_meeting(f.db.pool(), "m4", Some(&f.rec_root.join("missing"))).await;
    add_meeting(f.db.pool(), "m5", Some(&elsewhere.join("Gamma_3"))).await;
    let mut o = opts(&f.app_data, false);
    o.allowed_roots = vec![f.rec_root.clone()];
    let s = export_to(f.db.pool(), &f.out, &o, &NoProgress, &CancellationToken::new())
        .await
        .unwrap();
    let ids: Vec<_> = s.manifest.recordings.iter().map(|r| r.meeting_id.as_str()).collect();
    assert_eq!(ids, vec!["m1", "m2"]);
    assert_eq!(s.manifest.counts.meetings, 5);
}

#[tokio::test]
async fn export_duplicate_folder_names_get_suffix() {
    let f = fixture().await;
    let other = f.base.path().join("rec2");
    let dup = make_folder(&other, "Alpha_1");
    add_meeting(f.db.pool(), "m9", Some(&dup)).await;
    let s = export_to(f.db.pool(), &f.out, &opts(&f.app_data, false), &NoProgress, &CancellationToken::new())
        .await
        .unwrap();
    let folders: HashSet<_> = s.manifest.recordings.iter().map(|r| r.folder.clone()).collect();
    assert_eq!(folders.len(), 3);
    assert!(folders.contains("Alpha_1 (2)"));
    assert!(names(&s.path).contains(&"recordings/Alpha_1 (2)/metadata.json".to_string()));
}

#[tokio::test]
async fn export_failure_leaves_no_partial() {
    let f = fixture().await;
    let ro = f.base.path().join("ro");
    fs::create_dir_all(&ro).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o500)).unwrap();
    let r = export_to(f.db.pool(), &ro, &opts(&f.app_data, false), &NoProgress, &CancellationToken::new()).await;
    let leftovers = fs::read_dir(&ro).unwrap().count();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(r, Err(BackupError::Io(_))), "{r:?}");
    assert_eq!(leftovers, 0);
    assert!(no_snapshot_left(&f.app_data));
}

struct CancelOnWrite(CancellationToken);
impl Progress for CancelOnWrite {
    fn report(&self, phase: Phase, _d: u64, _t: u64) {
        if phase == Phase::Writing {
            self.0.cancel();
        }
    }
}

#[tokio::test]
async fn export_cancel_cleans_up() {
    let f = fixture().await;
    let cancel = CancellationToken::new();
    let r = export_to(f.db.pool(), &f.out, &opts(&f.app_data, false), &CancelOnWrite(cancel.clone()), &cancel).await;
    assert_eq!(r.unwrap_err(), BackupError::Cancelled);
    assert_eq!(fs::read_dir(&f.out).unwrap().count(), 0);
    assert!(no_snapshot_left(&f.app_data));
}

#[tokio::test]
async fn export_abort_if_cancels() {
    let f = fixture().await;
    let mut o = opts(&f.app_data, false);
    o.abort_if = Some(Arc::new(|| true));
    let r = export_to(f.db.pool(), &f.out, &o, &NoProgress, &CancellationToken::new()).await;
    assert_eq!(r.unwrap_err(), BackupError::Cancelled);
    assert_eq!(fs::read_dir(&f.out).unwrap().count(), 0);
}

#[tokio::test]
async fn export_name_collision_appends_suffix() {
    let f = fixture().await;
    let o = opts(&f.app_data, false);
    let c = CancellationToken::new();
    let a = export_to(f.db.pool(), &f.out, &o, &NoProgress, &c).await.unwrap();
    let b = export_to(f.db.pool(), &f.out, &o, &NoProgress, &c).await.unwrap();
    let d = export_to(f.db.pool(), &f.out, &o, &NoProgress, &c).await.unwrap();
    assert_ne!(a.path, b.path);
    assert!(b.path.to_string_lossy().ends_with(" 2.zip"), "{:?}", b.path);
    assert!(d.path.to_string_lossy().ends_with(" 3.zip"), "{:?}", d.path);
}

#[tokio::test]
async fn export_to_file_uses_explicit_path() {
    let f = fixture().await;
    let dest = f.out.join("mine.zip");
    let s = export_to_file(f.db.pool(), &dest, &opts(&f.app_data, false), &NoProgress, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(s.path, dest);
    assert_eq!(s.bytes, fs::metadata(&dest).unwrap().len());
}

#[tokio::test]
async fn final_file_mode_is_0600() {
    let f = fixture().await;
    let s = export_to(f.db.pool(), &f.out, &opts(&f.app_data, false), &NoProgress, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(fs::metadata(&s.path).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn read_manifest_rejects_non_backup() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("x.zip");
    let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    z.start_file("hello.txt", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(b"hi").unwrap();
    z.finish().unwrap();
    assert_eq!(read_manifest(&p).unwrap_err(), BackupError::NotABackup);
    let t = d.path().join("t.txt");
    fs::write(&t, b"not a zip").unwrap();
    assert_eq!(read_manifest(&t).unwrap_err(), BackupError::NotABackup);
}

#[test]
fn extract_rejects_zip_slip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("evil.zip");
    let manifest = serde_json::json!({
        "format": 1, "app": "Nixon", "app_version": "0", "schema_version": 1,
        "created_at": "x", "includes_audio": false,
        "counts": {"meetings":0,"people":0,"topics":0,"action_items":0}, "recordings": []
    });
    let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("manifest.json", o).unwrap();
    z.write_all(manifest.to_string().as_bytes()).unwrap();
    z.start_file("db/ok.txt", o).unwrap();
    z.write_all(b"ok").unwrap();
    z.start_file("recordings/../../evil", o).unwrap();
    z.write_all(b"x").unwrap();
    z.finish().unwrap();
    let (a, b) = (d.path().join("a/in"), d.path().join("b/in"));
    let r = extract(&p, &a, &b, None, &NoProgress, &CancellationToken::new());
    assert!(matches!(r, Err(BackupError::UnsafeEntry(_))), "{r:?}");
    assert!(!d.path().join("evil").exists() && !d.path().join("a/evil").exists());
    assert!(!a.exists() && !b.exists(), "nothing written at all");
}

#[tokio::test]
async fn export_cancel_during_lease_wait_returns_promptly() {
    use app_lib::audio::folder_lease::{acquire, LeaseHolder};
    let f = fixture().await;
    let _held = acquire("m1", LeaseHolder::Recording).await;
    let cancel = CancellationToken::new();
    let (pool, o, c2) = (f.db.pool().clone(), opts(&f.app_data, false), cancel.clone());
    let out = f.out.clone();
    let task = tokio::spawn(async move { export_to(&pool, &out, &o, &NoProgress, &c2).await });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!task.is_finished(), "should be waiting on the lease");
    cancel.cancel();
    let r = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("export must return promptly after cancel")
        .unwrap();
    assert_eq!(r.unwrap_err(), BackupError::Cancelled);
    assert_eq!(fs::read_dir(&f.out).unwrap().count(), 0);
    assert!(no_snapshot_left(&f.app_data));
}
