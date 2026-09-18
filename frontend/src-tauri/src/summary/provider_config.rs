//! The resolved LLM provider credentials/endpoints for one generation run.
//!
//! [`resolve_provider_config`] is the single place that turns a stored provider/model
//! choice into everything a call needs (API key, Ollama endpoint, CustomOpenAI endpoint +
//! tuning). It replaces two previously duplicated ~40-line blocks: the summary run
//! (`summary/background.rs::process_transcript_background`) and the manual action-item
//! extraction command (`action_items/commands.rs::api_extract_action_items`). Action-item
//! extraction deliberately reuses the summary's provider posture (specs/0034, decided
//! 2026-07-03: no separate extraction provider setting), so the two must never drift.
//!
//! Errors are `anyhow` with user-actionable messages; callers surface them (manual
//! command) or persist them as the process failure (summary run).

use anyhow::{anyhow, Context};
use sqlx::SqlitePool;
use tracing::info;

use crate::database::repositories::setting::SettingsRepository;
use crate::summary::llm_client::LLMProvider;

/// Everything one LLM generation run needs to reach its provider. Built by
/// [`resolve_provider_config`]; threaded as ONE value instead of 6–7 positional params.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Parsed provider (drives the client dispatch).
    pub provider: LLMProvider,
    /// The provider's stored string form (e.g. `"ollama"`) — persisted to ledgers/rows.
    pub model_provider: String,
    /// Model identifier (e.g. `"llama3.2:latest"`, `"gpt-4o"`).
    pub model_name: String,
    /// API key; empty for keyless providers (Ollama, BuiltInAI, keyless CustomOpenAI).
    pub api_key: String,
    /// Custom Ollama endpoint, when the provider is Ollama and one is configured.
    pub ollama_endpoint: Option<String>,
    /// CustomOpenAI base URL, when the provider is CustomOpenAI.
    pub custom_openai_endpoint: Option<String>,
    /// CustomOpenAI per-endpoint tuning (summary generation only; extraction pins its own
    /// deterministic settings).
    pub custom_openai_max_tokens: Option<u32>,
    pub custom_openai_temperature: Option<f32>,
    pub custom_openai_top_p: Option<f32>,
}

/// Resolve the credentials/endpoints for `model_provider` + `model_name` from settings.
///
/// - **CustomOpenAI**: requires a stored config (endpoint + optional key + tuning).
/// - **Ollama / BuiltInAI**: keyless; Ollama additionally reads its custom endpoint from
///   the model config (a read failure degrades to the default endpoint, matching the
///   summary path's long-standing behavior).
/// - **Cloud providers**: require a non-empty API key (Keychain-backed reads that come
///   back empty are treated as not configured — ADR-0009 — never a panic).
pub async fn resolve_provider_config(
    pool: &SqlitePool,
    model_provider: &str,
    model_name: &str,
) -> anyhow::Result<ProviderConfig> {
    let provider = LLMProvider::from_str(model_provider).map_err(|e| anyhow!(e))?;

    let (api_key, custom_openai_endpoint, max_tokens, temperature, top_p) = match provider {
        LLMProvider::CustomOpenAI => {
            let config = SettingsRepository::get_custom_openai_config(pool)
                .await
                .context("Failed to retrieve custom OpenAI config")?
                .ok_or_else(|| {
                    anyhow!("Custom OpenAI provider selected but no configuration found")
                })?;
            (
                config.api_key.unwrap_or_default(),
                Some(config.endpoint),
                config.max_tokens.map(|t| t as u32),
                config.temperature,
                config.top_p,
            )
        }
        LLMProvider::Ollama | LLMProvider::BuiltInAI => (String::new(), None, None, None, None),
        _ => {
            let key = SettingsRepository::get_api_key(pool, model_provider)
                .await
                .with_context(|| format!("Failed to retrieve API key for {model_provider}"))?
                .filter(|k| !k.is_empty())
                .ok_or_else(|| {
                    anyhow!(
                        "No API key configured for {model_provider} — add one in Settings → Summary"
                    )
                })?;
            (key, None, None, None, None)
        }
    };

    let ollama_endpoint = if provider == LLMProvider::Ollama {
        match SettingsRepository::get_model_config(pool).await {
            Ok(Some(config)) => config.ollama_endpoint,
            Ok(None) => None,
            Err(e) => {
                info!("Failed to retrieve Ollama endpoint: {}, using default", e);
                None
            }
        }
    } else {
        None
    };

    Ok(ProviderConfig {
        provider,
        model_provider: model_provider.to_string(),
        model_name: model_name.to_string(),
        api_key,
        ollama_endpoint,
        custom_openai_endpoint,
        custom_openai_max_tokens: max_tokens,
        custom_openai_temperature: temperature,
        custom_openai_top_p: top_p,
    })
}
