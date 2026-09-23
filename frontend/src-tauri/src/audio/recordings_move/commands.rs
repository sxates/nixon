//! Tauri commands, events and the startup hook for the recordings mover.
//!
//! Commands (registered in `registry.rs`):
//! - `api_plan_recordings_move { target? }` → [`MovePlan`] (read-only; no target = gather)
//! - `api_change_recordings_folder { target }` → prechecks, persist the folder, move
//! - `api_gather_recordings` → move every meeting into the current folder
//! - `api_cancel_recordings_move` → stop after the current folder
//! - `api_recordings_move_status` → `Option<MoveStatus>` (re-attach after a remount)
//! - `api_recordings_gather_state` → [`GatherState`] (first-run question / leftovers)
//!
//! Events: `recordings-move-progress` ([`MoveStatus`]), `recordings-move-finished`
//! ([`MoveFinished`]), `recordings-gather-needed` (`{ plan }`), `recordings-gather-blocked`
//! (`{ plan, reason }`).

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::exec::ExecOptions;
use super::journal;
use super::plan::MovePlan;
use super::roots::{profile_roots, MoveRoots};
use super::runner::{
    build_plan, gather, precheck, resume_from_journal, run_units, write_journal, GatherDecision,
    MoveEnv, MoveFinished, MoveReporter, MoveStatus, RunGuard, MOVE_ALREADY_RUNNING, MOVE_STATE,
};
use super::target_dir::{startup_root, CreatedTarget, StartupRoot};
use crate::audio::meeting_folder::canonical_or_lexical;
use crate::audio::recording_preferences::{
    load_recording_preferences, recordings_root, save_recording_preferences,
};
use crate::state::AppState;

pub const EVENT_PROGRESS: &str = "recordings-move-progress";
pub const EVENT_FINISHED: &str = "recordings-move-finished";
pub const EVENT_GATHER_NEEDED: &str = "recordings-gather-needed";
pub const EVENT_GATHER_BLOCKED: &str = "recordings-gather-blocked";

/// Emits the mover's events to the webview.
struct AppReporter<R: Runtime>(AppHandle<R>);

impl<R: Runtime> MoveReporter for AppReporter<R> {
    fn progress(&self, status: &MoveStatus) {
        let _ = self.0.emit(EVENT_PROGRESS, status);
    }
    fn finished(&self, finished: &MoveFinished) {
        let _ = self.0.emit(EVENT_FINISHED, finished);
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatherNeeded {
    plan: MovePlan,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatherBlocked {
    plan: MovePlan,
    reason: String,
}

/// What Settings needs to show about meetings outside the current folder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatherState {
    /// The one-time question hasn't been answered and there is something to gather.
    pub needs_confirmation: bool,
    /// Why the last startup gather couldn't run, if it couldn't.
    pub blocked_reason: Option<String>,
    /// What a gather into the current folder would move right now.
    pub plan: MovePlan,
}

fn pool<R: Runtime>(app: &AppHandle<R>) -> Result<sqlx::SqlitePool, String> {
    app.try_state::<AppState>()
        .map(|s| s.db_manager.pool().clone())
        .ok_or_else(|| "Nixon's database isn't ready yet. Try again in a moment.".to_string())
}

fn target_path(target: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(target.trim());
    if !path.is_absolute() {
        return Err("Choose a folder for your recordings.".into());
    }
    Ok(path)
}

/// Roots for moving into `target`, created first so every path canonicalizes the same way.
/// A refusal after this calls [`CreatedTarget::undo`], which removes every folder made here.
fn roots_into(target: &Path) -> Result<(MoveRoots, CreatedTarget), String> {
    let created = CreatedTarget::create(target)?;
    Ok((profile_roots(target), created))
}

/// Remove the folders the run emptied from `previous_save_folders` (a kept root stays).
async fn forget_removed_roots<R: Runtime>(app: &AppHandle<R>, removed: &[PathBuf]) {
    if removed.is_empty() {
        return;
    }
    let Ok(mut prefs) = load_recording_preferences(app).await else {
        return;
    };
    // A removed folder no longer canonicalizes, but its parent still does.
    let canon = |p: &Path| match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) => canonical_or_lexical(parent).join(name),
        _ => p.to_path_buf(),
    };
    let before = prefs.previous_save_folders.len();
    prefs
        .previous_save_folders
        .retain(|p| !removed.contains(p) && !removed.contains(&canon(p)));
    if prefs.previous_save_folders.len() != before {
        if let Err(e) = save_recording_preferences(app, &prefs).await {
            log::warn!("recordings move: couldn't update the earlier folders list: {e}");
        }
    }
}

