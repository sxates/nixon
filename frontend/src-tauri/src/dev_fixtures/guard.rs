//! Runtime guards for every dev-only path (specs/0059). Fail closed.
pub const ENV_FIXTURES: &str = "NIXON_FIXTURES";
pub const ENV_NO_AUDIO: &str = "NIXON_FIXTURES_NO_AUDIO";
pub const ENV_FAKE_DOWNLOADS: &str = "NIXON_FAKE_DOWNLOADS";
pub const ENV_RESET_ONBOARDING: &str = "NIXON_RESET_ONBOARDING";
pub const ENV_DEV_CONTROL: &str = "NIXON_DEV_CONTROL";

/// Pure check backing [`is_debug_identifier`]: an identifier counts as a dev
/// build only when present and suffixed with `.debug` (ADR-0004).
pub(crate) fn identifier_is_debug(id: Option<&str>) -> bool {
    id.map(|s| s.ends_with(".debug")).unwrap_or(false)
}

/// True only when the running bundle is the isolated dev identifier (ADR-0004).
pub fn is_debug_identifier() -> bool {
    identifier_is_debug(crate::app_paths::bundle_identifier())
}

pub fn env_flag(name: &str) -> bool {
    matches!(std::env::var(name).as_deref(), Ok("1"))
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct DevFlags {
    pub fixtures: bool,
    pub no_audio: bool,
    pub fake_downloads: bool,
    pub reset_onboarding: bool,
    pub control: bool,
}

impl DevFlags {
    /// Pure builder backing [`DevFlags::from_env`]: reads each variable through
    /// `lookup` instead of the process environment, so tests can supply values
    /// without mutating shared, unsynchronised process state.
    pub(crate) fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            fixtures: lookup(ENV_FIXTURES).as_deref() == Some("demo"),
            no_audio: matches!(lookup(ENV_NO_AUDIO).as_deref(), Some("1")),
            fake_downloads: matches!(lookup(ENV_FAKE_DOWNLOADS).as_deref(), Some("1")),
            reset_onboarding: matches!(lookup(ENV_RESET_ONBOARDING).as_deref(), Some("1")),
            control: lookup(ENV_DEV_CONTROL).as_deref() == Some("1"),
        }
    }

    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }
}

/// Shared gate: log and return false when a dev path must not run.
pub fn allowed(what: &str) -> bool {
    if is_debug_identifier() {
        true
    } else {
        log::warn!("[dev] {what} ignored: bundle identifier is not a .debug build");
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn env_flag_reads_1_and_demo_only() {
        std::env::set_var("NIXON_TEST_FLAG_A", "1");
        std::env::set_var("NIXON_TEST_FLAG_B", "0");
        std::env::remove_var("NIXON_TEST_FLAG_C");
        assert!(env_flag("NIXON_TEST_FLAG_A"));
        assert!(!env_flag("NIXON_TEST_FLAG_B"));
        assert!(!env_flag("NIXON_TEST_FLAG_C"));
    }
    fn lookup_from(
        vars: std::collections::HashMap<&'static str, &'static str>,
    ) -> impl Fn(&str) -> Option<String> {
        move |k: &str| vars.get(k).map(|v| v.to_string())
    }
    #[test]
    fn fixtures_flag_requires_demo_value() {
        use std::collections::HashMap;
        assert!(
            DevFlags::from_lookup(lookup_from(HashMap::from([(ENV_FIXTURES, "demo")]))).fixtures
        );
        assert!(!DevFlags::from_lookup(lookup_from(HashMap::from([(ENV_FIXTURES, "1")]))).fixtures);
        let all_set = DevFlags::from_lookup(lookup_from(HashMap::from([
            (ENV_FIXTURES, "demo"),
            (ENV_NO_AUDIO, "1"),
            (ENV_FAKE_DOWNLOADS, "1"),
            (ENV_RESET_ONBOARDING, "1"),
            (ENV_DEV_CONTROL, "1"),
        ])));
        assert!(all_set.fixtures);
        assert!(all_set.no_audio);
        assert!(all_set.fake_downloads);
        assert!(all_set.reset_onboarding);
        assert!(all_set.control);
        let none_set = DevFlags::from_lookup(lookup_from(HashMap::new()));
        assert!(!none_set.fixtures);
        assert!(!none_set.no_audio);
        assert!(!none_set.fake_downloads);
        assert!(!none_set.reset_onboarding);
        assert!(!none_set.control);
    }
    #[test]
    fn identifier_is_debug_requires_the_debug_suffix() {
        assert!(!identifier_is_debug(None));
        assert!(!identifier_is_debug(Some("ai.vinyl.app")));
        assert!(identifier_is_debug(Some("ai.vinyl.app.debug")));
        assert!(!identifier_is_debug(Some("debug")));
    }
    #[test]
    fn debug_identifier_is_false_when_uninitialised() {
        // The bundle identifier is a process-wide OnceLock that only the Tauri
        // `setup` hook ever populates; `cargo test --lib` never runs that hook,
        // so it is guaranteed unset here and this genuinely exercises the
        // fail-closed default rather than asserting a tautology.
        assert!(!is_debug_identifier());
    }
}
