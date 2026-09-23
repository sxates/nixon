//! specs/0073 W2 — the recordings mover, end to end on temp folders and a real SQLite
//! database: same- and cross-volume moves, every crash point of the recovery table, the
//! journal resume, the folder lease, the prechecks, the startup gather, the end-of-run
//! removal of emptied folders, and the rule that a debug build never touches the
//! production recordings folder.
//!
//! Run with: cargo test --features metal --test recordings_move

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use app_lib::audio::folder_lease::{self, LeaseHolder};
use app_lib::audio::recordings_move::exec::{Crashed, ExecOptions, FailPoint};
use app_lib::audio::recordings_move::journal::{self, Journal, JOURNAL_FILE};
use app_lib::audio::recordings_move::plan::staging_path;
use app_lib::audio::recordings_move::roots::{roots_for_profile, MoveRoots, ProfileRootsInput};
use app_lib::audio::recordings_move::runner::{
    build_plan, gather, precheck, resume_from_journal, start_run, GatherDecision, MoveEnv,
    MoveFinished, MoveReporter, MoveState, MoveStatus, NoReporter, RunError, JOURNAL_WRITE_FAILED,
    STOP_RECORDING_FIRST,
};
use app_lib::database::repositories::meeting::MeetingsRepository;
use sqlx::SqlitePool;

// -- harness ------------------------------------------------------------------------------

async fn pool_with_schema() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

struct Harness {
    pool: SqlitePool,
    tmp: tempfile::TempDir,
    old: PathBuf,
    new: PathBuf,
    app_data: PathBuf,
    journal: PathBuf,
    roots: MoveRoots,
    state: &'static MoveState,
}

impl Harness {
    /// A release-profile move from `old/` into `new/`.
    async fn new() -> Self {
        let pool = pool_with_schema().await;
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let (old, new, app_data) = (base.join("old"), base.join("new"), base.join("app-data"));
        for d in [&old, &new, &app_data] {
            std::fs::create_dir_all(d).unwrap();
        }
        let roots = release_roots(&new, &old, &[], &[]);
        Harness {
            pool,
            journal: app_data.join(JOURNAL_FILE),
            tmp,
            old,
            new,
            app_data,
            roots,
            state: Box::leak(Box::new(MoveState::new())),
        }
    }

    fn base(&self) -> PathBuf {
        self.tmp.path().canonicalize().unwrap()
    }

    fn env<'a>(&'a self, opts: ExecOptions, reporter: &'a dyn MoveReporter) -> MoveEnv<'a> {
        MoveEnv {
            pool: &self.pool,
            roots: &self.roots,
            journal: &self.journal,
            app_data: &self.app_data,
            opts,
            state: self.state,
            reporter,
        }
    }

    /// Plan and run a move into `roots.target`.
    async fn run(&self, opts: ExecOptions) -> Result<MoveFinished, Crashed> {
        let plan = build_plan(&self.pool, &self.roots).await.unwrap();
        let guard = self.state.try_begin().unwrap();
        start_run(&self.env(opts, &NoReporter), &plan, guard)
            .await
            .map_err(|e| match e {
                RunError::Crashed(c) => c,
                other => panic!("the run didn't start: {other:?}"),
            })
    }

    async fn resume(&self) -> Option<MoveFinished> {
        let guard = self.state.try_begin().unwrap();
        resume_from_journal(&self.env(ExecOptions::default(), &NoReporter), guard)
            .await
            .unwrap()
    }
}

fn release_roots(
    target: &Path,
    current: &Path,
    previous: &[PathBuf],
    defaults: &[PathBuf],
) -> MoveRoots {
    roots_for_profile(ProfileRootsInput {
        dev: false,
        target: target.to_path_buf(),
        current: current.to_path_buf(),
        previous: previous.to_vec(),
        platform_defaults: defaults.to_vec(),
        release_root: target.to_path_buf(),
        known: Vec::new(),
    })
}

