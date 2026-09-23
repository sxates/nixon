//! specs/0073 W1 — meetings keep working wherever their recording folder lives, and no two
//! jobs touch one meeting folder at once.
//!
//! Covers the parts of W1 that need a database or an app handle: the stale `folder_path`
//! write-back guard, retranscription resolving its folder from the DB after taking the
//! lease, delete removing a folder in an earlier recordings root, the retention sweep
//! skipping a leased meeting, the webview allow-list, and the recording start's re-read.
//!
//! Run with: cargo test --features metal --test recording_folders

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use app_lib::audio::folder_lease::{self, LeaseHolder};
use app_lib::database::manager::DatabaseManager;
use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::transcript::TranscriptsRepository;
use app_lib::state::AppState;
use app_lib::transcripts::TranscriptSegment;
use sqlx::SqlitePool;
use tauri::Manager;

async fn pool_with_schema() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

/// A mock app with a real, migrated, file-backed database managed as `AppState`.
async fn app_with_db() -> (
    tauri::App<tauri::test::MockRuntime>,
    SqlitePool,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t.sqlite").to_string_lossy().to_string();
    let db_manager = DatabaseManager::new(&db_path, &db_path).await.unwrap();
    let pool = db_manager.pool().clone();
    let app = tauri::test::mock_app();
    app.handle().manage(AppState { db_manager });
    (app, pool, dir)
}

async fn meeting_with_folder(pool: &SqlitePool, folder: Option<&Path>) -> String {
    let folder = folder.map(|f| f.to_string_lossy().to_string());
    MeetingsRepository::create_meeting(pool, Some("Weekly sync".into()), folder, None, None, None)
        .await
        .unwrap()
}

async fn stored_folder(pool: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn recording_folder(parent: &Path, name: &str, meeting_id: Option<&str>) -> PathBuf {
    let folder = parent.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let meta = match meeting_id {
        Some(id) => format!(r#"{{"meeting_id":"{id}","status":"completed"}}"#),
        None => r#"{"status":"completed"}"#.to_string(),
    };
    std::fs::write(folder.join("metadata.json"), meta).unwrap();
    std::fs::write(folder.join("audio.mp4"), b"not really audio").unwrap();
    folder
}

fn segment(text: &str) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("seg-{text}"),
        text: text.to_string(),
        timestamp: "2026-09-22T10:00:00Z".to_string(),
        audio_start_time: Some(0.0),
        audio_end_time: Some(1.0),
        duration: Some(1.0),
        speaker: None,
        channel: None,
        word_timestamps: None,
    }
}

/// The process-wide recordings roots (current + one earlier folder). They live in statics,
/// so every test that needs them shares ONE pair, set once.
fn roots() -> &'static (PathBuf, PathBuf) {
    static ROOTS: OnceLock<(tempfile::TempDir, tempfile::TempDir, (PathBuf, PathBuf))> =
        OnceLock::new();
    let (_, _, pair) = ROOTS.get_or_init(|| {
        let current = tempfile::tempdir().unwrap();
        let earlier = tempfile::tempdir().unwrap();
        let pair = (
            current.path().canonicalize().unwrap(),
            earlier.path().canonicalize().unwrap(),
        );
        app_lib::audio::recording_preferences::set_recordings_root(pair.0.clone());
        app_lib::audio::recording_preferences::set_previous_recording_roots(vec![pair.1.clone()]);
        (current, earlier, pair)
    });
    pair
}

// -- stale write-back guard (task 9, sabotage #4) ---------------------------------------

