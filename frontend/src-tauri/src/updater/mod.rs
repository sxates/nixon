//! specs/0058 — in-app updates.
//!
//! `state` is the pure state machine (unit-tested, no Tauri), `settings` the one
//! persisted preference, `driver` the only file that touches `tauri_plugin_updater`,
//! and `commands` the IPC surface. The unattended loop lives here.

pub mod settings;
pub mod state;

use std::sync::Mutex;

pub use state::{InstallRefusal, UpdateStatus, UpdaterCore};

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
