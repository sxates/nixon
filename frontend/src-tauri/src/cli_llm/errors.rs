//! Failure kinds for subscription CLI calls (specs/0085) and their user-facing copy.
//!
//! `CliError` is kind-free so the runner can produce it without knowing which CLI it ran.
//! The copy needs the CLI's name, so it lives on [`CliError::message`], and
//! [`CliFailure`] pairs an error with its [`CliKind`] for `Display`. That `Display` string
//! is what reaches `TaskOutcome::Failed.error` and `update_process_failed`, and
//! `processor.rs::is_retryable_llm_error` matches markers in it, so the copy is pinned by
//! the tests below.

use super::CliKind;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    NotFound,
    TooOld {
        found: String,
        min: String,
    },
    NotSignedIn,
    /// `detail` is the CLI's reset hint ("resets 5pm"), or empty when it gave none.
    UsageLimit {
        detail: String,
    },
    RateLimited,
    IsolationFailed,
    Timeout,
    /// A quick probe (`--version`, `auth status`) got no answer in time. Not `Timeout`: that
    /// copy says "5 minutes", which is false for a 10-second check.
    ProbeTimeout,
    Cancelled,
    BadOutput,
    OutputTooLarge,
    /// Non-zero exit. A signal death is reported shell-style as 128 + signal.
    Crashed {
        code: i32,
    },
    Unknown,
}

impl CliError {
    /// The user-facing copy for this error from `kind`'s CLI.
    pub fn message(&self, kind: CliKind) -> String {
        let cli = kind.display_name();
        let service = kind.service_name();
        match self {
            CliError::NotFound => format!(
                "{cli} wasn't found on this Mac. Install it, or choose it in Settings → Summary."
            ),
            CliError::TooOld { found, min } => {
                let fix = match kind {
                    CliKind::ClaudeCode => "Run `claude update` in Terminal.",
                    CliKind::CodexCli => "Update Codex in Terminal.",
                };
                format!("{cli} {found} is too old for Nixon (needs {min}). {fix}")
            }
            CliError::NotSignedIn => format!(
                "Nixon couldn't reach {service} with your account. Make sure you're signed in \
                 to {cli} on this Mac, then press Retry."
            ),
            CliError::UsageLimit { detail } => {
                let plan = kind.plan_name();
                let detail = detail.trim();
                let detail = if detail.is_empty() {
                    String::new()
                } else {
                    format!(" ({detail})")
                };
                format!(
                    "Your {plan} plan's usage limit is reached{detail}. Press Retry after it \
                     resets, or pick another summary model in Settings."
                )
            }
            CliError::RateLimited => format!("{service} is busy right now (rate limited)."),
            CliError::IsolationFailed => format!(
                "Nixon stopped {cli} because it started with tools or plugins enabled. \
                 Nothing was sent."
            ),
            CliError::Timeout => format!("{cli} didn't answer within 5 minutes."),
            CliError::ProbeTimeout => {
                format!("{cli} didn't respond. Check again in a moment.")
            }
            // The existing summary-cancel string, so cancel handling stays uniform.
            CliError::Cancelled => "Summary generation was cancelled".to_string(),
            CliError::BadOutput => format!("{cli} returned an answer Nixon couldn't read."),
            CliError::OutputTooLarge => {
                format!("{cli} returned more output than Nixon can handle, so Nixon stopped it.")
            }
            CliError::Crashed { code } => {
                format!("{cli} stopped unexpectedly (exit code {code}).")
            }
            CliError::Unknown => format!(
                "Something went wrong running {cli}. Press Retry, or pick another summary \
                 model in Settings."
            ),
        }
    }

    /// A short, content-free label for logs ("timeout").
    pub fn log_label(&self) -> &'static str {
        match self {
            CliError::NotFound => "not_found",
            CliError::TooOld { .. } => "too_old",
            CliError::NotSignedIn => "not_signed_in",
            CliError::UsageLimit { .. } => "usage_limit",
            CliError::RateLimited => "rate_limited",
            CliError::IsolationFailed => "isolation_failed",
            CliError::Timeout => "timeout",
            CliError::ProbeTimeout => "probe_timeout",
            CliError::Cancelled => "cancelled",
            CliError::BadOutput => "bad_output",
            CliError::OutputTooLarge => "output_too_large",
            CliError::Crashed { .. } => "crashed",
            CliError::Unknown => "unknown",
        }
    }
}

/// A [`CliError`] together with the CLI that produced it; `Display` is the user copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliFailure {
    pub kind: CliKind,
    pub error: CliError,
}

impl CliFailure {
    pub fn new(kind: CliKind, error: CliError) -> Self {
        Self { kind, error }
    }
}

