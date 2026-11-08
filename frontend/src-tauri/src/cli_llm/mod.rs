//! Subscription CLI providers (specs/0085): Nixon runs the user's own installed AI CLI
//! (Claude Code, Codex) as a one-shot subprocess for each LLM call.
//!
//! [`generate`] is the one entry `summary/llm_client.rs` calls. `runner` owns the process
//! (temp dir, env, stdin, capture, timeout, kill); `errors` owns the failure kinds and
//! their user-facing copy. `claude` is the Claude Code adapter. `discovery` finds the
//! binary and checks its version; `auth` asks it whether the user is signed in (neither
//! ever sends a prompt).

pub mod auth;
pub mod claude;
pub mod commands;
pub mod config;
pub mod discovery;
pub mod errors;
pub mod runner;
pub mod temp_sweep;

use crate::summary::summary_engine::options::GenerationOptions;
use errors::{CliError, CliFailure};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

/// Which subscription CLI a call runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CliKind {
    ClaudeCode,
    CodexCli,
}

impl CliKind {
    /// The provider string stored in `settings.provider` and sent over IPC.
    pub fn id(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "claude-code",
            CliKind::CodexCli => "codex-cli",
        }
    }

    /// The inverse of [`CliKind::id`].
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "claude-code" => Some(CliKind::ClaudeCode),
            "codex-cli" => Some(CliKind::CodexCli),
            _ => None,
        }
    }

    /// The CLI's name as users know it ("Claude Code").
    pub fn display_name(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "Claude Code",
            CliKind::CodexCli => "Codex",
        }
    }

    /// The command users type in Terminal ("claude").
    pub fn command_name(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "claude",
            CliKind::CodexCli => "codex",
        }
    }

    /// The service behind the CLI, as named in account and busy copy ("Claude").
    pub fn service_name(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "Claude",
            CliKind::CodexCli => "Codex",
        }
    }

    /// The subscription plan the CLI bills ("Claude" plan, "ChatGPT" plan).
    pub fn plan_name(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "Claude",
            CliKind::CodexCli => "ChatGPT",
        }
    }
}

/// One LLM call through `kind`'s CLI. Finds the binary (cached; `user_override` is the
/// Locate… path, and the login-shell probe never runs from here), runs the adapter, and
/// returns the reply or the user-facing copy of the failure. A `NotFound` forgets the
/// cached binary so the next call looks again.
///
/// From `options` only `json_schema` is used (the reply is then the validated structured
/// output as a JSON string); the CLIs expose no temperature, top-p or max-token control.
pub async fn generate(
    kind: CliKind,
    user_override: Option<PathBuf>,
    system: &str,
    user: &str,
    options: &GenerationOptions,
    cancel: Option<&CancellationToken>,
) -> Result<String, String> {
    let schema = options.json_schema.as_ref();
    let result = match kind {
        CliKind::ClaudeCode => {
            match discovery::discover(kind, user_override, false).await {
                Ok(found) => claude::generate(&found.path, system, user, schema, cancel).await,
                Err(e) => Err(e),
            }
        }
        // specs/0085 W2.
        CliKind::CodexCli => Err(CliError::NotFound),
    };
    result.map_err(|error| {
        if error == CliError::NotFound {
            discovery::invalidate(kind);
        }
        CliFailure::new(kind, error).to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::CliKind;

    #[test]
    fn names() {
        assert_eq!(CliKind::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(CliKind::CodexCli.display_name(), "Codex");
        assert_eq!(CliKind::ClaudeCode.command_name(), "claude");
        assert_eq!(CliKind::CodexCli.command_name(), "codex");
    }
}

#[cfg(test)]
mod wiring_tests;
