//! specs/0092 task 5: Replace restore (stage now, apply at the next launch).

mod common;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use app_lib::backup::archive::read_manifest;
use app_lib::backup::export::{export_to, ExportOptions};
use app_lib::backup::manifest::{build_schema_version, Manifest};
use app_lib::backup::progress::NoProgress;
use app_lib::backup::replace::{
    apply_pending, apply_until, cleanup_replaced, stage, MARKER_NAME, RESULT_NAME,
};
use app_lib::backup::BackupError;
use app_lib::database::manager::DatabaseManager;
use common::fresh_db;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

struct Profile {
    _dir: tempfile::TempDir,
    db: DatabaseManager,
    app_data: PathBuf,
    root: PathBuf,
    _base: tempfile::TempDir,
}

async fn profile() -> Profile {
    let (dir, db) = fresh_db().await;
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("rec");
    fs::create_dir_all(&root).unwrap();
    Profile {
        app_data: dir.path().to_path_buf(),
        _dir: dir,
        db,
        root,
        _base: base,
    }
}

fn make_folder(root: &Path, name: &str) -> PathBuf {
    let f = root.join(name);
    fs::create_dir_all(&f).unwrap();
    fs::write(f.join("metadata.json"), b"{\"a\":1}").unwrap();
    fs::write(f.join("audio.opus"), vec![7u8; 2048]).unwrap();
    f
}

async fn exec(pool: &SqlitePool, sql: &str) {
    sqlx::query(sql).execute(pool).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn add_meeting(pool: &SqlitePool, id: &str, folder: Option<&Path>) {
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES (?, ?, 't', 't', ?)",
    )
    .bind(id)
    .bind(format!("title {id}"))
    .bind(folder.map(|p| p.to_string_lossy().into_owned()))
    .execute(pool)
    .await
    .unwrap();
}

/// "Old Mac": two meetings with folders plus notes, to-do, topic, person, voiceprint.
async fn old_mac() -> Profile {
    let a = profile().await;
    let p = a.db.pool();
    add_meeting(p, "m1", Some(&make_folder(&a.root, "Alpha_1"))).await;
    add_meeting(p, "m2", Some(&make_folder(&a.root, "Beta_2"))).await;
    exec(p, "INSERT INTO meeting_notes (meeting_id, notes_markdown, created_at, updated_at) VALUES ('m1','hello','t','t')").await;
    exec(p, "INSERT INTO action_items (id, meeting_id, description, content_key, created_at, updated_at) VALUES ('ai1','m1','do it','k','t','t')").await;
    exec(p, "INSERT INTO topics (id, name, created_at, updated_at) VALUES ('t1','Topic','t','t')").await;
    exec(p, "INSERT INTO people (id, display_name, created_at, updated_at) VALUES ('p1','Pat','t','t')").await;
    exec(p, "INSERT INTO voiceprints (id, person_id, embedding, embedding_dim, embedding_model, created_at) VALUES ('vp1','p1',x'00',1,'m','t')").await;
    exec(p, "DELETE FROM settings").await;
    exec(p, "INSERT INTO settings (id, provider, model, whisperModel) VALUES ('a-settings','a-prov','m','w')").await;
    fs::write(a.app_data.join("preferences.json"), b"{\"from\":\"A\"}").unwrap();
    fs::write(
        a.app_data.join("recording_preferences.json"),
        br#"{"preferences":{"save_folder":"/old/mac/rec","save_folder_user_chosen":true,"previous_save_folders":["/x"],"auto_save":true}}"#,
    )
    .unwrap();
    a
}

/// "New Mac": its own meeting, settings row, calendar account and prefs.
async fn new_mac() -> Profile {
    let b = profile().await;
    let p = b.db.pool();
    add_meeting(p, "b1", Some(&make_folder(&b.root, "Local_1"))).await;
    exec(p, "DELETE FROM settings").await;
    exec(p, "INSERT INTO settings (id, provider, model, whisperModel) VALUES ('b-settings','b-prov','m','w')").await;
    exec(p, "INSERT INTO google_calendar_account (id, email, connected_at) VALUES (1,'b@x.com','t')").await;
    fs::write(b.app_data.join("preferences.json"), b"{\"from\":\"B\"}").unwrap();
    fs::write(
        b.app_data.join("recording_preferences.json"),
        br#"{"preferences":{"save_folder":"/new/mac/rec","save_folder_user_chosen":true,"previous_save_folders":["/y"]}}"#,
    )
    .unwrap();
    b
}

