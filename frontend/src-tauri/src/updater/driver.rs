//! specs/0058 — the only file that touches `tauri_plugin_updater`.
//!
//! One check = fetch `latest.json`, and if it offers a strictly newer version than what
//! is staged, stream the tarball to `<app-data-dir>/updates/`, let the plugin verify the
//! minisign signature, and mark Ready. Install reads the staged file back and restarts.

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_updater::UpdaterExt;

use super::{emit_status, InstallRefusal, UpdateStatus, UpdaterState};

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Fallback emit cadence while the server sends no Content-Length (no percent to track).
const PROGRESS_EMIT_BYTES: u64 = 256 * 1024;

fn staging_dir() -> PathBuf {
    crate::app_paths::app_data_dir().join("updates")
}

fn staged_path(version: &str) -> PathBuf {
    staging_dir().join(format!("Nixon-{version}.app.tar.gz"))
}

/// Forget the staged payload and tell everyone. Used when the file it names is gone from
/// disk or the manifest has moved on: without this the core keeps believing the version
/// is staged, `should_download` keeps returning false for it, and it is never
/// re-downloaded.
fn forget_staged<R: Runtime>(app: &AppHandle<R>, why: &str) {
    log::warn!("updater: forgetting the staged payload ({why})");
    {
        let state = app.state::<UpdaterState>();
        state.core.lock().unwrap().clear_staged(chrono::Utc::now());
    }
    emit_status(app);
    crate::tray::update_tray_menu(app);
}

/// Run one check (+ download when something newer is offered). Errors are folded into
/// the status; this never panics and never restarts the app.
pub async fn check_and_download<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<UpdaterState>();
    // Single-flight: the loop, a manual check and the preference-toggle check can all
    // fire at once. A second concurrent run would interleave progress counters and its
    // staging-dir wipe would delete the first run's tarball while `core` still says
    // Ready, so the loser does nothing at all — the caller reads the live status.
    let Some(_in_flight) = state.try_begin_check() else {
        log::debug!("updater: a check is already running; skipping this one");
        return;
    };
    // Self-heal before anything else: the staging record is only meaningful while its
    // file exists. If someone cleaned it out between ticks, forget it here so
    // `should_download` offers the same version again instead of skipping it forever.
    let staged_file_gone = {
        let staged = state
            .core
            .lock()
            .unwrap()
            .staged_version()
            .map(str::to_string);
        match staged {
            // On an IO error assume it is still there — never drop good state on a blip.
            Some(v) => !tokio::fs::try_exists(staged_path(&v)).await.unwrap_or(true),
            None => false,
        }
    };
    if staged_file_gone {
        forget_staged(app, "its file is no longer on disk");
    }

    {
        state.core.lock().unwrap().begin_check();
    }
    emit_status(app);

    let result = async {
        let updater = app.updater_builder().timeout(HTTP_TIMEOUT).build()?;
        let Some(update) = updater.check().await? else {
            return Ok::<_, tauri_plugin_updater::Error>(None);
        };
        let wanted = state.core.lock().unwrap().should_download(&update.version);
        if !wanted {
            return Ok(None);
        }
        let version = update.version.clone();
        let notes = update.body.clone().unwrap_or_default();
        {
            state.core.lock().unwrap().begin_download(&version);
        }
        emit_status(app);

        let app_for_progress = app.clone();
        // The byte counter advances on every chunk, but the IPC event does not: a
        // ~100 MB payload arrives in thousands of chunks and one event each would flood
        // the webview. Emit only when the integer percent changes (or every
        // PROGRESS_EMIT_BYTES when the server sent no Content-Length).
        let mut last_pct = u64::MAX;
        let mut last_emit_bytes = 0u64;
        // tauri-plugin-updater 2.11: `download` verifies the minisign signature against
        // the configured pubkey BEFORE returning the bytes, so what we stage on disk is
        // already verified and `install` never sees unverified bytes.
        let bytes = update
            .download(
                move |chunk, total| {
                    let st = app_for_progress.state::<UpdaterState>();
                    let (received, content_length) = {
                        let mut core = st.core.lock().unwrap();
                        core.progress(chunk as u64, total);
                        match core.status() {
                            UpdateStatus::Downloading {
                                received, total, ..
                            } => (*received, *total),
                            _ => return,
                        }
                    };
                    let emit = match content_length {
                        Some(len) if len > 0 => {
                            let pct = received.saturating_mul(100) / len;
                            let changed = pct != last_pct;
                            last_pct = pct;
                            changed
                        }
                        _ => {
                            let due =
                                received.saturating_sub(last_emit_bytes) >= PROGRESS_EMIT_BYTES;
                            if due {
                                last_emit_bytes = received;
                            }
                            due
                        }
                    };
                    if emit {
                        emit_status(&app_for_progress);
                    }
                },
                || {},
            )
            .await?;

        let dir = staging_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(tauri_plugin_updater::Error::Io)?;
        // Drop any older staged payloads so the dir never accumulates.
        if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
        tokio::fs::write(staged_path(&version), &bytes)
            .await
            .map_err(tauri_plugin_updater::Error::Io)?;
        Ok(Some((version, notes)))
    }
    .await;

    let now = chrono::Utc::now();
    {
        let mut core = state.core.lock().unwrap();
        match result {
            Ok(Some((version, notes))) => {
                log::info!("updater: {version} downloaded and verified; waiting for restart");
                core.ready(&version, &notes);
            }
            Ok(None) => core.found_none(now),
            Err(e) => {
                log::warn!("updater: check failed: {e}");
                core.failed(&e.to_string(), now);
            }
        }
    }
    emit_status(app);
    crate::tray::update_tray_menu(app);
}

/// Install the staged payload and relaunch. The caller has already passed
/// `UpdaterCore::can_install`; this re-reads the plugin metadata (cheap, one GET of
/// `latest.json`) so the plugin can install the verified on-disk bytes.
pub async fn install_and_restart<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    // Resolve the recorder state *before* taking the state handle so no lock or
    // `State` guard is held across an await.
    let stopped = !crate::audio::recording_commands::is_recording().await;
    let version = app
        .state::<UpdaterState>()
        .core
        .lock()
        .unwrap()
        .can_install(stopped)
        .map_err(|e| e.to_string())?;
    let bytes = match tokio::fs::read(staged_path(&version)).await {
        Ok(bytes) => bytes,
        Err(e) => {
            forget_staged(&app, "its file is missing at install time");
            return Err(format!(
                "Staged update is missing ({e}); it will be downloaded again"
            ));
        }
    };
    let updater = app
        .updater_builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| {
            log::warn!("updater: install-time check failed: {e}");
            "Couldn't reach GitHub to confirm the update; try again".to_string()
        })?
        .ok_or_else(|| "The update is no longer offered".to_string())?;
    if update.version != version {
        // The staged tarball is superseded: its bytes no longer match anything the
        // manifest signs for, so drop it rather than keep offering a dead restart.
        forget_staged(&app, "the manifest moved to a different version");
        return Err(format!(
            "A different version ({}) is now offered; checking again",
            update.version
        ));
    }
    // The gate above was evaluated before a network round trip that can take as long as
    // HTTP_TIMEOUT. Zoom auto-detect or the global record shortcut can have started a
    // recording in that window, so re-check immediately before the point of no return.
    if crate::audio::recording_commands::is_recording().await {
        return Err(InstallRefusal::RecordingInProgress.to_string());
    }
    // Extraction is synchronous filesystem work; keep it off the async runtime.
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|e| format!("Update install task failed: {e}"))?
        .map_err(|e| e.to_string())?;
    log::info!("updater: installed {version}; restarting");
    app.restart();
}
