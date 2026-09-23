//! Runs a planned move folder by folder under the folder lease, reports progress, resumes a
//! journaled move after a crash, and decides whether the startup gather may run.
//!
//! Everything here takes its database, roots, journal path, state and reporter as
//! arguments, so the integration tests drive it with temp folders; `commands.rs` wires it
//! to the running app.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use sqlx::SqlitePool;

use super::disk::{remove_emptied_roots, sweep_staging, RealFs};
use super::exec::{move_unit, Crashed, ExecOptions, UnitOutcome};
use super::journal::{self, Journal};
use super::plan::{plan_move, InterruptedFolder, MovePlan, MoveUnit, PlanRow};
use super::roots::MoveRoots;
use crate::audio::folder_lease::{self, LeaseHolder};

/// Refusal while recording. Also the Settings tooltip copy.
pub const STOP_RECORDING_FIRST: &str = "Stop the recording to change where recordings are saved.";
/// Refusal while another move runs.
pub const MOVE_ALREADY_RUNNING: &str =
    "Recordings are already being moved. Wait for that to finish, or stop it first.";

/// Live status of the running move, for `recordings-move-progress` and
/// `api_recordings_move_status`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveStatus {
    /// Meetings finished (moved, skipped or failed).
    pub done: usize,
    /// Meetings in this run.
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current_title: Option<String>,
    /// Set while the mover waits for another job to release a meeting's folder.
    pub waiting_for: Option<LeaseHolder>,
    pub target: PathBuf,
}

/// One meeting that couldn't be moved.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveFailure {
    pub meeting_id: String,
    pub title: String,
    pub reason: String,
    /// Where the recording still is (for "Show").
    pub folder_path: PathBuf,
}

/// Payload of `recordings-move-finished`. Counts are meetings.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveFinished {
    pub moved: usize,
    pub skipped: usize,
    pub failed: Vec<MoveFailure>,
    pub cancelled: bool,
    /// Old recordings folders the run emptied and removed.
    pub removed_roots: Vec<PathBuf>,
}

/// Receives progress and the final result (events in the app; a recorder in tests).
pub trait MoveReporter: Send + Sync {
    fn progress(&self, status: &MoveStatus);
    fn finished(&self, finished: &MoveFinished);
}

/// A reporter that reports nothing.
pub struct NoReporter;

impl MoveReporter for NoReporter {
    fn progress(&self, _: &MoveStatus) {}
    fn finished(&self, _: &MoveFinished) {}
}

/// Process-wide move state: one move at a time, a between-folders cancel flag, the live
/// status, and why the last startup gather couldn't run.
#[derive(Default)]
pub struct MoveState {
    running: AtomicBool,
    cancel: AtomicBool,
    status: Mutex<Option<MoveStatus>>,
    gather_blocked: Mutex<Option<String>>,
}

/// The app's move state.
pub static MOVE_STATE: MoveState = MoveState::new();

