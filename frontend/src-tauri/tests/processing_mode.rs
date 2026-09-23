//! Repo-level coverage for `meetings.processing_mode` (low-power-mode spec §3):
//! set/get round-trip, clearing back to NULL, invalid-value rejection, and the
//! "unknown meeting" setter/getter contract. Lives outside `crud.rs` to keep
//! that file under the 800-line ratchet (specs/0042).

use app_lib::database::repositories::meeting::MeetingsRepository;
use sqlx::SqlitePool;

async fn pool_with_schema() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn processing_mode_get_set_roundtrip() {
    let pool = pool_with_schema().await;
    let id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    // Fresh meeting: no override (NULL = "follow the global decision").
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &id)
            .await
            .unwrap(),
        None
    );

    // Set + read back 'live'.
    assert!(
        MeetingsRepository::set_processing_mode(&pool, &id, Some("live"))
            .await
            .unwrap()
    );
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &id)
            .await
            .unwrap(),
        Some("live".to_string())
    );

    // Overwrite with 'defer'.
    assert!(
        MeetingsRepository::set_processing_mode(&pool, &id, Some("defer"))
            .await
            .unwrap()
    );
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &id)
            .await
            .unwrap(),
        Some("defer".to_string())
    );

    // The metadata read (MeetingModel) carries the column too.
    let meta = MeetingsRepository::get_meeting_metadata(&pool, &id)
        .await
        .unwrap()
        .expect("meeting exists");
    assert_eq!(meta.processing_mode.as_deref(), Some("defer"));
}

#[tokio::test]
async fn processing_mode_none_and_blank_clear_to_null() {
    let pool = pool_with_schema().await;
    let id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    assert!(
        MeetingsRepository::set_processing_mode(&pool, &id, Some("live"))
            .await
            .unwrap()
    );

    // None clears.
    assert!(MeetingsRepository::set_processing_mode(&pool, &id, None)
        .await
        .unwrap());
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &id)
            .await
            .unwrap(),
        None
    );

    // Set again, then a blank/whitespace-only string also normalizes to a clear.
    assert!(
        MeetingsRepository::set_processing_mode(&pool, &id, Some("defer"))
            .await
            .unwrap()
    );
    assert!(
        MeetingsRepository::set_processing_mode(&pool, &id, Some("  "))
            .await
            .unwrap()
    );
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &id)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn processing_mode_rejects_invalid_value() {
    let pool = pool_with_schema().await;
    let id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    let err = MeetingsRepository::set_processing_mode(&pool, &id, Some("bogus"))
        .await
        .expect_err("invalid mode must be rejected");
    assert!(matches!(err, sqlx::Error::Protocol(_)));

    // Rejected write must not have touched the row.
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &id)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn processing_mode_unknown_meeting() {
    let pool = pool_with_schema().await;

    // Setter on an unknown meeting reports "no row updated" rather than erroring.
    assert!(
        !MeetingsRepository::set_processing_mode(&pool, "meeting-missing", Some("live"))
            .await
            .unwrap()
    );

    // Getter on an unknown meeting is simply None (indistinguishable from "no override").
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, "meeting-missing")
            .await
            .unwrap(),
        None
    );

    // Empty meeting_id is rejected outright (matches the other repo methods).
    assert!(MeetingsRepository::set_processing_mode(&pool, "  ", None)
        .await
        .is_err());
    assert!(MeetingsRepository::get_processing_mode(&pool, "")
        .await
        .is_err());
}

/// Inserts `count` throwaway transcript rows for `meeting_id`, to push a
/// meeting's transcript_count past `MIN_TRANSCRIPT_SEGMENTS` in the tests below.
async fn seed_transcripts(pool: &SqlitePool, meeting_id: &str, count: i64) {
    for i in 0..count {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, ?, ?)",
        )
        .bind(format!("t-{meeting_id}-{i}"))
        .bind(meeting_id)
        .bind(format!("segment {i}"))
        .bind(chrono::Utc::now())
        .execute(pool)
        .await
        .unwrap();
    }
}

/// Inserts a `completed` summary_processes row for `meeting_id` — the durable
/// evidence that the meeting already went through full processing.
async fn seed_completed_summary(pool: &SqlitePool, meeting_id: &str) {
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO summary_processes (meeting_id, status, result, created_at, updated_at) \
         VALUES (?, 'completed', '{\"markdown\":\"x\"}', ?, ?)",
    )
    .bind(meeting_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();
}