async fn export_a(a: &Profile, out: &Path) -> PathBuf {
    let opts = ExportOptions {
        include_audio: true,
        app_data_dir: a.app_data.clone(),
        abort_if: None,
        allowed_roots: vec![],
    };
    export_to(a.db.pool(), out, &opts, &NoProgress, &CancellationToken::new())
        .await
        .unwrap()
        .path
}

async fn stage_into(b: &Profile, zip: &Path) -> Result<(), BackupError> {
    stage(b.db.pool(), zip, &b.app_data, &b.root, &NoProgress, &CancellationToken::new()).await
}

async fn reopen(b: &Profile) -> DatabaseManager {
    DatabaseManager::new(
        b.app_data.join("meeting_minutes.sqlite").to_str().unwrap(),
        b.app_data.join("none.db").to_str().unwrap(),
    )
    .await
    .unwrap()
}

async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

async fn ids(pool: &SqlitePool, sql: &str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(pool).await.unwrap()
}

/// Everything restored, with the recordings root masked so runs can be compared.
async fn state_of(pool: &SqlitePool, root: &Path) -> Vec<String> {
    let mut out = ids(
        pool,
        "SELECT id || '|' || COALESCE(folder_path, 'NULL') FROM meetings ORDER BY id",
    )
    .await;
    for t in ["meeting_notes", "action_items", "topics", "people", "voiceprints", "settings", "google_calendar_account"] {
        out.push(format!("{t}={}", count(pool, &format!("SELECT COUNT(*) FROM {t}")).await));
    }
    let root = root.to_string_lossy().into_owned();
    out.into_iter().map(|s| s.replace(&root, "ROOT")).collect()
}

fn dir_names(p: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(p)
        .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn result_json(app_data: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(app_data.join(RESULT_NAME)).unwrap()).unwrap()
}

#[tokio::test]
async fn replace_restores_everything_and_reroots() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;

    stage_into(&b, &zip).await.unwrap();
    assert!(b.app_data.join(MARKER_NAME).is_file());
    // Staging must not have touched the live DB.
    assert_eq!(count(b.db.pool(), "SELECT COUNT(*) FROM meetings").await, 1);
    b.db.pool().close().await;

    let outcome = apply_pending(&b.app_data).await.unwrap().unwrap();
    assert_eq!(outcome.meetings, 2);
    assert_eq!(outcome.from, read_manifest(&zip).unwrap().created_at);

    let db = reopen(&b).await;
    let p = db.pool();
    assert_eq!(ids(p, "SELECT id FROM meetings ORDER BY id").await, ["m1", "m2"]);
    assert_eq!(count(p, "SELECT COUNT(*) FROM meeting_notes").await, 1);
    assert_eq!(count(p, "SELECT COUNT(*) FROM action_items").await, 1);
    assert_eq!(count(p, "SELECT COUNT(*) FROM topics").await, 1);
    assert_eq!(count(p, "SELECT COUNT(*) FROM people").await, 1);
    assert_eq!(count(p, "SELECT COUNT(*) FROM voiceprints").await, 1);
    for f in ids(p, "SELECT folder_path FROM meetings").await {
        assert!(f.starts_with(b.root.to_str().unwrap()), "{f}");
        assert!(Path::new(&f).join("metadata.json").is_file());
        assert!(Path::new(&f).join("audio.opus").is_file());
    }
    // This Mac's own folder is left alone, and staging is gone.
    assert!(b.root.join("Local_1/metadata.json").is_file());
    assert_eq!(dir_names(&b.root), ["Alpha_1", "Beta_2", "Local_1"]);
    assert!(!b.app_data.join(MARKER_NAME).exists());
    assert!(!b.app_data.join("restore-staging").exists());

    let r = result_json(&b.app_data);
    assert_eq!(r["ok"], true);
    assert_eq!(r["meetings"], 2);

    // Settings: the backup's file, but this Mac's recording folder.
    assert_eq!(fs::read(b.app_data.join("preferences.json")).unwrap(), b"{\"from\":\"A\"}");
    let rp: serde_json::Value =
        serde_json::from_slice(&fs::read(b.app_data.join("recording_preferences.json")).unwrap()).unwrap();
    assert_eq!(rp["preferences"]["save_folder"], "/new/mac/rec");
    assert_eq!(rp["preferences"]["save_folder_user_chosen"], true);
    assert_eq!(rp["preferences"]["previous_save_folders"], serde_json::json!([]));
    assert_eq!(rp["preferences"]["auto_save"], true);

    // The old DB sits aside until cleanup.
    let aside = |names: Vec<String>| names.iter().filter(|n| n.contains(".replaced-")).count();
    assert!(aside(dir_names(&b.app_data)) >= 1);
    cleanup_replaced(&b.app_data);
    assert_eq!(aside(dir_names(&b.app_data)), 0);
}

