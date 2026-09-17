//! specs/0058 — in-app updates.
//!
//! `state` is the pure state machine (unit-tested, no Tauri), `settings` the one
//! persisted preference, `driver` the only file that touches `tauri_plugin_updater`,
//! and `commands` the IPC surface. The unattended loop lives here.

pub mod commands;
pub mod driver;
pub mod settings;
pub mod state;

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, Runtime};

pub use state::{InstallRefusal, UpdateStatus, UpdaterCore};

pub const EVENT_NAME: &str = "update-status";
const INITIAL_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// App-managed updater state. `core` is the pure machine; the staged payload lives on
/// disk (see `driver`), so nothing here holds the tarball.
pub struct UpdaterState {
    pub core: Mutex<UpdaterCore>,
}

impl Default for UpdaterState {
    fn default() -> Self {
        Self {
            core: Mutex::new(UpdaterCore::default()),
        }
    }
}

/// Broadcast the current status to the webview.
pub fn emit_status<R: Runtime>(app: &AppHandle<R>) {
    let status = app
        .state::<UpdaterState>()
        .core
        .lock()
        .unwrap()
        .status()
        .clone();
    if let Err(e) = app.emit(EVENT_NAME, &status) {
        log::debug!("updater: emit failed: {e}");
    }
}

/// Version of the staged payload, for the tray menu. `try_lock` so a menu rebuild can
/// never block on a download callback.
pub fn ready_version<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let state = app.try_state::<UpdaterState>()?;
    let core = state.core.try_lock().ok()?;
    core.staged_version().map(str::to_string)
}

/// The unattended loop: launch delay, then check every CHECK_INTERVAL while
/// `auto_update` is on. Disabled entirely by NIXON_DISABLE_UPDATER (dev builds).
pub fn spawn_update_loop<R: Runtime>(app: AppHandle<R>) {
    if std::env::var_os("NIXON_DISABLE_UPDATER").is_some() {
        log::info!("updater: disabled by NIXON_DISABLE_UPDATER");
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        loop {
            if settings::load_settings().await.auto_update {
                driver::check_and_download(&app).await;
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}