/// `MeetingsRepository::list_deferred_candidates` (low-power-mode spec §5):
/// surfaces meetings with recorded audio whose processing is still pending —
/// either explicitly deferred or effectively untranscribed (sparse transcript,
/// same threshold as the retention exemption) — and excludes everything else.
#[tokio::test]
async fn list_deferred_candidates_covers_the_matrix() {
    use app_lib::audio::lifecycle::MIN_TRANSCRIPT_SEGMENTS;

    let pool = pool_with_schema().await;

    // 1. Sparse transcript + folder → listed (effectively untranscribed).
    let sparse = MeetingsRepository::create_meeting(
        &pool,
        Some("Sparse".to_string()),
        Some("/tmp/sparse".to_string()),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    seed_transcripts(&pool, &sparse, MIN_TRANSCRIPT_SEGMENTS - 1).await;

    // 2. Explicit 'defer' + many transcripts + folder → listed (exempt override).
    let deferred = MeetingsRepository::create_meeting(
        &pool,
        Some("Deferred".to_string()),
        Some("/tmp/deferred".to_string()),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    seed_transcripts(&pool, &deferred, MIN_TRANSCRIPT_SEGMENTS + 20).await;
    assert!(
        MeetingsRepository::set_processing_mode(&pool, &deferred, Some("defer"))
            .await
            .unwrap()
    );

    // 3. NULL mode + many transcripts → not listed (fully processed, live).
    let done = MeetingsRepository::create_meeting(
        &pool,
        Some("Done".to_string()),
        Some("/tmp/done".to_string()),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    seed_transcripts(&pool, &done, MIN_TRANSCRIPT_SEGMENTS + 20).await;

    // 4. No folder_path → not listed, even though its transcript is sparse.
    let no_folder = MeetingsRepository::create_meeting(
        &pool,
        Some("NoFolder".to_string()),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    seed_transcripts(&pool, &no_folder, 0).await;

    // 5. Sparse transcript + folder BUT a completed summary → NOT listed. A
    //    legitimately short, fully-processed meeting must not re-list on every AC
    //    transition and re-run retranscription/diarization/summary forever
    //    (low-power-mode Finding 3). The completed summary is the durable trace.
    let short_summarized = MeetingsRepository::create_meeting(
        &pool,
        Some("ShortSummarized".to_string()),
        Some("/tmp/short".to_string()),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    seed_transcripts(&pool, &short_summarized, MIN_TRANSCRIPT_SEGMENTS - 1).await;
    seed_completed_summary(&pool, &short_summarized).await;

    // 6. Explicit 'defer' + a completed summary → STILL listed. The explicit-defer
    //    arm is unconditional; a completed summary never suppresses it.
    let deferred_summarized = MeetingsRepository::create_meeting(
        &pool,
        Some("DeferredSummarized".to_string()),
        Some("/tmp/deferred-sum".to_string()),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    seed_transcripts(&pool, &deferred_summarized, 1).await;
    seed_completed_summary(&pool, &deferred_summarized).await;
    assert!(
        MeetingsRepository::set_processing_mode(&pool, &deferred_summarized, Some("defer"))
            .await
            .unwrap()
    );

    let candidates = MeetingsRepository::list_deferred_candidates(&pool)
        .await
        .unwrap();
    let ids: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();

    assert!(
        ids.contains(&sparse.as_str()),
        "sparse+folder must be listed"
    );
    assert!(
        ids.contains(&deferred.as_str()),
        "explicit defer+folder must be listed"
    );
    assert!(
        !ids.contains(&done.as_str()),
        "NULL mode + many transcripts must not be listed"
    );
    assert!(
        !ids.contains(&no_folder.as_str()),
        "no folder_path must not be listed"
    );
    assert!(
        !ids.contains(&short_summarized.as_str()),
        "sparse but summarized meeting must NOT be listed (Finding 3)"
    );
    assert!(
        ids.contains(&deferred_summarized.as_str()),
        "explicit defer stays listed even with a completed summary"
    );
    assert_eq!(candidates.len(), 3);
}

/// spec 0051 WS2: a meeting stranded at processing_mode='live' (its stop-time handoff
/// never completed) is invisible to `list_deferred_candidates`, which matches only
/// 'defer' or sparse-transcript meetings. The startup sweep rewrites it to 'defer' so
/// the backlog can pick it up. 'defer' and NULL rows are left alone.
#[tokio::test]
async fn startup_reconciliation_rewrites_live_to_defer() {
    let pool = pool_with_schema().await;

    let stranded = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .expect("create stranded meeting");
    let already_deferred = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .expect("create deferred meeting");
    let untouched = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .expect("create normal meeting");

    MeetingsRepository::set_processing_mode(&pool, &stranded, Some("live"))
        .await
        .unwrap();
    MeetingsRepository::set_processing_mode(&pool, &already_deferred, Some("defer"))
        .await
        .unwrap();

    let rewritten =
        app_lib::audio::processing_reconcile::reconcile_stranded_live_markers(&pool, &[])
            .await
            .expect("reconcile");
    assert_eq!(rewritten, 1, "only the 'live' row is rewritten");

    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &stranded)
            .await
            .unwrap(),
        Some("defer".to_string()),
        "the stranded meeting becomes reclaimable by the backlog"
    );
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &already_deferred)
            .await
            .unwrap(),
        Some("defer".to_string()),
    );
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &untouched)
            .await
            .unwrap(),
        None,
        "a meeting with no marker is left alone"
    );
}

