//! specs/0085 acceptance 5, end to end: `generate_summary_with_retry` with the Claude Code
//! provider, through `llm_client` → the gate → `cli_llm::generate` → discovery → the
//! adapter → the runner, against the fake CLI. Never runs a real `claude`. Also the
//! map-reduce pass of `generate_meeting_summary`: a terminal error stops it with its copy.
//!
//! The fake is reached through the wrapper in `common_cli` (see its docs).
//!
//! Run with:
//!   cargo test --features metal --test cli_summary_retry

mod common_cli;

use app_lib::cli_llm::{discovery, CliKind};
use app_lib::summary::llm_client::LLMProvider;
use app_lib::summary::processor::{
    chunk_text, generate_meeting_summary, generate_summary_with_retry,
};
use common_cli::FakeClaude;

async fn summarize(fake: &FakeClaude, mode: &str) -> (Result<String, String>, usize) {
    fake.set_mode(mode);
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
    (result, fake.calls())
}

#[tokio::test]
async fn terminal_cli_errors_make_one_attempt_and_rate_limits_retry() {
    let fake = FakeClaude::install().await;

    let (ok, n) = summarize(&fake, "success").await;
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
        let (result, n) = summarize(&fake, mode).await;
        assert_eq!(result, Err(copy.to_string()), "{mode}");
        assert_eq!(n, 1, "{mode}: exactly one attempt, no backoff retries");
    }

    // NotFound forgot the cached binary; the next call finds it again and works.
    assert!(discovery::cached(CliKind::ClaudeCode).is_none());
    let (ok, n) = summarize(&fake, "success").await;
    assert!(ok.is_ok(), "rediscovered after NotFound: {ok:?}");
    assert_eq!(n, 1);

    // A transient rate limit gets the normal bounded retries: 1 + MAX_LLM_RETRIES (2).
    let (result, n) = summarize(&fake, "rate-limit").await;
    assert_eq!(
        result,
        Err("Claude is busy right now (rate limited).".to_string())
    );
    assert_eq!(n, 3, "rate limited: the first attempt plus two retries");
}

/// Runs the map-reduce path (a transcript well over `THRESHOLD`) against the fake.
async fn summarize_chunked(fake: &FakeClaude, mode: &str) -> (Result<String, String>, usize) {
    const THRESHOLD: usize = 1_300;
    fake.set_mode(mode);
    let text = "Synthetic line from a fictional demo meeting about the launch plan. ".repeat(400);
    assert!(
        chunk_text(&text, THRESHOLD - 300, 100).len() >= 2,
        "needs chunking"
    );
    let result = generate_meeting_summary(
        &reqwest::Client::new(),
        &LLMProvider::ClaudeCode,
        "sonnet",
        "",
        &text,
        "Summarize.",
        THRESHOLD,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        false,
        None,
    )
    .await
    .map(|(markdown, _, _)| markdown);
    (result, fake.calls())
}

#[tokio::test]
async fn a_terminal_error_stops_the_map_pass_with_its_copy() {
    let fake = FakeClaude::install().await;
    let (result, n) = summarize_chunked(&fake, "not-logged-in").await;
    assert_eq!(
        result,
        Err("Nixon couldn't reach Claude with your account. Make sure you're signed in to Claude Code on this Mac, then press Retry.".to_string())
    );
    assert_eq!(n, 1, "the first chunk's terminal error stops the pass");
}

#[tokio::test]
async fn when_every_chunk_fails_the_last_error_copy_is_surfaced() {
    let fake = FakeClaude::install().await;
    let text_chunks = chunk_text(
        &"Synthetic line from a fictional demo meeting about the launch plan. ".repeat(400),
        1_000,
        100,
    )
    .len();
    let (result, n) = summarize_chunked(&fake, "rate-limit").await;
    assert_eq!(
        result,
        Err("Claude is busy right now (rate limited).".to_string())
    );
    assert_eq!(n, 3 * text_chunks, "each chunk gets its bounded retries");
}