impl MoveState {
    pub const fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            status: Mutex::new(None),
            gather_blocked: Mutex::new(None),
        }
    }

    /// Claim the one move slot. `None` while another move runs.
    pub fn try_begin(&'static self) -> Option<RunGuard> {
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        self.cancel.store(false, Ordering::SeqCst);
        Some(RunGuard { state: self })
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Ask the running move to stop after the current folder.
    pub fn cancel(&self) {
        if self.is_running() {
            self.cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn status(&self) -> Option<MoveStatus> {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn gather_blocked(&self) -> Option<String> {
        self.gather_blocked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_gather_blocked(&self, reason: Option<String>) {
        *self
            .gather_blocked
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = reason;
    }

    fn update(&self, f: impl FnOnce(&mut MoveStatus)) -> MoveStatus {
        let mut guard = self.status.lock().unwrap_or_else(|p| p.into_inner());
        let status = guard.get_or_insert_with(MoveStatus::default);
        f(status);
        status.clone()
    }
}

/// The claimed move slot. Released (and the status cleared) on drop.
pub struct RunGuard {
    state: &'static MoveState,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        *self.state.status.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.state.cancel.store(false, Ordering::SeqCst);
        self.state.running.store(false, Ordering::SeqCst);
    }
}

/// Everything one move needs.
pub struct MoveEnv<'a> {
    pub pool: &'a SqlitePool,
    pub roots: &'a MoveRoots,
    pub journal: &'a Path,
    pub app_data: &'a Path,
    pub opts: ExecOptions,
    pub state: &'static MoveState,
    pub reporter: &'a dyn MoveReporter,
}

/// Plan moving this profile's meetings into `roots.target`, reading the rows and scanning
/// this profile's recordings folders for interrupted recordings.
pub async fn build_plan(pool: &SqlitePool, roots: &MoveRoots) -> anyhow::Result<MovePlan> {
    let rows: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, title, folder_path FROM meetings")
            .fetch_all(pool)
            .await?;
    let rows: Vec<PlanRow> = rows
        .into_iter()
        .map(|(meeting_id, title, folder_path)| PlanRow {
            meeting_id,
            title,
            folder_path,
        })
        .collect();
    let roots = roots.clone();
    let plan = tokio::task::spawn_blocking(move || {
        let ids: BTreeSet<&str> = rows.iter().map(|r| r.meeting_id.as_str()).collect();
        // Interrupted recordings of THIS database only, in this profile's own folders.
        let interrupted: Vec<InterruptedFolder> =
            crate::audio::recording_recovery::scan_interrupted_in_roots(&roots.sources)
                .into_iter()
                .filter(|r| ids.contains(r.meeting_id.as_str()))
                .map(|r| InterruptedFolder {
                    meeting_id: r.meeting_id,
                    folder: PathBuf::from(r.folder_path),
                })
                .collect();
        plan_move(&rows, &interrupted, &roots, &RealFs)
    })
    .await?;
    Ok(plan)
}

fn human_bytes(bytes: u64) -> String {
    let gb = bytes as f64 / 1_073_741_824.0;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", (bytes as f64 / 1_048_576.0).ceil())
    }
}

/// Every check a move must pass before anything changes. `Err` is user-facing copy.
pub fn precheck(
    plan: &MovePlan,
    roots: &MoveRoots,
    recording: bool,
    app_data: &Path,
) -> Result<(), String> {
    let target = &roots.target;
    if recording {
        return Err(STOP_RECORDING_FIRST.into());
    }
    if roots.is_protected(target) {
        return Err(
            "This test build can't save recordings in the Nixon app's own recordings \
                    folder. Choose another folder."
                .into(),
        );
    }
    crate::audio::volume_check::ensure_recordings_volume_allowed(target)
        .map_err(|e| e.to_string())?;
    let app_data = crate::audio::meeting_folder::canonical_or_lexical(app_data);
    if target.starts_with(&app_data) {
        return Err(
            "Recordings can't be saved inside Nixon's own data folder. Choose another \
                    folder."
                .into(),
        );
    }
    if let Some(unit) = plan.units.iter().find(|u| target.starts_with(&u.src)) {
        return Err(format!(
            "The new folder is inside the recording of \"{}\". Choose a folder outside it.",
            unit.title
        ));
    }
    std::fs::create_dir_all(target)
        .map_err(|e| format!("Couldn't create {}: {e}", target.display()))?;
    let probe = target.join(".nixon-write-test");
    std::fs::write(&probe, b"ok").map_err(|_| {
        format!(
            "Nixon can't write to {}. Choose another folder.",
            target.display()
        )
    })?;
    let _ = std::fs::remove_file(&probe);
    if !plan.enough_space {
        return Err(format!(
            "There isn't enough free space in the new folder: moving needs {} plus 1 GB to \
             spare, and {} is free.",
            human_bytes(plan.cross_volume_bytes),
            human_bytes(plan.free_bytes)
        ));
    }
    Ok(())
}

/// Refusal when the journal can't be written (before anything moved).
pub const JOURNAL_WRITE_FAILED: &str =
    "Nixon couldn't save its record of the move, so nothing was moved.";

/// Why [`start_run`] didn't finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The journal couldn't be written, so the run never started (user-facing reason).
    NoJournal(String),
    /// An injected fail point stopped the run (tests only).
    Crashed(Crashed),
}