impl fmt::Display for CliFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error.message(self.kind))
    }
}

impl std::error::Error for CliFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: CliKind = CliKind::ClaudeCode;
    const CODEX: CliKind = CliKind::CodexCli;

    fn too_old() -> CliError {
        CliError::TooOld {
            found: "2.1.200".into(),
            min: "2.1.290".into(),
        }
    }

    fn limit(detail: &str) -> CliError {
        CliError::UsageLimit {
            detail: detail.into(),
        }
    }

    #[test]
    fn claude_copy_matches_the_spec_table() {
        let cases = [
            (CliError::NotFound, "Claude Code wasn't found on this Mac. Install it, or choose it in Settings → Summary."),
            (too_old(), "Claude Code 2.1.200 is too old for Nixon (needs 2.1.290). Run `claude update` in Terminal."),
            (CliError::NotSignedIn, "Nixon couldn't reach Claude with your account. Make sure you're signed in to Claude Code on this Mac, then press Retry."),
            (limit("resets 5pm"), "Your Claude plan's usage limit is reached (resets 5pm). Press Retry after it resets, or pick another summary model in Settings."),
            (CliError::RateLimited, "Claude is busy right now (rate limited)."),
            (CliError::IsolationFailed, "Nixon stopped Claude Code because it started with tools or plugins enabled. Nothing was sent."),
            (CliError::Timeout, "Claude Code didn't answer within 5 minutes."),
            (CliError::ProbeTimeout, "Claude Code didn't respond. Check again in a moment."),
            (CliError::Cancelled, "Summary generation was cancelled"),
            (CliError::BadOutput, "Claude Code returned an answer Nixon couldn't read."),
            (CliError::OutputTooLarge, "Claude Code returned more output than Nixon can handle, so Nixon stopped it."),
            (CliError::Crashed { code: 2 }, "Claude Code stopped unexpectedly (exit code 2)."),
            (CliError::Unknown, "Something went wrong running Claude Code. Press Retry, or pick another summary model in Settings."),
        ];
        for (error, want) in cases {
            assert_eq!(
                CliFailure::new(CLAUDE, error.clone()).to_string(),
                want,
                "{error:?}"
            );
        }
    }

    #[test]
    fn codex_copy_swaps_the_name_and_command() {
        let cases = [
            (CliError::NotFound, "Codex wasn't found on this Mac. Install it, or choose it in Settings → Summary."),
            (too_old(), "Codex 2.1.200 is too old for Nixon (needs 2.1.290). Update Codex in Terminal."),
            (CliError::NotSignedIn, "Nixon couldn't reach Codex with your account. Make sure you're signed in to Codex on this Mac, then press Retry."),
            (limit("resets 5pm"), "Your ChatGPT plan's usage limit is reached (resets 5pm). Press Retry after it resets, or pick another summary model in Settings."),
            (CliError::RateLimited, "Codex is busy right now (rate limited)."),
            (CliError::IsolationFailed, "Nixon stopped Codex because it started with tools or plugins enabled. Nothing was sent."),
            (CliError::Timeout, "Codex didn't answer within 5 minutes."),
            (CliError::ProbeTimeout, "Codex didn't respond. Check again in a moment."),
            (CliError::Cancelled, "Summary generation was cancelled"),
            (CliError::BadOutput, "Codex returned an answer Nixon couldn't read."),
            (CliError::OutputTooLarge, "Codex returned more output than Nixon can handle, so Nixon stopped it."),
            (CliError::Crashed { code: 2 }, "Codex stopped unexpectedly (exit code 2)."),
            (CliError::Unknown, "Something went wrong running Codex. Press Retry, or pick another summary model in Settings."),
        ];
        for (error, want) in cases {
            assert_eq!(
                CliFailure::new(CODEX, error.clone()).to_string(),
                want,
                "{error:?}"
            );
        }
    }

    #[test]
    fn usage_limit_without_detail_drops_the_parentheses() {
        assert_eq!(
            CliFailure::new(CLAUDE, limit("  ")).to_string(),
            "Your Claude plan's usage limit is reached. Press Retry after it resets, or pick another summary model in Settings."
        );
    }

    #[test]
    fn usage_limit_copy_never_says_rate_limit() {
        // processor.rs checks terminal markers first, but keep the transient marker out of
        // the terminal copy too, so an ordering slip can't make a plan limit retry.
        for kind in [CLAUDE, CODEX] {
            let msg = limit("resets 5pm").message(kind).to_lowercase();
            assert!(msg.contains("usage limit is reached"));
            assert!(!msg.contains("rate limit"), "{msg}");
        }
    }
}
