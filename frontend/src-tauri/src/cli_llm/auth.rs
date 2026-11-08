//! Asks an installed AI CLI whether the user is signed in (specs/0085). No model call, no
//! prompt: `claude auth status` / `codex login status` answer locally in about 0.1 s.
//!
//! Privacy: the status JSON can carry the account email and org. Only `loggedIn` and
//! `authMethod` are deserialized, and nothing from the output is logged or stored.

use super::discovery::probe;
use super::errors::CliError;
use super::CliKind;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

/// Shown when the CLI is signed in with an API key rather than a subscription.
pub const API_KEY_NOTE: &str =
    "Signed in with an API key — usage is billed to that account, not a subscription";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthState {
    /// Usable. `note` is a heads-up to show beside the provider (the API-key billing note).
    Ready {
        note: Option<String>,
    },
    NeedsSignIn,
}

/// The only fields read from `claude auth status`.
#[derive(Deserialize)]
struct ClaudeStatus {
    #[serde(rename = "loggedIn")]
    logged_in: Option<bool>,
    #[serde(rename = "authMethod")]
    auth_method: Option<String>,
}

/// Checks `kind`'s sign-in with the production 10 s probe timeout.
pub async fn check(kind: CliKind, binary: &Path) -> Result<AuthState, CliError> {
    check_with(kind, binary, super::discovery::PROBE_TIMEOUT, &[]).await
}

/// [`check`] with an explicit timeout and the runner's test-only env (fake CLI mode/log).
pub async fn check_with(
    kind: CliKind,
    binary: &Path,
    timeout: Duration,
    test_env: &[(String, String)],
) -> Result<AuthState, CliError> {
    let args: &[&str] = match kind {
        CliKind::ClaudeCode => &["auth", "status"],
        CliKind::CodexCli => &["login", "status"],
    };
    let out = probe(kind, binary, args, timeout, test_env).await?;
    let code = out.exit_code();
    match kind {
        CliKind::ClaudeCode => classify_claude(code, &out.stdout),
        CliKind::CodexCli => classify_codex(code, &out.stdout, &out.stderr),
    }
}

fn classify_claude(code: i32, stdout: &[u8]) -> Result<AuthState, CliError> {
    let status: Option<ClaudeStatus> = serde_json::from_slice(stdout).ok();
    match code {
        0 => {
            let Some(status) = status else {
                return Err(CliError::BadOutput);
            };
            if status.logged_in == Some(false) {
                return Ok(AuthState::NeedsSignIn);
            }
            match status.auth_method.as_deref() {
                Some("claude.ai" | "oauth_token") => Ok(AuthState::Ready { note: None }),
                Some("api_key" | "api_key_helper" | "third_party") => Ok(AuthState::Ready {
                    note: Some(API_KEY_NOTE.to_string()),
                }),
                Some("none") => Ok(AuthState::NeedsSignIn),
                // A method this Nixon doesn't know: don't guess it is a subscription.
                _ => Err(CliError::BadOutput),
            }
        }
        code => classify_failed_exit(code),
    }
}

/// Exit 1 is the CLI's "signed out"; 127 means it couldn't run at all (a wrapper's
/// `node` is gone); anything else is a crash, never a reason to ask the user to sign in.
fn classify_failed_exit(code: i32) -> Result<AuthState, CliError> {
    match code {
        1 => Ok(AuthState::NeedsSignIn),
        127 => Err(CliError::NotFound),
        code => Err(CliError::Crashed { code }),
    }
}

fn classify_codex(code: i32, stdout: &[u8], stderr: &[u8]) -> Result<AuthState, CliError> {
    if code != 0 {
        return classify_failed_exit(code);
    }
    // Codex has printed its status on either stream across versions.
    let said_api_key = [stdout, stderr].iter().any(|s| {
        String::from_utf8_lossy(s)
            .to_lowercase()
            .contains("api key")
    });
    Ok(AuthState::Ready {
        note: said_api_key.then(|| API_KEY_NOTE.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNED_IN: &str =
        include_str!("../../tests/fixtures/cli/claude/auth-status-signed-in.json");
    const SIGNED_OUT: &str =
        include_str!("../../tests/fixtures/cli/claude/auth-status-signed-out.json");

    fn ready() -> AuthState {
        AuthState::Ready { note: None }
    }

    #[test]
    fn committed_fixtures_classify() {
        assert_eq!(classify_claude(0, SIGNED_IN.as_bytes()), Ok(ready()));
        assert_eq!(
            classify_claude(1, SIGNED_OUT.as_bytes()),
            Ok(AuthState::NeedsSignIn)
        );
    }

    #[test]
    fn oauth_token_is_a_subscription() {
        let json = br#"{"loggedIn":true,"authMethod":"oauth_token"}"#;
        assert_eq!(classify_claude(0, json), Ok(ready()));
    }

    #[test]
    fn api_key_methods_are_ready_with_the_billing_note() {
        for method in ["api_key", "api_key_helper", "third_party"] {
            let json = format!(r#"{{"loggedIn":true,"authMethod":"{method}"}}"#);
            assert_eq!(
                classify_claude(0, json.as_bytes()),
                Ok(AuthState::Ready {
                    note: Some(API_KEY_NOTE.to_string())
                }),
                "{method}"
            );
        }
    }

    #[test]
    fn signed_out_shapes_need_sign_in() {
        let none = br#"{"loggedIn":false,"authMethod":"none"}"#;
        assert_eq!(classify_claude(0, none), Ok(AuthState::NeedsSignIn));
        // Exit 1 wins even when the output is the `--text` sentence.
        let text = b"Not logged in. Run claude auth login to authenticate.";
        assert_eq!(classify_claude(1, text), Ok(AuthState::NeedsSignIn));
    }

    #[test]
    fn unreadable_or_unknown_is_not_guessed_as_ready() {
        assert_eq!(classify_claude(0, b"garbage"), Err(CliError::BadOutput));
        let unknown = br#"{"loggedIn":true,"authMethod":"mystery"}"#;
        assert_eq!(classify_claude(0, unknown), Err(CliError::BadOutput));
        assert_eq!(classify_claude(2, b""), Err(CliError::Crashed { code: 2 }));
    }

    #[test]
    fn codex_missing_or_crashed_is_not_a_sign_in_problem() {
        assert_eq!(classify_codex(127, b"", b""), Err(CliError::NotFound));
        assert_eq!(
            classify_codex(2, b"", b"panic"),
            Err(CliError::Crashed { code: 2 })
        );
        assert_eq!(
            classify_codex(137, b"", b""),
            Err(CliError::Crashed { code: 137 })
        );
    }

    #[test]
    fn claude_exit_127_is_not_found() {
        assert_eq!(classify_claude(127, b""), Err(CliError::NotFound));
    }

    #[test]
    fn codex_exit_code_and_api_key_mention() {
        assert_eq!(
            classify_codex(0, b"Logged in using ChatGPT", b""),
            Ok(ready())
        );
        assert_eq!(
            classify_codex(0, b"", b"Logged in using an API key - sk-***"),
            Ok(AuthState::Ready {
                note: Some(API_KEY_NOTE.to_string())
            })
        );
        assert_eq!(
            classify_codex(1, b"Not logged in", b""),
            Ok(AuthState::NeedsSignIn)
        );
    }
}