/// Write the journal for `plan`. A move must not start without one: after a crash between
/// a same-volume rename and the row update, only the journal says where the folder went
/// (the source no longer exists, so a plan from the rows alone would report it missing).
/// `Err` is user-facing copy.
pub fn write_journal(path: &Path, plan: &MovePlan) -> Result<(), String> {
    let journal = Journal::new(plan.target.clone(), plan.units.clone());
    journal::write(path, &journal).map_err(|e| {
        log::error!("recordings move: couldn't write the journal, not moving: {e:#}");
        format!("{JOURNAL_WRITE_FAILED} ({e:#})")
    })
}

/// Record the run in the journal, then move every folder (see [`run_units`]). Refuses to
/// start when the journal can't be written.
pub async fn start_run(
    env: &MoveEnv<'_>,
    plan: &MovePlan,
    guard: RunGuard,
) -> Result<MoveFinished, RunError> {
    write_journal(env.journal, plan).map_err(RunError::NoJournal)?;
    run_units(env, plan.units.clone(), guard)
        .await
        .map_err(RunError::Crashed)
}

/// Finish a move a crash interrupted: reconcile every folder the journal names, then
/// remove the journal. `Ok(None)` when there was nothing to resume.
pub async fn resume_from_journal(
    env: &MoveEnv<'_>,
    guard: RunGuard,
) -> Result<Option<MoveFinished>, Crashed> {
    let Some(journal) = journal::read(env.journal) else {
        journal::delete(env.journal);
        return Ok(None);
    };
    log::info!(
        "recordings move: resuming an interrupted move of {} folder(s)",
        journal.units.len()
    );
    for root in [&journal.target, &env.roots.target] {
        if !root.as_os_str().is_empty() && !env.roots.is_protected(root) {
            sweep_staging(root);
        }
    }
    let units: Vec<MoveUnit> = journal
        .units
        .into_iter()
        .filter(|u| !env.roots.is_protected(&u.src))
        .collect();
    run_units(env, units, guard).await.map(Some)
}

/// Take every lease of `unit`, in a fixed order. `Err` = skip the folder (it is being
/// recorded into right now; the next gather picks it up).
async fn lease_unit(
    unit: &MoveUnit,
    env: &MoveEnv<'_>,
) -> Result<Vec<folder_lease::FolderLease>, String> {
    // A continued meeting's folder belongs to several meetings, so the mover may hold more
    // than one lease. No other job ever holds two, and the mover takes them in sorted
    // order, so this cannot deadlock.
    let mut ids: Vec<&String> = unit.lease_ids.iter().collect();
    ids.sort();
    ids.dedup();
    let mut leases = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(lease) = folder_lease::try_acquire(id, LeaseHolder::Mover) {
            leases.push(lease);
            continue;
        }
        let holder = folder_lease::current_holder(id);
        if holder == Some(LeaseHolder::Recording) {
            return Err("it's being recorded right now".into());
        }
        let status = env.state.update(|s| s.waiting_for = holder);
        env.reporter.progress(&status);
        leases.push(folder_lease::acquire(id, LeaseHolder::Mover).await);
        let status = env.state.update(|s| s.waiting_for = None);
        env.reporter.progress(&status);
    }
    Ok(leases)
}