/// A meeting row with a recording folder `root/name` holding a few files.
async fn meeting(pool: &SqlitePool, root: &Path, name: &str) -> (String, PathBuf) {
    let folder = root.join(name);
    let id = MeetingsRepository::create_meeting(
        pool,
        Some(name.to_string()),
        Some(folder.to_string_lossy().to_string()),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    write_folder(&folder, &id, "completed");
    (id, folder)
}

fn write_folder(folder: &Path, id: &str, status: &str) {
    std::fs::create_dir_all(folder.join(".checkpoints")).unwrap();
    std::fs::write(
        folder.join("metadata.json"),
        format!(r#"{{"meeting_id":"{id}","status":"{status}","audio_file":"audio.mp4"}}"#),
    )
    .unwrap();
    std::fs::write(
        folder.join("audio.mp4"),
        format!("audio of {id}").repeat(50),
    )
    .unwrap();
    std::fs::write(folder.join("transcripts.json"), r#"{"segments":[]}"#).unwrap();
    std::fs::write(folder.join(".checkpoints/chunk_0.mp4"), b"chunk").unwrap();
}

/// Every file under `folder`, relative path → contents.
fn contents(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(base).unwrap().to_path_buf(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(folder, folder, &mut out);
    out
}

async fn stored_folder(pool: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// The meeting ended up at `dst` with exactly its original files, once.
async fn assert_moved(
    h: &Harness,
    id: &str,
    src: &Path,
    dst: &Path,
    original: &BTreeMap<PathBuf, Vec<u8>>,
) {
    assert_eq!(
        stored_folder(&h.pool, id).await,
        Some(s(dst)),
        "the row names the new folder"
    );
    assert!(!src.exists(), "the old folder is gone: {}", src.display());
    assert_eq!(
        &contents(dst),
        original,
        "every file arrived, byte for byte"
    );
    assert!(!staging_path(dst).exists(), "no staging copy is left");
}

/// The meeting is untouched: row and folder where they were.
async fn assert_untouched(
    h: &Harness,
    id: &str,
    src: &Path,
    original: &BTreeMap<PathBuf, Vec<u8>>,
) {
    assert_eq!(
        stored_folder(&h.pool, id).await,
        Some(s(src)),
        "the row still names the old folder"
    );
    assert_eq!(&contents(src), original, "the old folder is intact");
}

// -- moves --------------------------------------------------------------------------------

#[tokio::test]
async fn a_same_volume_move_takes_every_meeting_and_removes_the_emptied_root() {
    let h = Harness::new().await;
    let mut all = Vec::new();
    for name in ["Standup", "Planning", "Retro"] {
        let (id, src) = meeting(&h.pool, &h.old, name).await;
        let original = contents(&src);
        all.push((id, src, original));
    }
    std::fs::write(h.old.join(".DS_Store"), b"finder").unwrap();

    let finished = h.run(ExecOptions::default()).await.unwrap();

    assert_eq!(finished.moved, 3);
    assert!(finished.failed.is_empty());
    for (id, src, original) in &all {
        let dst = h.new.join(src.file_name().unwrap());
        assert_moved(&h, id, src, &dst, original).await;
    }
    assert!(!h.old.exists(), "a root holding only .DS_Store is removed");
    assert_eq!(finished.removed_roots, vec![h.old.clone()]);
    assert!(!h.journal.exists(), "the journal is removed at the end");
}

#[tokio::test]
async fn a_cross_volume_move_copies_verifies_and_removes_the_source() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Design review").await;
    let original = contents(&src);
    let opts = ExecOptions {
        force_cross_volume: true,
        ..ExecOptions::default()
    };
    let finished = h.run(opts).await.unwrap();
    assert_eq!(finished.moved, 1);
    assert_moved(&h, &id, &src, &h.new.join("Design review"), &original).await;
}

#[tokio::test]
async fn running_the_move_again_moves_nothing() {
    let h = Harness::new().await;
    meeting(&h.pool, &h.old, "Standup").await;
    h.run(ExecOptions::default()).await.unwrap();

    let plan = build_plan(&h.pool, &h.roots).await.unwrap();
    assert!(plan.is_empty());
    assert_eq!(plan.meetings, 0);
    let again = h.run(ExecOptions::default()).await.unwrap();
    assert_eq!((again.moved, again.skipped, again.failed.len()), (0, 0, 0));
}

#[tokio::test]
async fn a_shared_folder_moves_once_and_every_row_follows() {
    let h = Harness::new().await;
    let (first, src) = meeting(&h.pool, &h.old, "Weekly").await;
    let second = MeetingsRepository::create_meeting(
        &h.pool,
        Some("Weekly (continued)".into()),
        Some(s(&src)),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let original = contents(&src);

    let finished = h.run(ExecOptions::default()).await.unwrap();

    assert_eq!(finished.moved, 2, "both meetings count");
    let dst = h.new.join("Weekly");
    assert_moved(&h, &first, &src, &dst, &original).await;
    assert_eq!(stored_folder(&h.pool, &second).await, Some(s(&dst)));
}

#[tokio::test]
async fn a_name_already_taken_in_the_new_folder_gets_a_number() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Sync").await;
    let original = contents(&src);
    std::fs::create_dir_all(h.new.join("Sync")).unwrap();
    std::fs::write(h.new.join("Sync/unrelated.txt"), b"not ours").unwrap();

    h.run(ExecOptions::default()).await.unwrap();

    assert_moved(&h, &id, &src, &h.new.join("Sync (2)"), &original).await;
    assert!(
        h.new.join("Sync/unrelated.txt").exists(),
        "the other folder is untouched"
    );
}

#[tokio::test]
async fn a_copy_that_fails_verification_leaves_the_meeting_where_it_was() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Budget").await;
    let original = contents(&src);
    let opts = ExecOptions {
        force_cross_volume: true,
        truncate_copy: true,
        ..ExecOptions::default()
    };

    let finished = h.run(opts).await.unwrap();

    assert_eq!(finished.moved, 0);
    assert_eq!(finished.failed.len(), 1, "reported as failed");
    assert_eq!(finished.failed[0].meeting_id, id);
    assert_eq!(finished.failed[0].folder_path, src);
    assert_untouched(&h, &id, &src, &original).await;
    assert!(
        !h.new.join("Budget").exists(),
        "no half copy is left in the new folder"
    );
    assert!(
        !staging_path(&h.new.join("Budget")).exists(),
        "the staging copy is removed"
    );
    assert!(h.old.exists(), "a root that still holds a meeting is kept");
}

#[tokio::test]
async fn a_meeting_deleted_before_its_turn_is_skipped() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Gone soon").await;
    let plan = build_plan(&h.pool, &h.roots).await.unwrap();
    sqlx::query("DELETE FROM meetings WHERE id = ?")
        .bind(&id)
        .execute(&h.pool)
        .await
        .unwrap();
    let guard = h.state.try_begin().unwrap();
    let finished = start_run(&h.env(ExecOptions::default(), &NoReporter), &plan, guard)
        .await
        .unwrap();
    assert_eq!((finished.moved, finished.skipped), (0, 1));
    assert!(src.exists(), "a folder whose meeting is gone is not moved");
}

// -- crash points and the recovery table ---------------------------------------------------

/// Crash at `point`, check the on-disk state is the recovery-table row we expect, then
/// resume from the journal and check the meeting ended up moved, every file exactly once.
async fn crash_then_resume(cross: bool, point: FailPoint, expect: (bool, bool, bool, bool)) {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Crashy").await;
    let original = contents(&src);
    let dst = h.new.join("Crashy");
    let opts = ExecOptions {
        force_cross_volume: cross,
        fail_at: Some(point),
        ..ExecOptions::default()
    };

    let crashed = h.run(opts).await;
    assert_eq!(crashed, Err(Crashed(point)));
    assert!(h.journal.exists(), "a crash leaves the journal behind");

    let (src_e, staging_e, dst_e, row_at_dst) = expect;
    assert_eq!(src.exists(), src_e, "{point:?}: source exists?");
    assert_eq!(
        staging_path(&dst).exists(),
        staging_e,
        "{point:?}: staging exists?"
    );
    assert_eq!(dst.exists(), dst_e, "{point:?}: destination exists?");
    let row = stored_folder(&h.pool, &id).await;
    assert_eq!(
        row == Some(s(&dst)),
        row_at_dst,
        "{point:?}: row names the destination?"
    );

    let finished = h.resume().await.expect("the journal is resumed");
    assert_eq!(finished.moved, 1, "{point:?}: the resume finishes the move");
    assert!(
        finished.failed.is_empty(),
        "{point:?}: {:?}",
        finished.failed
    );
    assert_moved(&h, &id, &src, &dst, &original).await;
    assert!(
        !h.journal.exists(),
        "{point:?}: the journal is gone once reconciled"
    );
    assert!(
        !h.old.exists(),
        "{point:?}: the emptied old root is removed"
    );
}

// Recovery table row 2: src yes, staging yes, dst no, row src.
#[tokio::test]
async fn crash_after_copy_restarts_the_copy() {
    crash_then_resume(true, FailPoint::AfterCopy, (true, true, false, false)).await;
}

#[tokio::test]
async fn crash_after_verify_restarts_the_copy() {
    crash_then_resume(true, FailPoint::AfterVerify, (true, true, false, false)).await;
}

// Row 3: src yes, staging no, dst yes, row src → re-verify, update, remove source.
#[tokio::test]
async fn crash_after_the_cross_volume_rename_reverifies_and_finishes() {
    crash_then_resume(true, FailPoint::AfterRename, (true, false, true, false)).await;
}

// Row 4: src yes, dst yes, row dst → remove the source.
#[tokio::test]
async fn crash_after_the_row_update_removes_the_source() {
    crash_then_resume(true, FailPoint::AfterRowUpdate, (true, false, true, true)).await;
}

// Row 4 with a partly removed source.
#[tokio::test]
async fn crash_during_the_source_delete_finishes_the_delete() {
    crash_then_resume(
        true,
        FailPoint::DuringSourceDelete,
        (true, false, true, true),
    )
    .await;
}

// Row 5: src no, dst yes, row src → update the row.
#[tokio::test]
async fn crash_after_a_same_volume_rename_updates_the_row() {
    crash_then_resume(false, FailPoint::AfterRename, (false, false, true, false)).await;
}

// Row 6: src no, dst yes, row dst → done.
#[tokio::test]
async fn crash_after_a_same_volume_row_update_is_already_done() {
    crash_then_resume(false, FailPoint::AfterRowUpdate, (false, false, true, true)).await;
}

// Row 3 when the destination was damaged between runs: never delete the source.
#[tokio::test]
async fn a_damaged_destination_on_resume_keeps_the_source() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Tampered").await;
    let original = contents(&src);
    let opts = ExecOptions {
        force_cross_volume: true,
        fail_at: Some(FailPoint::AfterRename),
        ..ExecOptions::default()
    };
    assert!(h.run(opts).await.is_err());
    std::fs::write(h.new.join("Tampered/audio.mp4"), b"short").unwrap();

    let finished = h.resume().await.unwrap();

    assert_eq!(finished.moved, 0);
    assert_eq!(finished.failed.len(), 1);
    assert_untouched(&h, &id, &src, &original).await;
}

// Row 7: neither folder exists → reported, row left alone.
#[tokio::test]
async fn a_journaled_meeting_with_no_folder_anywhere_is_reported_and_left_alone() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Vanished").await;
    let plan = build_plan(&h.pool, &h.roots).await.unwrap();
    journal::write(&h.journal, &Journal::new(h.new.clone(), plan.units.clone())).unwrap();
    std::fs::remove_dir_all(&src).unwrap();

    let finished = h.resume().await.unwrap();

    assert_eq!(finished.failed.len(), 1);
    assert_eq!(
        stored_folder(&h.pool, &id).await,
        Some(s(&src)),
        "the row is left alone"
    );
    assert!(!h.journal.exists());
}