#[tokio::test]
async fn replace_keeps_machine_bound_tables() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    stage_into(&b, &zip).await.unwrap();
    b.db.pool().close().await;
    apply_pending(&b.app_data).await.unwrap();

    let db = reopen(&b).await;
    assert_eq!(ids(db.pool(), "SELECT id FROM settings").await, ["b-settings"]);
    assert_eq!(ids(db.pool(), "SELECT email FROM google_calendar_account").await, ["b@x.com"]);
}

#[tokio::test]
async fn replace_without_prior_recording_prefs_drops_save_folder() {
    let a = old_mac().await;
    let b = new_mac().await;
    fs::remove_file(b.app_data.join("recording_preferences.json")).unwrap();
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    stage_into(&b, &zip).await.unwrap();
    b.db.pool().close().await;
    apply_pending(&b.app_data).await.unwrap();
    let rp: serde_json::Value =
        serde_json::from_slice(&fs::read(b.app_data.join("recording_preferences.json")).unwrap()).unwrap();
    assert!(rp["preferences"].get("save_folder").is_none());
    assert_eq!(rp["preferences"]["save_folder_user_chosen"], false);
}

#[tokio::test]
async fn replace_writes_and_prunes_safety_copies() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    for _ in 0..4 {
        stage_into(&b, &zip).await.unwrap();
    }
    let names = dir_names(&b.app_data.join("pre-restore"));
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(names.iter().all(|n| n.starts_with("Nixon Backup ") && n.ends_with(" (before restore).zip")));
    // The safety copy is a real backup of B's data.
    let newest = b.app_data.join("pre-restore").join(names.last().unwrap());
    let m = read_manifest(&newest).unwrap();
    assert_eq!(m.counts.meetings, 1);
    assert!(!m.includes_audio);
}

#[tokio::test]
async fn replace_rollback_on_corrupt_staged_db() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    stage_into(&b, &zip).await.unwrap();
    b.db.pool().close().await;
    fs::write(b.app_data.join("restore-staging/db/meeting_minutes.sqlite"), b"not a database at all").unwrap();

    assert!(apply_pending(&b.app_data).await.is_err());
    assert_rolled_back(&b).await;
}

#[tokio::test]
async fn replace_rollback_after_folders_were_placed() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    stage_into(&b, &zip).await.unwrap();
    b.db.pool().close().await;
    // Step 1 passes, step 2 places the folders, step 3 fails (a trigger aborts updates).
    let staged = b.app_data.join("restore-staging/db/meeting_minutes.sqlite");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&staged))
        .await
        .unwrap();
    exec(&pool, "CREATE TRIGGER boom BEFORE UPDATE ON meetings BEGIN SELECT RAISE(ABORT, 'boom'); END").await;
    pool.close().await;

    assert!(apply_pending(&b.app_data).await.is_err());
    assert_rolled_back(&b).await;
    // The folders went back to staging for diagnosis.
    let staging = b.root.join(".nixon-restore-staging");
    assert_eq!(dir_names(&staging), ["Alpha_1", "Beta_2"]);
}

async fn assert_rolled_back(b: &Profile) {
    let db = reopen(b).await;
    assert_eq!(ids(db.pool(), "SELECT id FROM meetings").await, ["b1"]);
    assert_eq!(ids(db.pool(), "SELECT id FROM settings").await, ["b-settings"]);
    assert_eq!(dir_names(&b.root).into_iter().filter(|n| n != ".nixon-restore-staging").collect::<Vec<_>>(), ["Local_1"]);
    assert!(b.root.join("Local_1/audio.opus").is_file());
    assert_eq!(fs::read(b.app_data.join("preferences.json")).unwrap(), b"{\"from\":\"B\"}");
    assert!(!b.app_data.join(MARKER_NAME).exists());
    assert!(!dir_names(&b.app_data).iter().any(|n| n.contains(".replaced-")));
    let r = result_json(&b.app_data);
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().is_some_and(|e| !e.is_empty()));
}