#[tokio::test]
async fn a_stale_folder_path_never_replaces_a_live_one() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let live = recording_folder(tmp.path(), "moved-here", None);
    let stale = tmp.path().join("was-here-before-the-move"); // gone
    let id = meeting_with_folder(&pool, Some(&live)).await;

    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        &pool,
        &id,
        "Weekly sync",
        &[segment("hello")],
        Some(stale.to_string_lossy().to_string()),
    )
    .await
    .unwrap();
    assert!(attached, "the transcript itself is still saved");
    assert_eq!(
        stored_folder(&pool, &id).await.as_deref(),
        Some(live.to_string_lossy().as_ref()),
        "a stale path from the frontend must not undo a completed move"
    );

    // The resume append path is guarded the same way.
    TranscriptsRepository::append_transcripts_for_meeting(
        &pool,
        &id,
        &[segment("again")],
        Some(stale.to_string_lossy().to_string()),
        1.0,
    )
    .await
    .unwrap();
    assert_eq!(
        stored_folder(&pool, &id).await.as_deref(),
        Some(live.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn a_supplied_folder_path_fills_a_null_or_dead_one() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let real = recording_folder(tmp.path(), "real", None);
    let real_str = real.to_string_lossy().to_string();

    // NULL → set.
    let id = meeting_with_folder(&pool, None).await;
    TranscriptsRepository::save_transcripts_for_meeting(
        &pool,
        &id,
        "t",
        &[segment("a")],
        Some(real_str.clone()),
    )
    .await
    .unwrap();
    assert_eq!(stored_folder(&pool, &id).await, Some(real_str.clone()));

    // A stored path that no longer exists → replaced.
    let dead = meeting_with_folder(&pool, Some(&tmp.path().join("deleted"))).await;
    TranscriptsRepository::save_transcripts_for_meeting(
        &pool,
        &dead,
        "t",
        &[segment("b")],
        Some(real_str.clone()),
    )
    .await
    .unwrap();
    assert_eq!(stored_folder(&pool, &dead).await, Some(real_str));
}

#[tokio::test]
async fn a_folder_path_write_waits_for_the_folder_lease() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let folder = recording_folder(tmp.path(), "f", None);
    let id = meeting_with_folder(&pool, None).await;

    let mover = folder_lease::acquire(&id, LeaseHolder::Mover).await;
    let save = {
        let (pool, id) = (pool.clone(), id.clone());
        let path = folder.to_string_lossy().to_string();
        tokio::spawn(async move {
            TranscriptsRepository::save_transcripts_for_meeting(
                &pool,
                &id,
                "t",
                &[segment("x")],
                Some(path),
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!save.is_finished(), "the save must wait for the mover");
    assert_eq!(stored_folder(&pool, &id).await, None);
    drop(mover);
    assert!(save.await.unwrap().unwrap());
    assert!(stored_folder(&pool, &id).await.is_some());
}

// -- retranscription resolves its folder from the DB (task 8, sabotage #3) --------------

#[tokio::test]
async fn retranscription_uses_the_stored_folder_not_the_callers_stale_path() {
    let (app, pool, _db) = app_with_db().await;
    let tmp = tempfile::tempdir().unwrap();
    // The row points at where the folder lives now; it holds no audio, so the job stops
    // right after resolving the folder — no model needed — and its error names the folder
    // it actually used.
    let moved = tmp.path().join("after-move");
    std::fs::create_dir_all(&moved).unwrap();
    let stale = tmp.path().join("before-move");
    let id = meeting_with_folder(&pool, Some(&moved)).await;

    let err = app_lib::audio::retranscription::start_retranscription(
        app.handle().clone(),
        id.clone(),
        stale.to_string_lossy().to_string(),
        None,
        None,
        None,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains(&moved.display().to_string()),
        "retranscription must resolve the folder from the DB, got: {err}"
    );
    assert_eq!(
        folder_lease::current_holder(&id),
        None,
        "the lease is released when the job ends"
    );
}

// -- recording start re-reads a resume folder under its lease ---------------------------

#[tokio::test]
async fn a_resumed_recording_uses_the_stored_folder_and_holds_the_lease() {
    let (app, pool, _db) = app_with_db().await;
    let tmp = tempfile::tempdir().unwrap();
    let moved = recording_folder(tmp.path(), "after-move", None);
    let id = meeting_with_folder(&pool, Some(&moved)).await;

    let (lease, folder) = folder_lease::lease_for_recording_start(
        app.handle(),
        Some(&id),
        Some(tmp.path().join("before-move").to_string_lossy().to_string()),
    )
    .await
    .unwrap();
    assert_eq!(folder, Some(moved.to_string_lossy().to_string()));
    assert_eq!(
        folder_lease::current_holder(&id),
        Some(LeaseHolder::Recording)
    );
    drop(lease);

    // A recording with no meeting row takes no lease and keeps its folder as given.
    let (lease, folder) =
        folder_lease::lease_for_recording_start(app.handle(), None, Some("/x".into()))
            .await
            .unwrap();
    assert!(lease.is_none());
    assert_eq!(folder.as_deref(), Some("/x"));
}

// -- retention sweep skips a leased meeting (task 8) ------------------------------------

#[tokio::test]
async fn the_retention_sweep_skips_a_busy_meeting_and_follows_a_moved_one() {
    let pool = pool_with_schema().await;
    let tmp = tempfile::tempdir().unwrap();
    let before = recording_folder(tmp.path(), "before", None);
    let id = meeting_with_folder(&pool, Some(&before)).await;

    let mover = folder_lease::acquire(&id, LeaseHolder::Mover).await;
    let skipped = app_lib::audio::retention::purge_meeting_media(&pool, &id)
        .await
        .unwrap();
    assert!(skipped.is_none(), "a leased meeting is skipped");
    assert!(
        before.join("audio.mp4").exists(),
        "nothing is deleted under a mover"
    );

    // The mover finishes: the folder is somewhere else now and the row says so.
    let after = tmp.path().join("after");
    std::fs::rename(&before, &after).unwrap();
    MeetingsRepository::update_folder_path(&pool, &id, &after.to_string_lossy())
        .await
        .unwrap();
    drop(mover);

    let stats = app_lib::audio::retention::purge_meeting_media(&pool, &id)
        .await
        .unwrap()
        .expect("purged once the lease is free");
    assert_eq!(stats.files_removed, 1);
    assert!(!after.join("audio.mp4").exists());
    assert!(
        after.join("metadata.json").exists(),
        "only media is removed"
    );
}

// -- delete removes a folder in an earlier root (task 4, sabotage #5) -------------------

#[tokio::test]
async fn deleting_a_meeting_removes_its_folder_in_an_earlier_recordings_root() {
    let (current, earlier) = roots();
    let (app, pool, _db) = app_with_db().await;

    // A folder that names its meeting, and one from before specs/0037 with no id: both
    // live in the EARLIER root, where the old "under the current root" check skipped them.
    let id = meeting_with_folder(&pool, None).await;
    let with_id = recording_folder(earlier, "delete-with-id", Some(&id));
    MeetingsRepository::update_folder_path(&pool, &id, &with_id.to_string_lossy())
        .await
        .unwrap();
    let legacy = meeting_with_folder(&pool, None).await;
    let without_id = recording_folder(earlier, "delete-pre-0037", None);
    MeetingsRepository::update_folder_path(&pool, &legacy, &without_id.to_string_lossy())
        .await
        .unwrap();
    // And a row pointing at ANOTHER meeting's folder in the current root: never deleted.
    let wrong = meeting_with_folder(&pool, None).await;
    let someone_else = recording_folder(current, "delete-someone-else", Some("other-meeting"));
    MeetingsRepository::update_folder_path(&pool, &wrong, &someone_else.to_string_lossy())
        .await
        .unwrap();

    for meeting in [&id, &legacy, &wrong] {
        app_lib::meetings::api_delete_meeting(
            app.handle().clone(),
            app.state::<AppState>(),
            meeting.clone(),
        )
        .await
        .unwrap();
    }
    assert!(
        !with_id.exists(),
        "the folder naming the meeting is removed"
    );
    assert!(
        !without_id.exists(),
        "a pre-0037 folder inside a known root is removed"
    );
    assert!(
        someone_else.exists(),
        "a folder that names another meeting is left alone"
    );
    assert!(
        earlier.is_dir() && current.is_dir(),
        "roots are never removed"
    );
}

// -- the webview allow-list covers earlier roots (task 5) -------------------------------

#[tokio::test]
async fn the_webview_can_read_audio_from_an_earlier_root_and_nothing_else() {
    let (_, earlier) = roots();
    let app = tauri::test::mock_app();
    let folder = recording_folder(earlier, "fs-guard", Some("m-fs"));

    let bytes = app_lib::fs_guard::read_audio_file(
        app.handle().clone(),
        folder.join("audio.mp4").to_string_lossy().to_string(),
    )
    .await
    .expect("an earlier recordings root is allowed");
    assert_eq!(bytes, b"not really audio");

    let outside = tempfile::tempdir().unwrap();
    let stray = outside.path().join("secret.txt");
    std::fs::write(&stray, b"x").unwrap();
    let denied =
        app_lib::fs_guard::read_audio_file(app.handle().clone(), stray.to_string_lossy().into())
            .await;
    assert!(
        denied.is_err(),
        "anything outside the known roots is denied"
    );
}