/// Run `plan` (its journal already written) in the background with the claimed `guard`,
/// then tidy the preferences.
fn spawn_run<R: Runtime>(app: AppHandle<R>, roots: MoveRoots, plan: MovePlan, guard: RunGuard) {
    tauri::async_runtime::spawn(async move {
        let Ok(pool) = pool(&app) else { return };
        let journal_path = journal::default_path();
        let app_data = crate::app_paths::app_data_dir();
        let reporter = AppReporter(app.clone());
        let env = MoveEnv {
            pool: &pool,
            roots: &roots,
            journal: &journal_path,
            app_data: &app_data,
            opts: ExecOptions::default(),
            state: &MOVE_STATE,
            reporter: &reporter,
        };
        if let Ok(finished) = run_units(&env, plan.units, guard).await {
            forget_removed_roots(&app, &finished.removed_roots).await;
        }
    });
}

#[tauri::command]
pub async fn api_plan_recordings_move<R: Runtime>(
    app: AppHandle<R>,
    target: Option<String>,
) -> Result<MovePlan, String> {
    let pool = pool(&app)?;
    let target = match target {
        Some(t) => target_path(&t)?,
        None => recordings_root(),
    };
    // Planning is read-only: don't create a folder the owner may still cancel.
    let roots = profile_roots(&target);
    build_plan(&pool, &roots)
        .await
        .map_err(|e| format!("Couldn't look through your recordings: {e:#}"))
}

#[tauri::command]
pub async fn api_change_recordings_folder<R: Runtime>(
    app: AppHandle<R>,
    target: String,
) -> Result<(), String> {
    let pool = pool(&app)?;
    let target = target_path(&target)?;
    let guard = MOVE_STATE
        .try_begin()
        .ok_or_else(|| MOVE_ALREADY_RUNNING.to_string())?;
    let recording = crate::audio::recording_commands::is_recording().await;
    if recording {
        return Err(super::runner::STOP_RECORDING_FIRST.into());
    }
    // Refuse a removable or network drive before creating anything on it.
    crate::audio::volume_check::ensure_recordings_volume_allowed(&target)
        .map_err(|e| e.to_string())?;
    let (roots, created) = roots_into(&target)?;
    let plan = match build_plan(&pool, &roots).await {
        Ok(plan) => plan,
        Err(e) => {
            created.undo();
            return Err(format!("Couldn't look through your recordings: {e:#}"));
        }
    };
    if let Err(why) = precheck(&plan, &roots, recording, &crate::app_paths::app_data_dir()) {
        created.undo();
        return Err(why);
    }
    // No move starts without its journal (see `write_journal`); written before the folder
    // preference so a refusal here changes nothing.
    let journal_path = journal::default_path();
    if !plan.is_empty() {
        if let Err(why) = write_journal(&journal_path, &plan) {
            created.undo();
            return Err(why);
        }
    }

    // Persist the new folder FIRST: new recordings land there from now on, so a recording
    // started mid-move never joins the move.
    let stored = load_recording_preferences(&app)
        .await
        .map_err(|e| format!("Failed to load recording preferences: {e}"))?;
    let mut prefs = stored.clone();
    if stored.save_folder != target && !prefs.previous_save_folders.contains(&stored.save_folder) {
        prefs.previous_save_folders.push(stored.save_folder.clone());
    }
    prefs.previous_save_folders.retain(|p| *p != target);
    prefs.save_folder = target;
    prefs.save_folder_user_chosen = true;
    prefs.recordings_gathered_once = true;
    if let Err(e) = save_recording_preferences(&app, &prefs).await {
        journal::delete(&journal_path);
        return Err(format!("Failed to save recording preferences: {e}"));
    }
    MOVE_STATE.set_gather_blocked(None);

    if !plan.is_empty() {
        spawn_run(app, roots, plan, guard);
    }
    Ok(())
}

#[tauri::command]
pub async fn api_gather_recordings<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let pool = pool(&app)?;
    let guard = MOVE_STATE
        .try_begin()
        .ok_or_else(|| MOVE_ALREADY_RUNNING.to_string())?;
    let target = recordings_root();
    let default_root = crate::audio::recording_preferences::get_default_recordings_folder();
    if let StartupRoot::Unavailable(reason) = startup_root(&target, &default_root) {
        return Err(reason);
    }
    let (roots, created) = roots_into(&target)?;
    let plan = match build_plan(&pool, &roots).await {
        Ok(plan) => plan,
        Err(e) => {
            created.undo();
            return Err(format!("Couldn't look through your recordings: {e:#}"));
        }
    };
    let recording = crate::audio::recording_commands::is_recording().await;
    if let Err(why) = precheck(&plan, &roots, recording, &crate::app_paths::app_data_dir()) {
        created.undo();
        return Err(why);
    }
    let journal_path = journal::default_path();
    if !plan.is_empty() {
        write_journal(&journal_path, &plan)?;
    }
    if let Err(why) = mark_gathered(&app).await {
        journal::delete(&journal_path);
        return Err(why);
    }
    MOVE_STATE.set_gather_blocked(None);
    if !plan.is_empty() {
        spawn_run(app, roots, plan, guard);
    }
    Ok(())
}

