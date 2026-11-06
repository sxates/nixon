//! specs/0085 acceptance 5, end to end: `generate_summary_with_retry` with the Claude Code
//! provider, through `llm_client` → the gate → `cli_llm::generate` → discovery → the
//! adapter → the runner, against the fake CLI. Never runs a real `claude`.
//!
//! Production calls pass no test env to the runner, so the fake's mode can't travel in
//! `FAKE_CLI_MODE` from here. A wrapper script stands in for the binary instead: it
//! answers `--version` (discovery), counts every other run, and execs the fake in the
//! mode named in a file this test rewrites between cases. Discovery finds the wrapper
//! through `NIXON_CLI_CLAUDE_CODE`, and HOME points at an empty temp dir, so no real
//! install can be picked up. The cases share that process-wide state, so they run in one
//! test, in order.
//!
//! Run with:
//!   cargo test --features metal --test cli_summary_retry

use app_lib::cli_llm::{discovery, CliKind};
use app_lib::summary::llm_client::LLMProvider;
use app_lib::summary::processor::generate_summary_with_retry;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn fake_cli() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli/fake-ai-cli.sh")
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli/claude")
}

/// Writes the wrapper into `dir` and returns its path.
fn write_wrapper(dir: &Path) -> PathBuf {
    let wrapper = dir.join("claude");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = --version ]; then exec '{fake}' --version; fi
echo run >>'{dir}/calls'
FAKE_CLI_MODE="$(cat '{dir}/mode')" FAKE_CLI_FIXTURES='{fixtures}' exec '{fake}' "$@"
"#,
        fake = fake_cli().display(),
        dir = dir.display(),
        fixtures = fixtures_dir().display(),
    );
    std::fs::write(&wrapper, script).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    wrapper
}

fn calls(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("calls"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

async fn summarize(dir: &Path, mode: &str) -> (Result<String, String>, usize) {
    std::fs::write(dir.join("mode"), mode).unwrap();
    let _ = std::fs::remove_file(dir.join("calls"));
    let result = generate_summary_with_retry(
        &reqwest::Client::new(),
        &LLMProvider::ClaudeCode,
        "sonnet",
        "",
        "Synthetic system prompt.",
        "Synthetic transcript for a fictional demo meeting.",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    (result, calls(dir))
}

#[tokio::test]
async fn terminal_cli_errors_make_one_attempt_and_rate_limits_retry() {
    let scratch = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let wrapper = write_wrapper(scratch.path());
    // Edition 2021: set_var is safe. Nothing else in this test binary reads these.
    std::env::set_var("NIXON_CLI_CLAUDE_CODE", &wrapper);
    std::env::set_var("HOME", home.path());

    let (ok, n) = summarize(scratch.path(), "success").await;
    assert!(
        ok.as_deref().unwrap().starts_with("**Summary"),
        "success returns the reply: {ok:?}"
    );
    assert_eq!(n, 1, "success: one run");

    let terminal = [
        (
            "not-logged-in",
            "Nixon couldn't reach Claude with your account. Make sure you're signed in to Claude Code on this Mac, then press Retry.",
        ),
        (
            "limit",
            "Your Claude plan's usage limit is reached (resets 3:45pm). Press Retry after it resets, or pick another summary model in Settings.",
        ),
        (
            // The binary is found but its wrapper can't exec: NotFound.
            "exec-fail",
            "Claude Code wasn't found on this Mac. Install it, or choose it in Settings → Summary.",
        ),
    ];
    for (mode, copy) in terminal {
        assert!(discovery::cached(CliKind::ClaudeCode).is_some(), "{mode}");
        let (result, n) = summarize(scratch.path(), mode).await;
        assert_eq!(result, Err(copy.to_string()), "{mode}");
        assert_eq!(n, 1, "{mode}: exactly one attempt, no backoff retries");
    }

    // NotFound forgot the cached binary; the next call finds it again and works.
    assert!(discovery::cached(CliKind::ClaudeCode).is_none());
    let (ok, n) = summarize(scratch.path(), "success").await;
    assert!(ok.is_ok(), "rediscovered after NotFound: {ok:?}");
    assert_eq!(n, 1);

    // A transient rate limit gets the normal bounded retries: 1 + MAX_LLM_RETRIES (2).
    let (result, n) = summarize(scratch.path(), "rate-limit").await;
    assert_eq!(
        result,
        Err("Claude is busy right now (rate limited).".to_string())
    );
    assert_eq!(n, 3, "rate limited: the first attempt plus two retries");
}
