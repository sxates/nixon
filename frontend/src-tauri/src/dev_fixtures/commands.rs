//! Debug-only IPC (specs/0059). Registered under `#[cfg(debug_assertions)]` in registry.rs.
use super::{guard, seed::SeedReport};
use tauri::{AppHandle, Emitter};

#[tauri::command]
pub async fn dev_load_fixtures(app: AppHandle, no_audio: bool) -> Result<SeedReport, String> {
    if !guard::allowed("dev_load_fixtures") {
        return Err("not a .debug build".into());
    }
    super::run_seed(&app, no_audio)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn dev_reset_onboarding(app: AppHandle) -> Result<(), String> {
    if !guard::allowed("dev_reset_onboarding") {
        return Err("not a .debug build".into());
    }
    crate::onboarding::reset_onboarding_status(&app)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit("onboarding-reset", ());
    Ok(())
}

#[tauri::command]
pub async fn dev_get_flags() -> Result<guard::DevFlags, String> {
    Ok(guard::DevFlags::from_env())
}

/// specs/0060: the `ready` control command can't read anything back from a fire-and-forget
/// `eval`, so it round-trips through this command instead — the injected JS invokes it with
/// whether `document.documentElement.dataset.shotReady` is `"1"`, and the listener polls
/// `control::SHOT_READY` until that's true.
#[tauri::command]
pub async fn dev_shot_ping(ready: bool) -> Result<(), String> {
    super::control::SHOT_READY.store(ready, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}
