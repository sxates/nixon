//! Debug-only fixture seeding and onboarding harness (specs/0059).
//! Compiled out of release builds; every entry point re-checks the `.debug` identifier.
#![cfg(debug_assertions)]

pub mod audio;
pub mod commands;
pub mod dataset;
pub mod fake_downloads;
pub mod folder;
pub mod guard;
pub mod seed;
pub mod wav;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// Wipe + seed the debug profile. Creates the DB (and manages `AppState`) when this is the
/// first launch, because `database::setup` skips that on a missing DB file.
pub async fn run_seed(app: &AppHandle, no_audio: bool) -> Result<seed::SeedReport> {
    anyhow::ensure!(
        guard::is_debug_identifier(),
        "refusing to seed a non-.debug identifier"
    );
    let ds = dataset::load_embedded().map_err(anyhow::Error::msg)?;
    dataset::validate(&ds).map_err(|e| anyhow::anyhow!("fixture validation: {}", e.join("; ")))?;

    if app.try_state::<crate::state::AppState>().is_none() {
        let db_manager = crate::database::manager::DatabaseManager::new_from_app_handle(app)
            .await
            .context("create debug DB")?;
        app.manage(crate::state::AppState { db_manager });
    }
    let state = app.state::<crate::state::AppState>();
    let pool = state.db_manager.pool();

    let root = crate::audio::recordings_root();
    std::fs::create_dir_all(&root)?;
    let removed = folder::remove_demo_folders(&root)?;
    log::info!(
        "[dev] removed {removed} previous demo folders under {}",
        root.display()
    );

    let now = chrono::Utc::now();
    let mut folders = seed::FolderMap::new();
    for m in &ds.meetings {
        let start = seed::started_at(m, now);
        if let Some(f) = prepare_folder(m, &root, start, no_audio) {
            folders.insert(m.id.clone(), f);
        }
    }
    let report = seed::seed_all(pool, &ds, &folders, now).await?;

    // A seeded profile is post-onboarding unless --onboarding also asked for a reset.
    if !guard::DevFlags::from_env().reset_onboarding {
        let status = seeded_onboarding_status(now);
        crate::onboarding::save_onboarding_status(app, &status)
            .await
            .context("mark onboarding complete")?;
    }
    log::info!(
        "[dev] seeded {} meetings, {} people, {} segments, {} failed",
        report.meetings,
        report.people,
        report.segments,
        report.failed
    );
    Ok(report)
}

/// Called from `lib.rs` setup after database init. Blocking on purpose: the window opens
/// on a fully seeded profile.
pub fn seed_at_startup(app: &AppHandle) {
    let flags = guard::DevFlags::from_env();
    if !flags.fixtures {
        return;
    }
    if !guard::allowed("NIXON_FIXTURES") {
        return;
    }
    let t0 = std::time::Instant::now();
    match tauri::async_runtime::block_on(run_seed(app, flags.no_audio)) {
        Ok(_) => log::info!("[dev] fixtures ready in {:.1}s", t0.elapsed().as_secs_f32()),
        Err(e) => log::error!("[dev] fixture seeding failed: {e:#}"),
    }
}

/// Writes one meeting's recording folder (and, unless `no_audio`, its synthesized audio).
/// A folder-write failure is logged and reported as `None` rather than aborting the whole
/// run: `seed_all` seeds the meeting with `folder_path = NULL`, which the app already
/// treats as "no audio available" — matching the `--no-audio` spirit for that one meeting
/// while every other meeting still lands (specs/0059 fix round 1).
fn prepare_folder(
    m: &dataset::FixtureMeeting,
    root: &Path,
    start: DateTime<Utc>,
    no_audio: bool,
) -> Option<PathBuf> {
    let f = match folder::write_folder(m, root, start) {
        Ok(f) => f,
        Err(e) => {
            log::error!(
                "[dev] folder for {} failed: {e:#}; skipping its recording folder",
                m.id
            );
            return None;
        }
    };
    if !no_audio {
        let t0 = std::time::Instant::now();
        match audio::render_meeting_audio(m, &f) {
            Ok(true) => log::info!(
                "[dev] audio for {} in {:.1}s",
                m.id,
                t0.elapsed().as_secs_f32()
            ),
            Ok(false) => log::warn!("[dev] `say` unavailable; {} has no audio", m.id),
            Err(e) => log::warn!("[dev] audio for {} failed: {e:#}", m.id),
        }
    }
    Some(f)
}

/// The onboarding status a freshly-seeded debug profile is marked with: post-onboarding,
/// every model "downloaded", timestamped `now`.
pub(crate) fn seeded_onboarding_status(now: DateTime<Utc>) -> crate::onboarding::OnboardingStatus {
    crate::onboarding::OnboardingStatus {
        version: "1.0".into(),
        completed: true,
        current_step: 4,
        model_status: crate::onboarding::ModelStatus {
            parakeet: "downloaded".into(),
            summary: "downloaded".into(),
            selected_summary_model: None,
        },
        last_updated: now.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_onboarding_status_marks_a_completed_profile() {
        let now = Utc::now();
        let status = seeded_onboarding_status(now);
        assert!(status.completed);
        assert_eq!(status.current_step, 4);
        assert_eq!(status.model_status.parakeet, "downloaded");
        assert_eq!(status.model_status.summary, "downloaded");
        assert!(status.model_status.selected_summary_model.is_none());
        assert_eq!(status.last_updated, now.to_rfc3339());
    }

    #[test]
    fn prepare_folder_returns_none_when_the_root_cannot_be_created_under() {
        // A regular file can't be a directory's parent: `create_dir_all` under it fails,
        // so `write_folder` errors and `prepare_folder` must report `None` rather than
        // propagating (the caller keeps seeding the remaining meetings).
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        let bogus_root = file.path().join("nested").join("root");
        let m = dataset::load_embedded().expect("embedded dataset").meetings[0].clone();
        let start = Utc::now();
        assert!(prepare_folder(&m, &bogus_root, start, true).is_none());
    }

    #[test]
    fn prepare_folder_returns_the_folder_when_the_root_is_writable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = dataset::load_embedded().expect("embedded dataset").meetings[0].clone();
        let start = Utc::now();
        let folder = prepare_folder(&m, dir.path(), start, true).expect("folder is created");
        assert!(folder.exists());
    }
}
