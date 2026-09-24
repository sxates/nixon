//! Remember the main window's size and position across launches (owner feedback
//! 2026-09-24: "every time I start Nixon it resets my window size").
//!
//! `tauri-plugin-window-state` restores the saved frame when the window is created and
//! saves it on `RunEvent::Exit`, which covers the tray's Quit and ⌘Q. The updater's
//! `app.restart()` on the main thread skips `RunEvent::Exit`, so it calls [`save`] first.
//! The state file is `.window-state.json` in the app config dir, so the dev and
//! production builds keep separate sizes (ADR-0004).

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{plugin::TauriPlugin, AppHandle, Runtime};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

/// Size, position and maximized — not visibility: the main window hides to the tray on
/// close, and restoring "hidden" would launch Nixon with no window.
const FLAGS: StateFlags = StateFlags::SIZE
    .union(StateFlags::POSITION)
    .union(StateFlags::MAXIMIZED);

/// Whether the plugin was registered; [`save`] must not touch its state otherwise.
static REGISTERED: AtomicBool = AtomicBool::new(false);

/// The plugin, or `None` under the dev control channel: screenshot runs resize the window
/// to fixed capture sizes, which must not become the developer's saved size.
pub fn plugin<R: Runtime>() -> Option<TauriPlugin<R>> {
    use crate::dev_fixtures::guard;
    if guard::is_debug_identifier() && guard::env_flag(guard::ENV_DEV_CONTROL) {
        return None;
    }
    REGISTERED.store(true, Ordering::SeqCst);
    Some(
        tauri_plugin_window_state::Builder::default()
            .with_state_flags(FLAGS)
            .build(),
    )
}

/// Save the window's current frame now. For exits that bypass `RunEvent::Exit`.
pub fn save<R: Runtime>(app: &AppHandle<R>) {
    if !REGISTERED.load(Ordering::SeqCst) {
        return;
    }
    if let Err(e) = app.save_window_state(FLAGS) {
        log::warn!("window state: could not save before exit: {e}");
    }
}