#[tokio::test]
async fn a_damaged_journal_still_reconciles_the_meetings_it_names() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Named").await;
    let original = contents(&src);
    let assert_err = h
        .run(ExecOptions {
            fail_at: Some(FailPoint::AfterRename),
            ..ExecOptions::default()
        })
        .await;
    assert!(assert_err.is_err());
    // Damage the journal: keep the unit, break the rest.
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&h.journal).unwrap()).unwrap();
    value["version"] = serde_json::json!("not a number");
    value["units"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"src": 1}));
    std::fs::write(&h.journal, value.to_string()).unwrap();

    let finished = h.resume().await.unwrap();

    assert_eq!(finished.moved, 1);
    assert_moved(&h, &id, &src, &h.new.join("Named"), &original).await;
}

#[tokio::test]
async fn resume_without_a_journal_does_nothing() {
    let h = Harness::new().await;
    meeting(&h.pool, &h.old, "Untouched").await;
    assert!(h.resume().await.is_none());
    assert!(h.old.join("Untouched").exists());
}

// -- the folder lease ---------------------------------------------------------------------

/// Records every progress update.
struct Recorder(Mutex<Vec<MoveStatus>>);

impl MoveReporter for Recorder {
    fn progress(&self, status: &MoveStatus) {
        self.0.lock().unwrap().push(status.clone());
    }
    fn finished(&self, _: &MoveFinished) {}
}

