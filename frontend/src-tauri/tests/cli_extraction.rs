//! specs/0085 acceptance 7, end to end: `extract_candidates` with the Claude Code provider
//! passes the action-items schema to the CLI and turns its structured output into
//! candidates; a schema run with no structured output is a failure, not zero items.
//! Against the fake CLI only (the wrapper in `common_cli`).
//!
//! Run with:
//!   cargo test --features metal --test cli_extraction

mod common_cli;

use app_lib::action_items::extractor::{extract_candidates, OwnerIdentity};
use app_lib::action_items::schema::action_items_schema;
use app_lib::summary::llm_client::LLMProvider;
use app_lib::summary::provider_config::ProviderConfig;
use common_cli::FakeClaude;

fn claude_code() -> ProviderConfig {
    ProviderConfig {
        provider: LLMProvider::ClaudeCode,
        model_provider: "claude-code".into(),
        model_name: "sonnet".into(),
        api_key: String::new(),
        ollama_endpoint: None,
        custom_openai_endpoint: None,
        custom_openai_max_tokens: None,
        custom_openai_temperature: None,
        custom_openai_top_p: None,
    }
}

async fn extract(fake: &FakeClaude, mode: &str) -> anyhow::Result<usize> {
    fake.set_mode(mode);
    let candidates = extract_candidates(
        &reqwest::Client::new(),
        &claude_code(),
        None,
        "## Action items\n- Synthetic summary of a fictional demo meeting.",
        None,
        &[],
        &[],
        &OwnerIdentity::default(),
    )
    .await?;
    Ok(candidates.len())
}

#[tokio::test]
async fn schema_runs_yield_every_item_and_fail_without_structured_output() {
    let fake = FakeClaude::install().await;

    assert_eq!(extract(&fake, "json-3").await.unwrap(), 3);
    assert_eq!(fake.calls(), 1, "parsed first time, no JSON-only retry");
    let argv = fake.last_argv();
    let at = argv
        .iter()
        .position(|a| a == "--json-schema")
        .expect("the schema reaches the CLI");
    assert_eq!(argv[at + 1], action_items_schema().to_string());

    // "success" replays a plain-text capture: no structured_output on a schema run.
    let err = extract(&fake, "success").await.unwrap_err();
    assert!(
        format!("{err:#}").contains("Claude Code returned an answer Nixon couldn't read."),
        "{err:#}"
    );
}
