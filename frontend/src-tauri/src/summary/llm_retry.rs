//! Whether a failed LLM call is worth retrying (spec 0028), split out of `processor.rs`.
//!
//! We only have the error *string* here, so the classifier matches markers in it. The
//! subscription-CLI copy (specs/0085, `cli_llm/errors.rs`) is pinned against it below: a
//! signed-out account, a spent plan, a missing or outdated CLI and an isolation stop are
//! terminal, and are checked before any transient marker, so a copy edit can't make a plan
//! limit retry.

/// Terminal for the whole run, not just this request: the account, key or provider is
/// unusable, so every later call would fail the same way. The map pass stops on these
/// (`processor.rs`) instead of dropping one chunk and trying the next.
const RUN_TERMINAL_MARKERS: [&str; 12] = [
    "invalid api key",
    "api key not found",
    "authentication",
    "unauthorized",
    "not configured",
    "unsupported llm provider",
    "unknown model",
    // Subscription CLIs (specs/0085, `cli_llm/errors.rs` copy, lowercased).
    "couldn't reach", // NotSignedIn: "Nixon couldn't reach Claude with your account"
    "usage limit is reached", // UsageLimit: the plan is spent until it resets
    "wasn't found on this mac", // NotFound
    "too old for nixon", // TooOld
    "nixon stopped",  // IsolationFailed / OutputTooLarge
];

/// Terminal for this request only (a malformed or oversized request).
const REQUEST_TERMINAL_MARKERS: [&str; 2] = ["invalid_request", "invalid request"];

/// Whether `e` means every further call in this run would fail too (see
/// [`RUN_TERMINAL_MARKERS`]). Never true for a cancellation, which callers handle first.
pub(crate) fn is_run_terminal_llm_error(e: &str) -> bool {
    let lower = e.to_lowercase();
    RUN_TERMINAL_MARKERS.iter().any(|m| lower.contains(m))
}