#[tokio::test]
async fn the_mover_waits_for_a_job_holding_the_meeting_and_says_so() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Busy").await;
    let original = contents(&src);
    let lease = folder_lease::acquire(&id, LeaseHolder::Retranscription).await;
    let recorder = Recorder(Mutex::new(Vec::new()));
    let plan = build_plan(&h.pool, &h.roots).await.unwrap();
    let guard = h.state.try_begin().unwrap();
    let env = h.env(ExecOptions::default(), &recorder);

    let run = start_run(&env, &plan, guard);
    tokio::pin!(run);
    let waited = tokio::time::timeout(std::time::Duration::from_millis(300), &mut run).await;
    assert!(
        waited.is_err(),
        "the move must wait for the retranscription"
    );
    assert!(src.exists(), "nothing moved while the job held the folder");
    assert_eq!(
        h.state.status().and_then(|s| s.waiting_for),
        Some(LeaseHolder::Retranscription),
        "the status says what it waits for"
    );

    drop(lease);
    let finished = run.await.unwrap();
    assert_eq!(finished.moved, 1);
    assert_moved(&h, &id, &src, &h.new.join("Busy"), &original).await;
    assert!(recorder
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.waiting_for == Some(LeaseHolder::Retranscription)));
}

#[tokio::test]
async fn a_meeting_being_recorded_into_is_skipped_not_waited_for() {
    let h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Live").await;
    let _lease = folder_lease::acquire(&id, LeaseHolder::Recording).await;
    let finished = h.run(ExecOptions::default()).await.unwrap();
    assert_eq!((finished.moved, finished.skipped), (0, 1));
    assert!(src.exists());
}