#[tokio::test]
async fn apply_is_idempotent_after_partial_run() {
    let a = old_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;

    // Reference: one clean apply.
    let b = new_mac().await;
    stage_into(&b, &zip).await.unwrap();
    b.db.pool().close().await;
    apply_pending(&b.app_data).await.unwrap().unwrap();
    let reference = state_of(reopen(&b).await.pool(), &b.root).await;
    assert_eq!(reference.len(), 2 + 7);

    // Crash after each step, then launch again.
    for stop in 1..=5u8 {
        let b = new_mac().await;
        stage_into(&b, &zip).await.unwrap();
        b.db.pool().close().await;
        assert!(apply_until(&b.app_data, stop).await.unwrap().is_none());
        assert!(b.app_data.join(MARKER_NAME).is_file(), "stop {stop}");
        let again = apply_pending(&b.app_data).await.unwrap();
        assert!(again.is_some(), "stop {stop}");
        assert_eq!(state_of(reopen(&b).await.pool(), &b.root).await, reference, "stop {stop}");
        assert_eq!(ids(reopen(&b).await.pool(), "SELECT id FROM settings").await, ["b-settings"], "stop {stop}");
        assert_eq!(dir_names(&b.root), ["Alpha_1", "Beta_2", "Local_1"], "stop {stop}");
        assert!(!b.app_data.join(MARKER_NAME).exists());
    }

    // Marker left behind after a full apply (staging already gone): harmless.
    let b = new_mac().await;
    stage_into(&b, &zip).await.unwrap();
    let marker = fs::read(b.app_data.join(MARKER_NAME)).unwrap();
    b.db.pool().close().await;
    apply_pending(&b.app_data).await.unwrap().unwrap();
    let once = state_of(reopen(&b).await.pool(), &b.root).await;
    fs::write(b.app_data.join(MARKER_NAME), marker).unwrap();
    assert!(apply_pending(&b.app_data).await.is_ok());
    assert_eq!(state_of(reopen(&b).await.pool(), &b.root).await, once);
    assert!(!b.app_data.join(MARKER_NAME).exists());
}

#[tokio::test]
async fn stage_refuses_newer_schema() {
    let b = new_mac().await;
    let a = old_mac().await;
    let out = tempfile::tempdir().unwrap();
    let real = read_manifest(&export_a(&a, out.path()).await).unwrap();
    let newer = Manifest { schema_version: build_schema_version() + 1, ..real };
    let zip = out.path().join("newer.zip");
    let mut w = zip::ZipWriter::new(fs::File::create(&zip).unwrap());
    w.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
    w.write_all(&serde_json::to_vec(&newer).unwrap()).unwrap();
    w.finish().unwrap();

    let err = stage_into(&b, &zip).await.unwrap_err();
    assert!(matches!(err, BackupError::NewerSchema { .. }), "{err:?}");
    assert!(!b.app_data.join(MARKER_NAME).exists());
    assert!(!b.app_data.join("pre-restore").exists());
}

#[tokio::test]
async fn apply_without_marker_is_noop() {
    let b = new_mac().await;
    assert_eq!(apply_pending(&b.app_data).await.unwrap(), None);
    assert!(!b.app_data.join(RESULT_NAME).exists());
}

fn aside_count(app_data: &Path) -> usize {
    dir_names(app_data).iter().filter(|n| n.contains(".replaced-")).count()
}