/// Classifies an LLM error string as transient (worth retrying) vs terminal.
///
/// Conservative on purpose: only clearly-transient conditions (timeouts, network
/// send failures, provider overload / rate-limit / 5xx) are retried. Everything
/// else — auth failures, malformed requests, cancellation — is treated as terminal
/// so we don't hammer a provider on a request that can never succeed. We only have
/// the error *string* (no status code) here, so we match on well-known markers.
/// Terminal markers are checked before transient ones.
pub(crate) fn is_retryable_llm_error(e: &str) -> bool {
    if e.contains("cancelled") {
        return false;
    }
    let lower = e.to_lowercase();
    if is_run_terminal_llm_error(e) || REQUEST_TERMINAL_MARKERS.iter().any(|m| lower.contains(m)) {
        return false;
    }
    let transient_markers = [
        "timed out",
        "timeout",
        "didn't answer within", // CLI Timeout: "Claude Code didn't answer within 5 minutes."
        "failed to send request",
        "error sending request",
        "connection",
        "connect",
        "reset by peer",
        "overloaded",
        "rate limit",
        "rate_limit",
        "too many requests",
        "429",
        "500",
        "502",
        "503",
        "504",
        "server_error",
        "internal server error",
        "temporarily unavailable",
        "service unavailable",
    ];
    transient_markers.iter().any(|m| lower.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_llm::errors::{CliError, CliFailure};
    use crate::cli_llm::CliKind;

    #[test]
    fn retryable_errors_are_transient_only() {
        assert!(is_retryable_llm_error(
            "LLM request timed out after 300 seconds"
        ));
        assert!(is_retryable_llm_error(
            "Failed to send request to LLM: connection refused"
        ));
        assert!(is_retryable_llm_error(
            "LLM API request failed: overloaded_error"
        ));
        assert!(is_retryable_llm_error(
            "LLM API request failed: 503 Service Unavailable"
        ));
    }

    #[test]
    fn terminal_errors_are_not_retried() {
        assert!(!is_retryable_llm_error("Summary generation was cancelled"));
        assert!(!is_retryable_llm_error("Invalid API key format"));
        assert!(!is_retryable_llm_error(
            "LLM API request failed: invalid_request_error"
        ));
        assert!(!is_retryable_llm_error("Unsupported LLM provider: foo"));
    }

    /// Every `CliError`, so a new variant must be classified here before this compiles.
    fn every_cli_error() -> Vec<CliError> {
        let all = vec![
            CliError::NotFound,
            CliError::TooOld {
                // A version containing "429" must still be terminal.
                found: "2.1.429".into(),
                min: "2.1.500".into(),
            },
            CliError::NotSignedIn,
            CliError::UsageLimit {
                detail: "resets 5pm".into(),
            },
            CliError::UsageLimit {
                detail: String::new(),
            },
            CliError::RateLimited,
            CliError::IsolationFailed,
            CliError::Timeout,
            CliError::ProbeTimeout,
            CliError::Cancelled,
            CliError::BadOutput,
            CliError::OutputTooLarge,
            CliError::Crashed { code: 2 },
            CliError::Unknown,
        ];
        for e in &all {
            // Exhaustive on purpose: add the new variant to `all` and to `retryable`.
            match e {
                CliError::NotFound
                | CliError::TooOld { .. }
                | CliError::NotSignedIn
                | CliError::UsageLimit { .. }
                | CliError::RateLimited
                | CliError::IsolationFailed
                | CliError::Timeout
                | CliError::ProbeTimeout
                | CliError::Cancelled
                | CliError::BadOutput
                | CliError::OutputTooLarge
                | CliError::Crashed { .. }
                | CliError::Unknown => {}
            }
        }
        all
    }

    /// The spec's "In-call retry" column: only a transient rate limit and a timeout retry.
    fn retryable(e: &CliError) -> bool {
        matches!(e, CliError::RateLimited | CliError::Timeout)
    }

    #[test]
    fn cli_error_copy_is_pinned_against_the_classifier() {
        for kind in [CliKind::ClaudeCode, CliKind::CodexCli] {
            for error in every_cli_error() {
                let copy = CliFailure::new(kind, error.clone()).to_string();
                assert_eq!(
                    is_retryable_llm_error(&copy),
                    retryable(&error),
                    "{kind:?} {error:?}: {copy}"
                );
            }
        }
    }

    #[test]
    fn terminal_cli_markers_win_over_transient_ones() {
        // A usage limit whose detail happens to mention a rate limit or a 429 stays terminal.
        for detail in ["rate limit resets 5pm", "429 · resets 5pm", "timeout"] {
            let copy = CliFailure::new(
                CliKind::ClaudeCode,
                CliError::UsageLimit {
                    detail: detail.into(),
                },
            )
            .to_string();
            assert!(!is_retryable_llm_error(&copy), "{copy}");
        }
        // Each terminal CLI copy, with a transient word appended, stays terminal: every
        // marker is load-bearing, not just the ones whose copy collides today.
        for error in [
            CliError::NotSignedIn,
            CliError::NotFound,
            CliError::TooOld {
                found: "2.1.0".into(),
                min: "2.1.1".into(),
            },
            CliError::IsolationFailed,
        ] {
            let copy = CliFailure::new(CliKind::ClaudeCode, error).to_string();
            let copy = format!("{copy} (connection timeout)");
            assert!(!is_retryable_llm_error(&copy), "{copy}");
        }
    }

    #[test]
    fn run_terminal_errors_are_the_account_and_config_ones() {
        for kind in [CliKind::ClaudeCode, CliKind::CodexCli] {
            for error in every_cli_error() {
                let copy = CliFailure::new(kind, error.clone()).to_string();
                let want = matches!(
                    error,
                    CliError::NotFound
                        | CliError::TooOld { .. }
                        | CliError::NotSignedIn
                        | CliError::UsageLimit { .. }
                        | CliError::IsolationFailed
                        | CliError::OutputTooLarge
                );
                assert_eq!(is_run_terminal_llm_error(&copy), want, "{copy}");
            }
        }
        assert!(is_run_terminal_llm_error("Invalid API key format"));
        // Per-request failures still let the map pass go on to the next chunk.
        assert!(!is_run_terminal_llm_error(
            "LLM API request failed: invalid_request_error"
        ));
        assert!(!is_run_terminal_llm_error("Model returned an empty reply"));
    }
}
