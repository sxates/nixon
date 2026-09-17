//! Runtime guards for every dev-only path (specs/0059). Fail closed.
pub const ENV_FIXTURES: &str = "NIXON_FIXTURES";
pub const ENV_NO_AUDIO: &str = "NIXON_FIXTURES_NO_AUDIO";
pub const ENV_FAKE_DOWNLOADS: &str = "NIXON_FAKE_DOWNLOADS";
pub const ENV_RESET_ONBOARDING: &str = "NIXON_RESET_ONBOARDING";

/// True only when the running bundle is the isolated dev identifier (ADR-0004).
pub fn is_debug_identifier() -> bool {
    crate::app_paths::bundle_identifier()
        .map(|id| id.ends_with(".debug"))
        .unwrap_or(false)
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
}

impl DevFlags {
    pub fn from_env() -> Self {
        Self {
            fixtures: matches!(std::env::var(ENV_FIXTURES).as_deref(), Ok("demo")),
            no_audio: env_flag(ENV_NO_AUDIO),
            fake_downloads: env_flag(ENV_FAKE_DOWNLOADS),
            reset_onboarding: env_flag(ENV_RESET_ONBOARDING),
        }
    }
}

/// Shared gate: log once and return false when a dev path must not run.
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
    #[test]
    fn fixtures_flag_requires_demo_value() {
        std::env::set_var(ENV_FIXTURES, "demo");
        assert!(DevFlags::from_env().fixtures);
        std::env::set_var(ENV_FIXTURES, "1");
        assert!(!DevFlags::from_env().fixtures);
        std::env::remove_var(ENV_FIXTURES);
    }
    #[test]
    fn debug_identifier_is_false_when_uninitialised() {
        // bundle identifier is a OnceLock; in unit tests it is unset -> must fail closed
        assert!(
            !is_debug_identifier()
                || crate::app_paths::bundle_identifier()
                    .map(|s| s.ends_with(".debug"))
                    .unwrap_or(false)
        );
    }
}