/// Idempotent: a second pass finds nothing to do.
#[tokio::test]
async fn startup_reconciliation_is_idempotent() {
    let pool = pool_with_schema().await;
    let id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .expect("create meeting");
    MeetingsRepository::set_processing_mode(&pool, &id, Some("live"))
        .await
        .unwrap();

    assert_eq!(
        app_lib::audio::processing_reconcile::reconcile_stranded_live_markers(&pool, &[])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        app_lib::audio::processing_reconcile::reconcile_stranded_live_markers(&pool, &[])
            .await
            .unwrap(),
        0
    );
}

/// spec 0051 WS2 fix round 1 — Important finding: `spawn_startup_reconciliation` used
/// `app.state::<AppState>()`, which *panics* if `AppState` isn't managed yet. That is
/// the normal state during the first-launch onboarding window (see
/// `database::setup::initialize_database_on_startup`), not an edge case, so every
/// fresh install hit the panic. This exercises the wrapper itself (not just the pure
/// `reconcile_stranded_live_markers`) against a `tauri::test::mock_app()` with NO
/// `AppState` managed — the exact first-launch race — and asserts the spawned task
/// completes without panicking (`JoinHandle::await` surfaces a `JoinError` if the
/// task panicked, so this fails loudly if `try_state` regresses back to `state()`).
#[tokio::test]
async fn spawn_startup_reconciliation_skips_when_database_not_initialized() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();

    let join = app_lib::audio::processing_reconcile::spawn_startup_reconciliation(handle);
    assert!(
        join.await.is_ok(),
        "the sweep must not panic when AppState isn't managed yet (first-launch race)"
    );
}

/// Companion to the skip test above: with `AppState` actually managed (the normal,
/// post-onboarding case), the wrapper must still do real work — pull the pool out of
/// `AppState` and delegate to `reconcile_stranded_live_markers`. Exercises the full
/// `AppHandle` -> `try_state` -> pool -> UPDATE path end to end, not just the pure
/// SQL function against a raw pool.
#[tokio::test]
async fn spawn_startup_reconciliation_rewrites_when_database_initialized() {
    use app_lib::database::manager::DatabaseManager;
    use app_lib::state::AppState;
    use tauri::Manager;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir
        .path()
        .join("startup-reconcile-test.sqlite")
        .to_string_lossy()
        .to_string();
    let db_manager = DatabaseManager::new(&db_path, &db_path)
        .await
        .expect("create real (file-backed, migrated) DatabaseManager");
    let pool = db_manager.pool().clone();

    let stranded = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .expect("create stranded meeting");
    MeetingsRepository::set_processing_mode(&pool, &stranded, Some("live"))
        .await
        .unwrap();

    let app = tauri::test::mock_app();
    app.handle().manage(AppState { db_manager });
    let handle = app.handle().clone();

    app_lib::audio::processing_reconcile::spawn_startup_reconciliation(handle)
        .await
        .expect("sweep task must not panic when AppState is managed");

    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &stranded)
            .await
            .unwrap(),
        Some("defer".to_string()),
        "the wrapper must actually reconcile via the pool it pulled from AppState"
    );
}

