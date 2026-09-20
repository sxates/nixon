//! specs/0058 — in-app updates.
//!
//! `state` is the pure state machine (unit-tested, no Tauri), `verify` the pure
//! digest check, `settings` the one persisted preference, `driver` the only file that
//! *calls* `tauri_plugin_updater`, and `commands` the IPC surface. The unattended loop
//! lives here.

pub mod commands;
pub mod driver;
pub mod settings;
pub mod state;
pub mod verify;

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, Runtime};

pub use state::{InstallRefusal, UpdateStatus, UpdaterCore};

pub const EVENT_NAME: &str = "update-status";
/// Emitted when an install attempt is refused, so the refusal is visible wherever the
/// user asked for it — the tray as well as Settings > About.
pub const EVENT_INSTALL_REFUSED: &str = "update-install-refused";
/// specs/0069 W5 — the tray asks the window to confirm rather than restarting behind the
/// user's back. The tray menu has no dialog of its own, and a restart is not undoable.
pub const EVENT_CONFIRM_RESTART: &str = "update-confirm-restart";
/// Dev builds set this to switch the whole feature off (loop, commands, toggle).
const DISABLE_VAR: &str = "NIXON_DISABLE_UPDATER";
/// The message both commands return while that kill switch is on.
pub const DISABLED_MESSAGE: &str = "Updates are disabled in this build";
const INITIAL_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// App-managed updater state. `core` is the pure machine; the staged payload lives on
/// disk (see `driver`), so nothing here holds the tarball.
///
/// `pending` holds the plugin's `Update` handle for whatever is staged. Keeping it is
/// what lets the install path be entirely offline: the handle carries the extraction
/// context, so with it plus the digest recorded in `core` we can install verified bytes
/// without a second `check()` (which would need the network and could hand back a
/// *different* release than the one on disk).
///
/// `in_flight` is the single-flight latch: the unattended loop, a manual check and the
/// check kicked by turning the preference on can all fire at once, and two concurrent
/// runs would interleave progress counters and let one wipe the other's staged tarball
/// out from under a `Ready` status. Only one check runs at a time; the losers are no-ops.
pub struct UpdaterState {
    pub core: Mutex<UpdaterCore>,
    pub pending: Mutex<Option<tauri_plugin_updater::Update>>,
    in_flight: AtomicBool,
}

impl Default for UpdaterState {
    fn default() -> Self {
        Self {
            core: Mutex::new(UpdaterCore::default()),
            pending: Mutex::new(None),
            in_flight: AtomicBool::new(false),
        }
    }
}

impl UpdaterState {
    /// Claim the single check slot. `None` means a check is already running — the caller
    /// must do nothing at all (not even touch `core`). The slot is released when the
    /// returned guard drops, including on an early return or a panic.
    pub fn try_begin_check(&self) -> Option<CheckGuard<'_>> {
        self.in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| CheckGuard {
                flag: &self.in_flight,
            })
    }
}

/// RAII release of the single-flight latch (see [`UpdaterState::try_begin_check`]).
pub struct CheckGuard<'a> {
    flag: &'a AtomicBool,
}

impl Drop for CheckGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
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

/// Is the whole updater switched off for this build? One answer for the loop, the
/// preference toggle and both IPC commands — a kill switch that only stops the
/// background loop is not a kill switch.
pub fn updater_disabled() -> bool {
    disabled_by(std::env::var_os(DISABLE_VAR))
}

/// The decision, separated from the environment so it can be tested.
fn disabled_by(var: Option<OsString>) -> bool {
    var.is_some()
}

/// The unattended loop: launch delay, then check every CHECK_INTERVAL while
/// `auto_update` is on. Disabled entirely by NIXON_DISABLE_UPDATER (dev builds).
pub fn spawn_update_loop<R: Runtime>(app: AppHandle<R>) {
    if updater_disabled() {
        log::info!("updater: disabled by {DISABLE_VAR}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kill_switch_is_presence_not_value() {
        assert!(!disabled_by(None));
        assert!(disabled_by(Some(OsString::from("1"))));
        // Anything at all switches it off, including an empty value — the variable is a
        // dev-build marker, not a tri-state flag.
        assert!(disabled_by(Some(OsString::from(""))));
        assert!(disabled_by(Some(OsString::from("0"))));
    }

    #[test]
    fn single_flight_latch_admits_one_and_releases_on_drop() {
        let state = UpdaterState::default();
        let first = state
            .try_begin_check()
            .expect("first check claims the slot");
        assert!(
            state.try_begin_check().is_none(),
            "a concurrent check must be refused while one is in flight"
        );
        drop(first);
        assert!(
            state.try_begin_check().is_some(),
            "the slot is free again once the guard drops"
        );
    }

    #[test]
    fn latch_is_released_even_when_the_check_panics() {
        let state = UpdaterState::default();
        // Silence the deliberate panic so the test output stays pristine.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = state.try_begin_check().expect("claims the slot");
            panic!("driver blew up");
        }));
        std::panic::set_hook(prev);
        assert!(result.is_err());
        assert!(
            state.try_begin_check().is_some(),
            "a panicking check must not wedge the latch shut"
        );
    }
}
