//! Subscription CLI providers (specs/0085): Nixon runs the user's own installed AI CLI
//! (Claude Code, Codex) as a one-shot subprocess for each LLM call.
//!
//! `runner` owns the process (temp dir, env, stdin, capture, timeout, kill); `errors` owns
//! the failure kinds and their user-facing copy. The adapters and `generate()` arrive in
//! later tasks.

pub mod errors;
pub mod runner;

/// Which subscription CLI a call runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CliKind {
    ClaudeCode,
    CodexCli,
}

impl CliKind {
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
