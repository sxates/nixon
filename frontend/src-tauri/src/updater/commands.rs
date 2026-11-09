//! specs/0058 — IPC surface. Names are the frontend `invoke()` strings.

use tauri::{AppHandle, Manager};

use super::{driver, settings, updater_disabled, UpdateStatus, UpdaterState, DISABLED_MESSAGE};

#[tauri::command]
pub async fn api_get_update_status(app: AppHandle) -> Result<UpdateStatus, String> {
    let status = app
        .state::<UpdaterState>()
        .core
        .lock()
        .unwrap()
        .status()
        .clone();
    Ok(status)
}

/// Manual check from Settings > About. Runs regardless of `auto_update` — asking is
/// consent — and downloads if something newer is offered. NIXON_DISABLE_UPDATER wins
/// over the asking: a build with the updater switched off has no update path at all.
#[tauri::command]
pub async fn api_check_for_updates(app: AppHandle) -> Result<UpdateStatus, String> {
    if updater_disabled() {
        return Err(DISABLED_MESSAGE.to_string());
    }
    driver::check_and_download(&app).await;
    let status = app
        .state::<UpdaterState>()
        .core
        .lock()
        .unwrap()
        .status()
        .clone();
    Ok(status)
}

/// Install the staged update and relaunch. Refused (typed message) unless Ready and
/// the recorder is stopped; the UI disables the control in that case, this is the
/// backstop. Refusals also go out as `update-install-refused` (see `driver`).
#[tauri::command]
pub async fn api_install_update(app: AppHandle) -> Result<(), String> {
    if updater_disabled() {
        return Err(DISABLED_MESSAGE.to_string());
    }
    driver::install_and_restart(app).await
}

#[tauri::command]
pub async fn api_get_updater_settings() -> Result<settings::UpdaterSettings, String> {
    Ok(settings::load_settings().await)
}

/// Persist the toggle; turning it on kicks an immediate check. Read-modify-write, so the
/// early access setting beside it survives (specs/0086).
#[tauri::command]
pub async fn api_set_updater_settings(app: AppHandle, auto_update: bool) -> Result<(), String> {
    settings::update_settings(|s| s.auto_update = auto_update)
        .await
        .map_err(|e| format!("Failed to save update setting: {e}"))?;
    if auto_update && !updater_disabled() {
        tauri::async_runtime::spawn(async move { driver::check_and_download(&app).await });
    }
    Ok(())
}

/// specs/0086 — the Early access updates switch.
///
/// On: check the early feed right away, but only if automatic downloads are on
/// (DECIDED-3); otherwise Check for updates in About is how the early build is pulled.
/// Off: forget anything staged, since it may be an early build the user just opted out
/// of, then check the everyone feed if automatic downloads are on (DECIDED-1). Nothing
/// is downgraded: the everyone feed only offers versions newer than the installed one.
#[tauri::command]
pub async fn api_set_early_access(app: AppHandle, enabled: bool) -> Result<(), String> {
    let saved = settings::update_settings(|s| s.early_access = enabled)
        .await
        .map_err(|e| format!("Failed to save the early access setting: {e}"))?;
    if !enabled {
        let state = app.state::<UpdaterState>();
        // Bump before forgetting: an early check still in flight compares this count
        // under the core lock, so it either sees the bump or is forgotten here after it.
        state.note_early_opt_out();
        let staged = state.core.lock().unwrap().staged_version().is_some();
        if staged {
            driver::forget_staged(&app, "early access was switched off");
        }
    }
    if saved.auto_update && !updater_disabled() {
        tauri::async_runtime::spawn(async move { driver::check_and_download(&app).await });
    }
    Ok(())
}

/// specs/0069 W6 — the first launch after an install asks for the note the old version left.
/// Take-once: the file is consumed here, so a second window or a reload shows nothing.
#[tauri::command]
pub async fn api_take_update_receipt(
    app: AppHandle,
) -> Result<Option<super::receipt::UpdateReceipt>, String> {
    Ok(super::receipt::take(&app.package_info().version.to_string()))
}
