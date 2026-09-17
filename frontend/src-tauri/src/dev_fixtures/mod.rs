//! Debug-only fixture seeding and onboarding harness (specs/0059).
//! Compiled out of release builds; every entry point re-checks the `.debug` identifier.
#![cfg(debug_assertions)]

pub mod audio;
pub mod dataset;
pub mod folder;
pub mod guard;
pub mod seed;
pub mod wav;

use anyhow::{Context, Result};
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
        let f = folder::write_folder(m, &root, start)?;
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
        folders.insert(m.id.clone(), f);
    }
    let report = seed::seed_all(pool, &ds, &folders, now).await?;

    // A seeded profile is post-onboarding unless --onboarding also asked for a reset.
    if !guard::DevFlags::from_env().reset_onboarding {
        let status = crate::onboarding::OnboardingStatus {
            version: "1.0".into(),
            completed: true,
            current_step: 4,
            model_status: crate::onboarding::ModelStatus {
                parakeet: "downloaded".into(),
                summary: "downloaded".into(),
                selected_summary_model: None,
            },
            last_updated: now.to_rfc3339(),
        };
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