/// Record that the owner agreed to (or never needed) the one-time gather.
async fn mark_gathered<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let mut prefs = load_recording_preferences(app)
        .await
        .map_err(|e| format!("Failed to load recording preferences: {e}"))?;
    if prefs.recordings_gathered_once {
        return Ok(());
    }
    prefs.recordings_gathered_once = true;
    save_recording_preferences(app, &prefs)
        .await
        .map_err(|e| format!("Failed to save recording preferences: {e}"))
}

#[tauri::command]
pub async fn api_cancel_recordings_move() -> Result<(), String> {
    MOVE_STATE.cancel();
    Ok(())
}

#[tauri::command]
pub async fn api_recordings_move_status() -> Result<Option<MoveStatus>, String> {
    Ok(MOVE_STATE.status())
}

#[tauri::command]
pub async fn api_recordings_gather_state<R: Runtime>(
    app: AppHandle<R>,
) -> Result<GatherState, String> {
    let pool = pool(&app)?;
    let prefs = load_recording_preferences(&app)
        .await
        .map_err(|e| format!("Failed to load recording preferences: {e}"))?;
    let plan = build_plan(&pool, &profile_roots(&recordings_root()))
        .await
        .map_err(|e| format!("Couldn't look through your recordings: {e:#}"))?;
    Ok(GatherState {
        needs_confirmation: !prefs.recordings_gathered_once && !plan.is_empty(),
        blocked_reason: MOVE_STATE.gather_blocked(),
        plan,
    })
}

/// Startup (specs/0073 task 15): finish a move a crash or quit interrupted, then gather any
/// meeting outside the current folder. The first launch of this version asks first
/// (`recordings-gather-needed`); after that the gather is silent. Best-effort: failures log.
pub fn spawn_startup_resume_and_gather<R: Runtime>(
    app: AppHandle<R>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let Ok(pool) = pool(&app) else {
            log::debug!("recordings move: database not initialized yet; nothing to gather");
            return;
        };
        let journal_path = journal::default_path();
        let app_data = crate::app_paths::app_data_dir();
        let reporter = AppReporter(app.clone());
        let target = recordings_root();
        let default_root = crate::audio::recording_preferences::get_default_recordings_folder();
        match startup_root(&target, &default_root) {
            StartupRoot::Ready => {}
            StartupRoot::CreateDefault => {
                if let Err(e) = std::fs::create_dir_all(&target) {
                    log::warn!("recordings move: couldn't create the default folder: {e}");
                    return;
                }
            }
            StartupRoot::Unavailable(reason) => {
                // Never create it: on an unplugged drive that would be a stray local folder
                // at the mount path. Tell Settings why nothing was gathered.
                log::warn!("recordings move: current folder missing, no gather or resume");
                MOVE_STATE.set_gather_blocked(Some(reason.clone()));
                let plan = MovePlan::default();
                let _ = app.emit(EVENT_GATHER_BLOCKED, GatherBlocked { plan, reason });
                return;
            }
        }
        let roots = profile_roots(&target);
        let env = MoveEnv {
            pool: &pool,
            roots: &roots,
            journal: &journal_path,
            app_data: &app_data,
            opts: ExecOptions::default(),
            state: &MOVE_STATE,
            reporter: &reporter,
        };

        if journal_path.exists() {
            if let Some(guard) = MOVE_STATE.try_begin() {
                if let Ok(Some(finished)) = resume_from_journal(&env, guard).await {
                    forget_removed_roots(&app, &finished.removed_roots).await;
                }
            }
        }

        let prefs = match load_recording_preferences(&app).await {
            Ok(p) => p,
            Err(e) => {
                log::warn!("recordings move: couldn't load preferences, no gather: {e}");
                return;
            }
        };
        let recording = crate::audio::recording_commands::is_recording().await;
        match gather(&env, prefs.recordings_gathered_once, recording).await {
            Ok((GatherDecision::Nothing, _, _)) => {
                if !prefs.recordings_gathered_once {
                    let _ = mark_gathered(&app).await; // nothing to ask about
                }
            }
            Ok((GatherDecision::NeedsConfirmation, plan, _)) => {
                log::info!(
                    "recordings move: {} meeting(s) are outside the recordings folder; asking first",
                    plan.meetings
                );
                let _ = app.emit(EVENT_GATHER_NEEDED, GatherNeeded { plan });
            }
            Ok((GatherDecision::Blocked(reason), plan, _)) => {
                log::info!("recordings move: gather can't run: {reason}");
                MOVE_STATE.set_gather_blocked(Some(reason.clone()));
                let _ = app.emit(EVENT_GATHER_BLOCKED, GatherBlocked { plan, reason });
            }
            Ok((GatherDecision::Run, _, Some(finished))) => {
                forget_removed_roots(&app, &finished.removed_roots).await;
            }
            Ok((GatherDecision::Run, _, None)) => {}
            Err(e) => log::warn!("recordings move: gather failed: {e:#}"),
        }
    })
}
