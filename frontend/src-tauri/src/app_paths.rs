//! Single source of truth for the app-data directory.
//!
//! Historically several subsystems built their own storage paths from a
//! hardcoded `"Meetily"`/`"meetily"` literal joined onto `dirs::data_dir()` /
//! `dirs::config_dir()` (templates, model fallbacks, Zoom/notification settings).
//! That bypassed the Tauri bundle identifier, so dev and prod shared the same
//! on-disk location — breaking the dev/prod isolation guaranteed by ADR-0004 and
//! leaking across the rebrand.
//!
//! Everything that needs an app-data path should go through [`app_data_dir`],
//! which returns the *identifier-derived* directory
//! (`~/Library/Application Support/<bundle-id>/` on macOS) — the same root the
//! database and downloaded models already use. The directory is captured once at
//! startup via [`init`] from the Tauri `setup` hook.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Product name — the single place the human-facing app name lives for any
/// non-Tauri fallback path. See `ADR-0002` (`APP_NAME` is a variable).
pub const APP_NAME: &str = "Nixon";

static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static BUNDLE_IDENTIFIER: OnceLock<String> = OnceLock::new();

/// Record the identifier-derived app-data directory. Call once from the Tauri
/// `setup` hook with `app.path().app_data_dir()`. Subsequent calls are ignored.
pub fn init(dir: PathBuf) {
    if APP_DATA_DIR.set(dir).is_err() {
        log::warn!("app_paths::init called more than once; keeping the first value");
    }
}

/// Record the Tauri bundle identifier (`ai.vinyl.app` / `ai.vinyl.app.debug`).
/// Call once from the Tauri `setup` hook with `app.config().identifier`. Used to
/// derive per-identifier names for OS-level stores (e.g. the Keychain service in
/// `secrets`, ADR-0009) so dev and prod stay isolated per ADR-0004.
pub fn init_bundle_identifier(identifier: String) {
    if BUNDLE_IDENTIFIER.set(identifier).is_err() {
        log::warn!(
            "app_paths::init_bundle_identifier called more than once; keeping the first value"
        );
    }
}

/// The bundle identifier captured at startup, or `None` when uninitialized
/// (e.g. unit tests with no Tauri app). Callers must degrade gracefully rather
/// than invent a fallback identifier — a wrong identifier would silently read
/// another install's Keychain items.
pub fn bundle_identifier() -> Option<&'static str> {
    BUNDLE_IDENTIFIER.get().map(String::as_str)
}

/// The app-data directory: the identifier-derived path set at startup, or — when
/// uninitialized (e.g. unit tests with no Tauri app) — a best-effort fallback of
/// `dirs::data_dir()/<APP_NAME>`. Callers join their own subpaths onto this.
pub fn app_data_dir() -> PathBuf {
    if let Some(dir) = APP_DATA_DIR.get() {
        return dir.clone();
    }
    dirs::data_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_nixon() {
        assert_eq!(APP_NAME, "Nixon");
    }

    #[test]
    fn fallback_app_data_dir_ends_with_app_name() {
        // Uninitialized (no Tauri app in tests): the fallback path is <data_dir>/<APP_NAME>.
        assert!(app_data_dir().ends_with("Nixon"));
    }
}