#[tokio::test]
async fn a_cancel_stops_between_meetings_and_leaves_the_rest_in_place() {
    struct CancelAfterFirst(&'static MoveState);
    impl MoveReporter for CancelAfterFirst {
        fn progress(&self, s: &MoveStatus) {
            if s.done >= 1 {
                self.0.cancel();
            }
        }
        fn finished(&self, _: &MoveFinished) {}
    }
    let h = Harness::new().await;
    for name in ["A", "B", "C"] {
        meeting(&h.pool, &h.old, name).await;
    }
    let plan = build_plan(&h.pool, &h.roots).await.unwrap();
    let guard = h.state.try_begin().unwrap();
    let reporter = CancelAfterFirst(h.state);
    let finished = start_run(&h.env(ExecOptions::default(), &reporter), &plan, guard)
        .await
        .unwrap();
    assert!(finished.cancelled);
    assert_eq!((finished.moved, finished.skipped), (1, 2));
    assert!(h.old.join("B").exists() && h.old.join("C").exists());
    assert!(h.old.exists(), "a root with meetings left in it is kept");
    assert!(!h.journal.exists(), "a cancelled run closes the journal");
    assert!(!h.state.is_running(), "the move slot is free again");
}

// -- prechecks ----------------------------------------------------------------------------

#[tokio::test]
async fn prechecks_refuse_recording_no_space_and_a_target_inside_a_meeting() {
    let h = Harness::new().await;
    let (_, src) = meeting(&h.pool, &h.old, "Inside").await;
    let plan = build_plan(&h.pool, &h.roots).await.unwrap();

    assert_eq!(
        precheck(&plan, &h.roots, true, &h.app_data),
        Err(STOP_RECORDING_FIRST.to_string())
    );

    let mut no_space = plan.clone();
    no_space.enough_space = false;
    no_space.cross_volume_bytes = 5 << 30;
    let err = precheck(&no_space, &h.roots, false, &h.app_data).unwrap_err();
    assert!(err.contains("enough free space"), "{err}");

    let inside = release_roots(&src.join("sub"), &h.old, &[], &[]);
    let plan_inside = build_plan(&h.pool, &inside).await.unwrap();
    let err = precheck(&plan_inside, &inside, false, &h.app_data).unwrap_err();
    assert!(err.contains("inside the recording"), "{err}");

    let in_app_data = release_roots(&h.app_data.join("rec"), &h.old, &[], &[]);
    let err = precheck(&plan, &in_app_data, false, &h.app_data).unwrap_err();
    assert!(err.contains("Nixon's own data folder"), "{err}");

    assert_eq!(precheck(&plan, &h.roots, false, &h.app_data), Ok(()));
}

// -- the startup gather ---------------------------------------------------------------------

#[tokio::test]
async fn the_gather_collects_meetings_from_an_earlier_and_the_legacy_folder() {
    let mut h = Harness::new().await;
    let base = h.base();
    let legacy = base.join("meetily-recordings");
    std::fs::create_dir_all(&legacy).unwrap();
    // Gather into the current folder `new/`; `old/` is an earlier folder.
    h.roots = release_roots(
        &h.new,
        &h.new,
        &[h.old.clone()],
        std::slice::from_ref(&legacy),
    );
    let (a, a_src) = meeting(&h.pool, &h.old, "Earlier").await;
    let (b, b_src) = meeting(&h.pool, &legacy, "Legacy").await;
    let (a_orig, b_orig) = (contents(&a_src), contents(&b_src));

    let env = h.env(ExecOptions::default(), &NoReporter);
    let (decision, plan, finished) = gather(&env, true, false).await.unwrap();

    assert_eq!(decision, GatherDecision::Run);
    assert_eq!(plan.meetings, 2);
    assert_eq!(finished.unwrap().moved, 2);
    assert_moved(&h, &a, &a_src, &h.new.join("Earlier"), &a_orig).await;
    assert_moved(&h, &b, &b_src, &h.new.join("Legacy"), &b_orig).await;
    assert!(!legacy.exists(), "the emptied legacy folder is removed");
    assert!(!h.old.exists());
}

#[tokio::test]
async fn the_gather_does_not_run_while_recording() {
    let mut h = Harness::new().await;
    h.roots = release_roots(&h.new, &h.new, &[h.old.clone()], &[]);
    let (_, src) = meeting(&h.pool, &h.old, "Earlier").await;
    let env = h.env(ExecOptions::default(), &NoReporter);
    let (decision, _, finished) = gather(&env, true, true).await.unwrap();
    assert_eq!(
        decision,
        GatherDecision::Blocked(STOP_RECORDING_FIRST.into())
    );
    assert!(finished.is_none());
    assert!(src.exists());
}

#[tokio::test]
async fn the_first_gather_asks_before_moving_anything() {
    let mut h = Harness::new().await;
    h.roots = release_roots(&h.new, &h.new, &[h.old.clone()], &[]);
    let (id, src) = meeting(&h.pool, &h.old, "Earlier").await;
    let env = h.env(ExecOptions::default(), &NoReporter);
    let (decision, plan, finished) = gather(&env, false, false).await.unwrap();
    assert_eq!(decision, GatherDecision::NeedsConfirmation);
    assert_eq!(plan.meetings, 1);
    assert!(finished.is_none());
    assert_eq!(stored_folder(&h.pool, &id).await, Some(s(&src)));
    assert!(!h.journal.exists());
}

#[tokio::test]
async fn a_root_with_a_stray_file_is_kept_after_the_move() {
    let h = Harness::new().await;
    meeting(&h.pool, &h.old, "Standup").await;
    std::fs::write(h.old.join("my notes.txt"), b"keep me").unwrap();
    let finished = h.run(ExecOptions::default()).await.unwrap();
    assert_eq!(finished.moved, 1);
    assert!(finished.removed_roots.is_empty());
    assert!(
        h.old.join("my notes.txt").exists(),
        "unrelated files are never touched"
    );
}

#[tokio::test]
async fn an_interrupted_recording_moves_with_the_rest() {
    let mut h = Harness::new().await;
    h.roots = release_roots(&h.new, &h.new, &[h.old.clone()], &[]);
    // A crashed recording whose row has no stored folder (pre-backfill).
    let id =
        MeetingsRepository::create_meeting(&h.pool, Some("Crashed".into()), None, None, None, None)
            .await
            .unwrap();
    let src = h.old.join("Crashed");
    write_folder(&src, &id, "recording");
    let original = contents(&src);

    let env = h.env(ExecOptions::default(), &NoReporter);
    let (_, plan, finished) = gather(&env, true, false).await.unwrap();

    assert_eq!(plan.meetings, 1);
    assert_eq!(finished.unwrap().moved, 1);
    assert_eq!(contents(&h.new.join("Crashed")), original);
    assert!(!src.exists());
    assert_eq!(
        stored_folder(&h.pool, &id).await,
        None,
        "a row without a path stays without"
    );
}

// -- Ruling 4: a debug build never touches the production recordings folder ------------------

#[tokio::test]
async fn a_debug_gather_never_touches_the_production_recordings_folder() {
    let mut h = Harness::new().await;
    let base = h.base();
    let production = base.join("nixon-recordings");
    std::fs::create_dir_all(&production).unwrap();
    std::fs::write(production.join(".DS_Store"), b"finder").unwrap();
    std::fs::write(production.join("owner's loose file.txt"), b"theirs").unwrap();
    // Worst case: the production folder is even listed as an earlier dev folder.
    h.roots = roots_for_profile(ProfileRootsInput {
        dev: true,
        target: h.new.clone(),
        current: h.new.clone(),
        previous: vec![h.old.clone(), production.clone()],
        platform_defaults: vec![base.join("meetily-recordings"), production.clone()],
        release_root: production.clone(),
        known: vec![production.clone()],
    });
    // A dev meeting recorded into the production folder (the 0070 case), a crashed one
    // there too, and a normal dev meeting in an earlier dev folder.
    let (prod_id, prod_src) = meeting(&h.pool, &production, "Dev meeting in prod").await;
    let crashed =
        MeetingsRepository::create_meeting(&h.pool, Some("c".into()), None, None, None, None)
            .await
            .unwrap();
    write_folder(&production.join("Crashed in prod"), &crashed, "recording");
    let (dev_id, dev_src) = meeting(&h.pool, &h.old, "Dev meeting").await;
    let before = contents(&production);
    let dev_orig = contents(&dev_src);

    let env = h.env(ExecOptions::default(), &NoReporter);
    let (decision, plan, finished) = gather(&env, true, false).await.unwrap();

    assert_eq!(decision, GatherDecision::Run);
    assert_eq!(plan.meetings, 1, "only the dev folder's meeting moves");
    assert_eq!(plan.protected, 1);
    assert_eq!(
        plan.unreferenced, 0,
        "nothing in the production folder is counted"
    );
    let finished = finished.unwrap();
    assert!(!finished.removed_roots.contains(&production));
    assert_moved(&h, &dev_id, &dev_src, &h.new.join("Dev meeting"), &dev_orig).await;

    assert_eq!(
        contents(&production),
        before,
        "the production folder is byte-for-byte unchanged"
    );
    assert_eq!(stored_folder(&h.pool, &prod_id).await, Some(s(&prod_src)));
    assert!(!h.new.join("Dev meeting in prod").exists());
    assert!(!h.new.join("Crashed in prod").exists());
}

// -- fix round 1: no move without a journal; a lost journal still reconciles row 5 -----------

#[tokio::test]
async fn a_move_refuses_to_start_when_its_journal_cannot_be_written() {
    let mut h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Unjournaled").await;
    let original = contents(&src);
    // The journal's folder is a plain file, so the journal can't be written.
    let blocker = h.base().join("not-a-folder");
    std::fs::write(&blocker, b"file").unwrap();
    h.journal = blocker.join(JOURNAL_FILE);

    let plan = build_plan(&h.pool, &h.roots).await.unwrap();
    let guard = h.state.try_begin().unwrap();
    let result = start_run(&h.env(ExecOptions::default(), &NoReporter), &plan, guard).await;

    match result {
        Err(RunError::NoJournal(why)) => assert!(why.starts_with(JOURNAL_WRITE_FAILED), "{why}"),
        other => panic!("the move must not start without its journal: {other:?}"),
    }
    assert_untouched(&h, &id, &src, &original).await;
    assert!(!h.new.join("Unjournaled").exists(), "nothing was moved");
    assert!(!h.state.is_running(), "the move slot is free again");

    // The startup gather reports it as a reason instead of moving.
    h.roots = release_roots(&h.new, &h.new, &[h.old.clone()], &[]);
    let env = h.env(ExecOptions::default(), &NoReporter);
    let (decision, _, finished) = gather(&env, true, false).await.unwrap();
    assert!(
        matches!(&decision, GatherDecision::Blocked(why) if why.starts_with(JOURNAL_WRITE_FAILED)),
        "{decision:?}"
    );
    assert!(finished.is_none());
    assert_untouched(&h, &id, &src, &original).await;
}

#[tokio::test]
async fn a_lost_journal_after_a_same_volume_rename_is_reconciled_by_the_gather() {
    let mut h = Harness::new().await;
    let (id, src) = meeting(&h.pool, &h.old, "Renamed").await;
    let original = contents(&src);
    let crashed = h
        .run(ExecOptions {
            fail_at: Some(FailPoint::AfterRename),
            ..ExecOptions::default()
        })
        .await;
    assert_eq!(crashed, Err(Crashed(FailPoint::AfterRename)));
    // Recovery row 5, and the journal is gone (damaged beyond reading, deleted, …).
    std::fs::remove_file(&h.journal).unwrap();
    assert!(!src.exists());
    assert_eq!(stored_folder(&h.pool, &id).await, Some(s(&src)));

    h.roots = release_roots(&h.new, &h.new, &[h.old.clone()], &[]);
    let env = h.env(ExecOptions::default(), &NoReporter);
    let (decision, plan, finished) = gather(&env, true, false).await.unwrap();

    assert_eq!(decision, GatherDecision::Run);
    assert_eq!(plan.missing, 0, "not reported missing");
    assert_eq!(finished.unwrap().moved, 1);
    assert_moved(&h, &id, &src, &h.new.join("Renamed"), &original).await;
}

// Row 5 when the stored path runs through a symlink: after the same-volume rename the
// source no longer canonicalizes, so the row must still be recognised as naming it.
#[tokio::test]
async fn crash_after_a_same_volume_rename_updates_a_row_stored_through_a_symlink() {
    let mut h = Harness::new().await;
    let real = h.base().join("real");
    std::fs::create_dir_all(real.join("old")).unwrap();
    let link = h.base().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    h.old = link.join("old");
    h.roots = release_roots(&h.new, &h.old, &[], &[]);
    let (id, src) = meeting(&h.pool, &h.old, "Linked").await;
    let original = contents(&src);
    let dst = h.new.join("Linked");

    let crashed = h
        .run(ExecOptions {
            fail_at: Some(FailPoint::AfterRename),
            ..ExecOptions::default()
        })
        .await;
    assert_eq!(crashed, Err(Crashed(FailPoint::AfterRename)));
    assert!(
        !src.exists() && dst.exists(),
        "renamed, row not yet updated"
    );

    let finished = h.resume().await.expect("the journal is resumed");
    assert_eq!(finished.moved, 1, "the resume finishes the move");
    assert_moved(&h, &id, &src, &dst, &original).await;
    assert!(!h.journal.exists(), "the journal is gone once reconciled");
}