/// spec 0051 final review, Finding 2 — the SAME first-launch panic Task 3's review fixed
/// in `processing_reconcile`, newly exposed on a second command by Task 5's provider
/// hoist: `DeferredBacklogProvider` is now mounted unconditionally, so
/// `useDeferredBacklog`'s mount effect invokes `api_list_deferred_meetings` ~5s into
/// EVERY launch — including the whole first-launch onboarding window, during which
/// `AppState` is unmanaged (`database::setup::initialize_database_on_startup` defers
/// `manage()` to a later, frontend-triggered command).
///
/// `app.state::<AppState>()` panics there. Unwinding means the app survives, but the IPC
/// response is never sent: the frontend `try/catch` never fires and the promise hangs
/// forever, silently disabling the backlog for the session. Exercised against a
/// `tauri::test::mock_app()` with NO `AppState` managed — the exact first-launch race.
#[tokio::test]
async fn list_deferred_meetings_is_empty_when_database_not_initialized() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();

    // The command itself, not a pure helper: a panic here fails the test (and is exactly
    // what `state()` does), and a non-empty/erroring result would be wrong too.
    let listed = app_lib::audio::deferred_backlog::api_list_deferred_meetings(handle)
        .await
        .expect("unmanaged AppState must be answered, not errored");
    assert!(
        listed.is_empty(),
        "no database means no meetings; the backlog must get an empty list"
    );
}

/// Companion: with `AppState` managed (the normal case), the command must still do real
/// work — pull the pool out of `AppState` and return the deferred candidates. Without
/// this, `Ok(vec![])` from `try_state` would be indistinguishable from a command that
/// always returns nothing.
#[tokio::test]
async fn list_deferred_meetings_returns_candidates_when_database_initialized() {
    use app_lib::database::manager::DatabaseManager;
    use app_lib::state::AppState;
    use tauri::Manager;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir
        .path()
        .join("deferred-list-test.sqlite")
        .to_string_lossy()
        .to_string();
    let db_manager = DatabaseManager::new(&db_path, &db_path)
        .await
        .expect("create real (file-backed, migrated) DatabaseManager");
    let pool = db_manager.pool().clone();

    // The command filters to folders that still hold audio, so give it a real one.
    let folder = dir.path().join("Deferred_2026-08-14_10-00");
    std::fs::create_dir_all(&folder).expect("create meeting folder");
    std::fs::write(folder.join("recording.wav"), b"not really audio").expect("write audio");

    let deferred = MeetingsRepository::create_meeting(
        &pool,
        Some("Deferred".to_string()),
        Some(folder.to_string_lossy().to_string()),
        None,
        None,
        None,
    )
    .await
    .expect("create deferred meeting");
    MeetingsRepository::set_processing_mode(&pool, &deferred, Some("defer"))
        .await
        .unwrap();

    let app = tauri::test::mock_app();
    app.handle().manage(AppState { db_manager });
    let handle = app.handle().clone();

    let listed = app_lib::audio::deferred_backlog::api_list_deferred_meetings(handle)
        .await
        .expect("list deferred meetings");
    assert_eq!(
        listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec![deferred.as_str()],
        "the command must actually query through the pool it pulled from AppState"
    );
}

/// Write a recording folder for `meeting_id` with the given metadata `status`, plus the
/// `.checkpoints/` dir that marks it as holding resumable audio (specs/0037).
fn write_recording_folder(root: &std::path::Path, meeting_id: &str, status: &str) {
    let folder = root.join(format!("Meeting_{meeting_id}"));
    std::fs::create_dir_all(folder.join(".checkpoints")).unwrap();
    std::fs::write(
        folder.join("metadata.json"),
        format!(
            r#"{{"meeting_id":"{meeting_id}","meeting_name":"M","created_at":"2026-08-14T10:00:00Z","status":"{status}","segments":[]}}"#
        ),
    )
    .unwrap();
}

