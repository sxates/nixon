//! How the subscription CLIs plug into the app's one LLM choke point (specs/0085 task 6):
//! the provider strings, keyless config, context budget, display name, the API-key save
//! arm, and acceptance 9 (a CLI provider is never a default anywhere).

use super::CliKind;
use crate::database::repositories::setting::SettingsRepository;
use crate::summary::llm_client::{provider_name, LLMProvider};
use crate::summary::summary_engine::options::GenerationOptions;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// A pool with no tables at all: any code path that touches the DB fails on it.
async fn empty_pool() -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite")
}

#[test]
fn provider_strings_parse_and_unknown_ones_still_error() {
    assert_eq!(
        LLMProvider::from_str("claude-code"),
        Ok(LLMProvider::ClaudeCode)
    );
    assert_eq!(
        LLMProvider::from_str("codex-cli"),
        Ok(LLMProvider::CodexCli)
    );
    assert_eq!(
        LLMProvider::from_str("Claude-Code"),
        Ok(LLMProvider::ClaudeCode)
    );
    // The Anthropic API provider is unrelated and unchanged.
    assert_eq!(LLMProvider::from_str("claude"), Ok(LLMProvider::Claude));
    // Reserved, not parsed (Gemini was dropped).
    for s in ["gemini-cli", "claude_code", "codex", ""] {
        assert!(LLMProvider::from_str(s).is_err(), "{s:?} must not parse");
    }
}

#[test]
fn only_the_cli_providers_have_a_cli_kind() {
    assert_eq!(
        LLMProvider::ClaudeCode.cli_kind(),
        Some(CliKind::ClaudeCode)
    );
    assert_eq!(LLMProvider::CodexCli.cli_kind(), Some(CliKind::CodexCli));
    for p in [
        LLMProvider::OpenAI,
        LLMProvider::Claude,
        LLMProvider::Groq,
        LLMProvider::Ollama,
        LLMProvider::OpenRouter,
        LLMProvider::BuiltInAI,
        LLMProvider::CustomOpenAI,
    ] {
        assert_eq!(p.cli_kind(), None, "{p:?}");
    }
}

#[test]
fn display_names() {
    assert_eq!(provider_name(&LLMProvider::ClaudeCode), "Claude Code");
    assert_eq!(provider_name(&LLMProvider::CodexCli), "Codex");
}

#[tokio::test]
async fn context_budget_is_the_fixed_cloud_threshold() {
    use crate::summary::service::resolve_context_budget;
    assert_eq!(
        resolve_context_budget(&LLMProvider::ClaudeCode, "sonnet", None).await,
        180_000
    );
    assert_eq!(
        resolve_context_budget(&LLMProvider::CodexCli, "gpt-5", None).await,
        120_000
    );
}

#[tokio::test]
async fn cli_providers_need_no_api_key_and_no_db_read() {
    use crate::summary::provider_config::resolve_provider_config;
    let pool = empty_pool().await;
    for (s, provider) in [
        ("claude-code", LLMProvider::ClaudeCode),
        ("codex-cli", LLMProvider::CodexCli),
    ] {
        let config = resolve_provider_config(&pool, s, "sonnet")
            .await
            .unwrap_or_else(|e| panic!("{s}: {e:#}"));
        assert_eq!(config.provider, provider);
        assert_eq!(config.api_key, "");
        assert_eq!(config.ollama_endpoint, None);
        assert_eq!(config.custom_openai_endpoint, None);
    }
    // Control: a keyed provider does read the DB, and fails on this empty one.
    assert!(resolve_provider_config(&pool, "openai", "gpt-4o")
        .await
        .is_err());
}

#[tokio::test]
async fn saving_an_api_key_for_a_cli_provider_is_a_no_op() {
    let pool = empty_pool().await;
    for s in ["claude-code", "codex-cli"] {
        assert!(SettingsRepository::save_api_key(&pool, s, "sk-not-used")
            .await
            .is_ok());
    }
    assert!(
        SettingsRepository::save_api_key(&pool, "gemini-cli", "sk-x")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn codex_is_not_found_until_its_adapter_lands() {
    let err = super::generate(
        CliKind::CodexCli,
        None,
        "system",
        "user",
        &GenerationOptions::default(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        "Codex wasn't found on this Mac. Install it, or choose it in Settings → Summary."
    );
}

// ---- acceptance 9: never a default ----

#[test]
fn the_fresh_install_and_onboarding_provider_is_built_in_ai() {
    // `initialize_fresh_database` and `complete_onboarding` both save this constant.
    let provider = LLMProvider::from_str(crate::config::DEFAULT_SUMMARY_PROVIDER).unwrap();
    assert_eq!(provider, LLMProvider::BuiltInAI);
    assert_eq!(provider.cli_kind(), None);
}

#[test]
fn the_recommended_model_is_always_a_built_in_model() {
    use crate::summary::summary_engine::commands::recommend_summary_model;
    use crate::summary::summary_engine::models::get_model_by_name;
    for is_macos in [true, false] {
        for ram in [0, 4, 8, 13, 14, 16, 32, 64, 128, 512] {
            let model = recommend_summary_model(is_macos, ram);
            assert!(
                get_model_by_name(model).is_some(),
                "{model} is not a bundled Built-in AI model"
            );
            assert!(
                LLMProvider::from_str(model).map_or(true, |p| p.cli_kind().is_none()),
                "{model}"
            );
        }
    }
}
