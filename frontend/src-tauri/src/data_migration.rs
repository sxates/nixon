//! One-time app-data migration across a bundle-identifier change.
//!
//! macOS derives the app-data directory, the single-instance lock, and TCC
//! permissions from the Tauri bundle identifier. When we change the identifier
//! (e.g. `com.vinyl.dev` → `ai.vinyl.app`, see `specs/0002` / ADR-0004), the new
//! app gets a *fresh, empty* data dir and would appear to lose every meeting.
//!
//! This module copies the user's data from the previous identifier's directory
//! into the current one on first launch under the new identifier. It is:
//! - **Non-destructive for the database/settings:** files are *copied*, leaving
//!   the legacy directory intact as a fallback.
//! - **Idempotent:** guarded by a marker file and a "new dir already has data"
//!   check, so it runs at most once and never clobbers newer data.
//! - **Best-effort:** failures are logged, not fatal — a partial/failed migration
//!   must not block startup (the app still opens, just without the old data).
//!
//! Models (large, re-downloadable) are *moved* (rename, instant on the same
//! volume) rather than copied, to avoid duplicating gigabytes on first launch.

use std::path::{Path, PathBuf};

/// Map the current bundle identifier to the previous one it superseded.
/// Returns `None` when there is no known predecessor (fresh identifier history,
/// upstream meetily, etc.) — in which case there is nothing to migrate.
fn legacy_identifier_for(current: &str) -> Option<&'static str> {
    match current {
        "ai.vinyl.app" => Some("com.vinyl.dev"),
        "ai.vinyl.app.debug" => Some("com.vinyl.dev.debug"),
        _ => None,
    }
}

/// Small per-file items copied wholesale (the DB and its WAL sidecars, plus the
/// JSON settings blobs). The model directory is handled separately.
const MIGRATED_FILES: &[&str] = &[
    "meeting_minutes.sqlite",
    "meeting_minutes.sqlite-wal",
    "meeting_minutes.sqlite-shm",
    "preferences.json",
    "recording_preferences.json",
    "onboarding-status.json",
    "zoom.json",
    "notifications.json",
];

/// Migrate user data from the previous identifier's directory into `new_dir`
/// (the current identifier's app-data dir) if needed. Never returns an error:
/// all failures are logged and swallowed so startup proceeds.
pub fn migrate_legacy_app_data(new_dir: &Path, current_identifier: &str) {
    let Some(old_identifier) = legacy_identifier_for(current_identifier) else {
        return;
    };

    let marker = new_dir.join(".migrated-from-legacy");
    if marker.exists() {
        return; // already migrated (or intentionally marked done)
    }

    let Some(parent) = new_dir.parent() else {
        log::warn!("data_migration: app-data dir has no parent; skipping");
        return;
    };
    let old_dir = parent.join(old_identifier);
    let old_db = old_dir.join("meeting_minutes.sqlite");
    if !old_db.exists() {
        // Fresh install (no legacy data) — nothing to do. Don't write a marker:
        // harmless to re-check next launch, and avoids masking a later restore.
        return;
    }

    if let Err(e) = std::fs::create_dir_all(new_dir) {
        log::error!("data_migration: could not create new app-data dir: {e}");
        return;
    }

    // Never overwrite data the new identifier has already accumulated.
    if new_dir.join("meeting_minutes.sqlite").exists() {
        log::info!(
            "data_migration: new dir already has a database; not importing legacy {old_identifier} data"
        );
        write_marker(
            &marker,
            old_identifier,
            "skipped (new dir already populated)",
        );
        return;
    }

    log::info!("data_migration: importing data from legacy identifier {old_identifier}");

    let mut copied = 0u32;
    for name in MIGRATED_FILES {
        let src = old_dir.join(name);
        if !src.exists() {
            continue;
        }
        match std::fs::copy(&src, new_dir.join(name)) {
            Ok(_) => copied += 1,
            Err(e) => log::error!("data_migration: failed copying {name}: {e}"),
        }
    }

    // Models: move (fast) when the new dir has none; fall back to a deep copy if
    // rename fails (e.g. cross-volume, which shouldn't happen under one home).
    let old_models = old_dir.join("models");
    let new_models = new_dir.join("models");
    if old_models.exists() && !new_models.exists() {
        if let Err(e) = std::fs::rename(&old_models, &new_models) {
            log::warn!("data_migration: model dir rename failed ({e}); falling back to copy");
            if let Err(e) =
                crate::audio::meeting_folder::copy_dir_recursive(&old_models, &new_models)
            {
                log::error!("data_migration: model dir copy failed: {e}");
            }
        }
    }

    log::info!("data_migration: imported {copied} file(s) + models from {old_identifier}");
    write_marker(&marker, old_identifier, "migrated");
}

fn write_marker(marker: &Path, old_identifier: &str, status: &str) {
    let body = format!("{status} from {old_identifier}\n");
    if let Err(e) = std::fs::write(marker, body) {
        log::error!("data_migration: failed writing marker file: {e}");
    }
}

/// Convenience used by the Tauri setup hook: resolve the migration inputs from
/// the app handle and run it.
pub fn run_at_startup<R: tauri::Runtime>(app: &tauri::AppHandle<R>, new_dir: PathBuf) {
    let identifier = app.config().identifier.clone();
    migrate_legacy_app_data(&new_dir, &identifier);
}