/// spec 0051 final review, Finding 3 — the startup sweep vs crash-resume (specs/0037).
///
/// The sweep's premise ("no recording can be in progress at startup") holds for RUNNING
/// recordings but not for INTERRUPTED ones: `ResumeRecordingPrompt` offers to continue a
/// crashed recording into the same meeting id and folder. Rewriting its `'live'` marker
/// to `'defer'` before the user answers would (a) silently restart the resumed session in
/// record-only mode (`decide_session_mode` reads the marker at start; `effective_live_stt`
/// maps Some("defer") => false) and (b) let the backlog retranscribe the meeting
/// underneath the resumed recording — `'defer'` matches `list_deferred_candidates`
/// unconditionally and the backlog auto-drains 5s after launch on AC, with no
/// recording-state dependency by design.
#[tokio::test]
async fn startup_sweep_leaves_interrupted_recordings_alone() {
    let pool = pool_with_schema().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // A crash-interrupted, still-resumable recording that was toggled Live mid-meeting.
    let interrupted = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .unwrap();
    MeetingsRepository::set_processing_mode(&pool, &interrupted, Some("live"))
        .await
        .unwrap();
    write_recording_folder(root, &interrupted, "recording");

    // A genuinely stranded meeting: its folder finalized cleanly, but the stop-time
    // handoff never cleared the marker. This is what the sweep exists for.
    let stranded = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .unwrap();
    MeetingsRepository::set_processing_mode(&pool, &stranded, Some("live"))
        .await
        .unwrap();
    write_recording_folder(root, &stranded, "completed");

    let rewritten =
        app_lib::audio::processing_reconcile::reconcile_at_startup(&pool, &[root.to_path_buf()])
            .await
            .expect("startup sweep");
    assert_eq!(rewritten, 1, "only the finalized meeting may be rewritten");

    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &interrupted)
            .await
            .unwrap(),
        Some("live".to_string()),
        "a crash-interrupted recording keeps its Live marker until the resume prompt is \
         answered — otherwise the resumed session downgrades to record-only AND the \
         backlog can retranscribe it underneath the live recording"
    );
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &stranded)
            .await
            .unwrap(),
        Some("defer".to_string()),
        "a finalized-but-stranded meeting is still returned to the backlog"
    );
}

/// Once the interrupted recording is finalized — the user resumed and stopped cleanly, or
/// declined the resume ("discarded") — the next launch's sweep reclaims it as usual. The
/// skip is a deferral, not an exemption.
#[tokio::test]
async fn startup_sweep_reclaims_a_finalized_recording_on_the_next_launch() {
    let pool = pool_with_schema().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    let id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .unwrap();
    MeetingsRepository::set_processing_mode(&pool, &id, Some("live"))
        .await
        .unwrap();
    write_recording_folder(root, &id, "recording");

    assert_eq!(
        app_lib::audio::processing_reconcile::reconcile_at_startup(&pool, &[root.to_path_buf()])
            .await
            .unwrap(),
        0,
        "launch 1: still interrupted, left alone"
    );

    // The user declined the resume; specs/0037 flips the folder to "discarded".
    write_recording_folder(root, &id, "discarded");

    assert_eq!(
        app_lib::audio::processing_reconcile::reconcile_at_startup(&pool, &[root.to_path_buf()])
            .await
            .unwrap(),
        1,
        "launch 2: no longer interrupted, so the sweep reclaims it"
    );
    assert_eq!(
        MeetingsRepository::get_processing_mode(&pool, &id)
            .await
            .unwrap(),
        Some("defer".to_string()),
    );
}

/// A missing recordings root (no recordings yet) must not stop the sweep.
#[tokio::test]
async fn startup_sweep_tolerates_a_missing_recordings_root() {
    let pool = pool_with_schema().await;
    let id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .unwrap();
    MeetingsRepository::set_processing_mode(&pool, &id, Some("live"))
        .await
        .unwrap();

    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no-recordings-here");
    assert_eq!(
        app_lib::audio::processing_reconcile::reconcile_at_startup(&pool, &[missing])
            .await
            .unwrap(),
        1
    );
}

/// specs/0073 W1: an interrupted recording in an EARLIER recordings folder (the user
/// changed the folder since) is skipped exactly like one in the current folder.
#[tokio::test]
async fn startup_sweep_skips_an_interrupted_recording_in_an_earlier_root() {
    let pool = pool_with_schema().await;
    let current = tempfile::tempdir().expect("tempdir");
    let earlier = tempfile::tempdir().expect("tempdir");

    let id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
        .await
        .unwrap();
    MeetingsRepository::set_processing_mode(&pool, &id, Some("live"))
        .await
        .unwrap();
    write_recording_folder(earlier.path(), &id, "recording");

    let roots = [current.path().to_path_buf(), earlier.path().to_path_buf()];
    assert_eq!(
        app_lib::audio::processing_reconcile::reconcile_at_startup(&pool, &roots)
            .await
            .unwrap(),
        0,
        "the interrupted recording in the earlier root keeps its Live marker"
    );
}