#[tokio::test]
async fn replace_rollback_after_db_swap_when_settings_step_fails() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    stage_into(&b, &zip).await.unwrap();
    b.db.pool().close().await;
    // The settings step cannot write preferences.json (a directory is in the way).
    fs::remove_file(b.app_data.join("preferences.json")).unwrap();
    fs::create_dir(b.app_data.join("preferences.json")).unwrap();

    assert!(apply_pending(&b.app_data).await.is_err());

    let db = reopen(&b).await;
    assert_eq!(ids(db.pool(), "SELECT id FROM meetings").await, ["b1"]);
    assert_eq!(ids(db.pool(), "SELECT id FROM settings").await, ["b-settings"]);
    assert_eq!(aside_count(&b.app_data), 0);
    assert!(!b.app_data.join(MARKER_NAME).exists());
    assert_eq!(result_json(&b.app_data)["ok"], false);
    assert_eq!(dir_names(&b.root).into_iter().filter(|n| n != ".nixon-restore-staging").collect::<Vec<_>>(), ["Local_1"]);
    // The restored DB is kept in staging for diagnosis.
    assert!(b.app_data.join("restore-staging/db/meeting_minutes.sqlite").is_file());
}

/// Arrange a launch that died after moving only the main DB file aside.
async fn half_swapped(b: &Profile, zip: &Path) {
    stage_into(b, zip).await.unwrap();
    b.db.pool().close().await;
    assert!(apply_until(&b.app_data, 3).await.unwrap().is_none());
    let state_path = b.app_data.join("restore-staging/apply-state.json");
    let mut st: serde_json::Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    st["swap_ts"] = serde_json::json!("T1");
    fs::write(&state_path, serde_json::to_vec(&st).unwrap()).unwrap();
    // An (empty, hence harmless) WAL/SHM pair left next to the original.
    fs::write(b.app_data.join("meeting_minutes.sqlite-wal"), b"").unwrap();
    fs::write(b.app_data.join("meeting_minutes.sqlite-shm"), b"").unwrap();
    fs::rename(
        b.app_data.join("meeting_minutes.sqlite"),
        b.app_data.join("meeting_minutes.sqlite.replaced-T1"),
    )
    .unwrap();
}

#[tokio::test]
async fn half_swapped_launch_completes_the_restore() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    half_swapped(&b, &zip).await;

    assert!(apply_pending(&b.app_data).await.unwrap().is_some());
    let db = reopen(&b).await;
    assert_eq!(ids(db.pool(), "SELECT id FROM meetings ORDER BY id").await, ["m1", "m2"]);
    assert_eq!(ids(db.pool(), "SELECT id FROM settings").await, ["b-settings"]);
    // Original main, WAL and SHM are all aside (none left to confuse the new DB).
    let names = dir_names(&b.app_data);
    for n in ["meeting_minutes.sqlite.replaced-T1", "meeting_minutes.sqlite-wal.replaced-T1", "meeting_minutes.sqlite-shm.replaced-T1"] {
        assert!(names.contains(&n.to_string()), "{n} in {names:?}");
    }
    cleanup_replaced(&b.app_data);
    assert_eq!(aside_count(&b.app_data), 0);
}

#[tokio::test]
async fn half_swapped_launch_that_then_fails_restores_the_original() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    half_swapped(&b, &zip).await;
    fs::remove_file(b.app_data.join("preferences.json")).unwrap();
    fs::create_dir(b.app_data.join("preferences.json")).unwrap();

    assert!(apply_pending(&b.app_data).await.is_err());
    let db = reopen(&b).await;
    assert_eq!(ids(db.pool(), "SELECT id FROM meetings").await, ["b1"]);
    assert_eq!(ids(db.pool(), "SELECT id FROM settings").await, ["b-settings"]);
    assert_eq!(aside_count(&b.app_data), 0);
    assert!(!b.app_data.join(MARKER_NAME).exists());
}

#[tokio::test]
async fn cancel_pending_removes_marker_and_staging() {
    let a = old_mac().await;
    let b = new_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export_a(&a, out.path()).await;
    stage_into(&b, &zip).await.unwrap();
    assert!(b.app_data.join(MARKER_NAME).is_file());
    assert!(b.app_data.join("restore-staging").is_dir());

    app_lib::backup::replace::cancel_pending(&b.app_data);

    assert!(!b.app_data.join(MARKER_NAME).exists());
    assert!(!b.app_data.join("restore-staging").exists());
    assert!(!b.root.join(".nixon-restore-staging").exists());
    // Nothing else is touched, and a second call is harmless.
    assert_eq!(count(b.db.pool(), "SELECT COUNT(*) FROM meetings").await, 1);
    app_lib::backup::replace::cancel_pending(&b.app_data);
}
