//! specs/0058 — the only file that touches `tauri_plugin_updater`.
//!
//! One check = fetch `latest.json`, and if it offers a strictly newer version than what
//! is staged, stream the tarball to `<app-data-dir>/updates/`, let the plugin verify the
//! minisign signature, and mark Ready. Install reads the staged file back and restarts.

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_updater::UpdaterExt;

use super::{emit_status, UpdaterState};

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

fn staging_dir() -> PathBuf {
    crate::app_paths::app_data_dir().join("updates")
}

fn staged_path(version: &str) -> PathBuf {
    staging_dir().join(format!("Nixon-{version}.app.tar.gz"))
}

/// Run one check (+ download when something newer is offered). Errors are folded into
/// the status; this never panics and never restarts the app.
pub async fn check_and_download<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<UpdaterState>();
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
        // tauri-plugin-updater 2.11: `download` verifies the minisign signature against
        // the configured pubkey BEFORE returning the bytes, so what we stage on disk is
        // already verified and `install` never sees unverified bytes.
        let bytes = update
            .download(
                move |chunk, total| {
                    let st = app_for_progress.state::<UpdaterState>();
                    st.core.lock().unwrap().progress(chunk as u64, total);
                    emit_status(&app_for_progress);
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
    let bytes = tokio::fs::read(staged_path(&version))
        .await
        .map_err(|e| format!("Staged update is missing ({e}); it will be downloaded again"))?;
    let updater = app
        .updater_builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "The update is no longer offered".to_string())?;
    if update.version != version {
        return Err(format!(
            "A different version ({}) is now offered; checking again",
            update.version
        ));
    }
    update.install(bytes).map_err(|e| e.to_string())?;
    log::info!("updater: installed {version}; restarting");
    app.restart();
}