/// Move `units` one folder at a time, each under its leases. One folder failing doesn't
/// stop the run; a cancel takes effect between folders. At the end the journal is removed
/// and every old root the run emptied is removed. An injected fail point returns
/// `Err(Crashed)` immediately and leaves everything as a killed app would.
pub async fn run_units(
    env: &MoveEnv<'_>,
    units: Vec<MoveUnit>,
    guard: RunGuard,
) -> Result<MoveFinished, Crashed> {
    let total: usize = units.iter().map(|u| u.lease_ids.len()).sum();
    let bytes_total: u64 = units.iter().map(|u| u.bytes).sum();
    let status = env.state.update(|s| {
        *s = MoveStatus {
            total,
            bytes_total,
            target: env.roots.target.clone(),
            ..MoveStatus::default()
        }
    });
    env.reporter.progress(&status);

    let mut finished = MoveFinished::default();
    let mut moved_from: Vec<PathBuf> = Vec::new();
    for unit in &units {
        let n = unit.lease_ids.len();
        if env.state.cancel.load(Ordering::SeqCst) {
            finished.cancelled = true;
            finished.skipped += n;
            continue;
        }
        let status = env
            .state
            .update(|s| s.current_title = Some(unit.title.clone()));
        env.reporter.progress(&status);
        let bytes_before = status.bytes_done;

        let outcome = match lease_unit(unit, env).await {
            Err(why) => UnitOutcome::Skipped(why),
            Ok(_leases) => {
                let on_bytes = |b: u64| {
                    let s = env.state.update(|s| s.bytes_done += b);
                    env.reporter.progress(&s);
                };
                move_unit(env.pool, unit, env.roots, &env.opts, &on_bytes).await?
            }
        };
        match outcome {
            // AlreadyDone is a move this run (or the crashed run it resumes) finished, so
            // its old root counts as emptied by the run too.
            UnitOutcome::Moved | UnitOutcome::AlreadyDone => {
                finished.moved += n;
                if let Some(parent) = unit.src.parent() {
                    moved_from.push(parent.to_path_buf());
                }
            }
            UnitOutcome::Skipped(why) => {
                log::info!("recordings move: skipped {}: {why}", unit.title);
                finished.skipped += n;
            }
            UnitOutcome::Missing => finished
                .failed
                .push(failure(unit, "its folder couldn't be found")),
            UnitOutcome::Failed(why) => {
                log::warn!("recordings move: {} wasn't moved: {why}", unit.title);
                finished.failed.push(failure(unit, &why));
            }
        }
        let status = env.state.update(|s| {
            s.done += n;
            s.bytes_done = bytes_before + unit.bytes;
        });
        env.reporter.progress(&status);
    }

    journal::delete(env.journal);
    finished.removed_roots = remove_emptied_roots(&moved_from, env.roots);
    log::info!(
        "recordings move: moved {}, skipped {}, failed {}{}",
        finished.moved,
        finished.skipped,
        finished.failed.len(),
        if finished.cancelled { " (stopped)" } else { "" }
    );
    env.reporter.finished(&finished);
    drop(guard);
    Ok(finished)
}

fn failure(unit: &MoveUnit, reason: &str) -> MoveFailure {
    MoveFailure {
        meeting_id: unit.lease_ids.first().cloned().unwrap_or_default(),
        title: unit.title.clone(),
        reason: reason.to_string(),
        folder_path: unit.src.clone(),
    }
}

/// What the startup gather does with a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatherDecision {
    /// Every meeting is already in the current folder.
    Nothing,
    /// First launch of this version with meetings elsewhere: ask before moving.
    NeedsConfirmation,
    /// A precheck failed (user-facing reason).
    Blocked(String),
    /// Move them.
    Run,
}

/// Decide the startup gather. `precheck` is only consulted when the gather would run.
pub fn decide_gather(
    plan: &MovePlan,
    gathered_once: bool,
    precheck: impl FnOnce() -> Result<(), String>,
) -> GatherDecision {
    if plan.is_empty() {
        GatherDecision::Nothing
    } else if !gathered_once {
        GatherDecision::NeedsConfirmation
    } else {
        match precheck() {
            Ok(()) => GatherDecision::Run,
            Err(why) => GatherDecision::Blocked(why),
        }
    }
}

/// The gather: plan a move of every meeting into `env.roots.target` and, when the plan
/// isn't empty, the owner already agreed once, and every precheck passes, run it.
pub async fn gather(
    env: &MoveEnv<'_>,
    gathered_once: bool,
    recording: bool,
) -> anyhow::Result<(GatherDecision, MovePlan, Option<MoveFinished>)> {
    let plan = build_plan(env.pool, env.roots).await?;
    let decision = decide_gather(&plan, gathered_once, || {
        precheck(&plan, env.roots, recording, env.app_data)
    });
    if decision != GatherDecision::Run {
        return Ok((decision, plan, None));
    }
    let Some(guard) = env.state.try_begin() else {
        return Ok((
            GatherDecision::Blocked(MOVE_ALREADY_RUNNING.into()),
            plan,
            None,
        ));
    };
    env.state.set_gather_blocked(None);
    match start_run(env, &plan, guard).await {
        Ok(finished) => Ok((decision, plan, Some(finished))),
        Err(RunError::NoJournal(why)) => Ok((GatherDecision::Blocked(why), plan, None)),
        Err(RunError::Crashed(c)) => Err(anyhow::anyhow!("the gather stopped at {:?}", c.0)),
    }
}
