use crate::summary::llm_gate;
use crate::summary::summary_engine::options::GenerationOptions;
use reqwest::{header, Client};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::info;

// specs/0056 W1: the wire format (constants, request builder, structs, reply validation)
// lives in `llm_wire.rs`; re-exported so every existing `llm_client::…` path still resolves.
pub(crate) use crate::summary::llm_wire::{
    build_chat_request_body, non_empty_reply, provider_name, wants_json_mode,
};
pub use crate::summary::llm_wire::{
    ChatMessage, ChatRequest, ChatResponse, Choice, ClaudeChatContent, ClaudeChatResponse,
    ClaudeRequest, MessageContent, CHUNK_MAP_MAX_TOKENS, DEFAULT_CLAUDE_MAX_TOKENS,
    DEFAULT_OLLAMA_MAX_TOKENS, DEFAULT_SUMMARY_TEMPERATURE, OLLAMA_REASONING_EFFORT,
    PREP_BRIEF_MAX_TOKENS,
};

const REQUEST_TIMEOUT_DURATION: Duration = Duration::from_secs(300);

/// LLM Provider enumeration for multi-provider support
#[derive(Debug, Clone, PartialEq)]
pub enum LLMProvider {
    OpenAI,
    Claude,
    Groq,
    Ollama,
    OpenRouter,
    BuiltInAI,
    CustomOpenAI,
}

impl LLMProvider {
    /// Parse provider from string (case-insensitive)
    #[allow(clippy::should_implement_trait)] // inherent parser; not the std FromStr trait
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "claude" => Ok(Self::Claude),
            "groq" => Ok(Self::Groq),
            "ollama" => Ok(Self::Ollama),
            "openrouter" => Ok(Self::OpenRouter),
            "builtin-ai" | "local-llama" | "localllama" => Ok(Self::BuiltInAI),
            "custom-openai" => Ok(Self::CustomOpenAI),
            _ => Err(format!("Unsupported LLM provider: {}", s)),
        }
    }
}

/// `generate_summary` with explicit per-call-site generation options
/// (specs/0053 W1). Prefer this for structured output; `generate_summary`
/// delegates here with defaults.
///
/// # Arguments
/// * `client` - Reqwest HTTP client (reused for performance)
/// * `provider` - The LLM provider to use
/// * `model_name` - The specific model to use (e.g., "gpt-4", "claude-3-opus")
/// * `api_key` - API key for the provider (not needed for Ollama)
/// * `system_prompt` - System instructions for the LLM
/// * `user_prompt` - User query/content to process
/// * `ollama_endpoint` - Optional custom Ollama endpoint (defaults to localhost:11434)
/// * `custom_openai_endpoint` - Optional custom OpenAI-compatible endpoint
/// * `max_tokens` - Optional max output tokens, forwarded for ALL providers. When
///   `None`, OpenAI-compatible providers use their default; Claude (which requires it)
///   falls back to [`DEFAULT_CLAUDE_MAX_TOKENS`].
/// * `temperature` - Optional temperature, forwarded for ALL providers. When `None`,
///   defaults to the low [`DEFAULT_SUMMARY_TEMPERATURE`] for determinism.
/// * `top_p` - Optional top_p, forwarded for ALL providers when set.
/// * `app_data_dir` - Optional app data directory (for BuiltInAI provider)
/// * `cancellation_token` - Optional token to cancel the request
/// * `options` - Generation overrides (grammar/temperature/max_tokens/penalties) for
///   the BuiltInAI sidecar, and JSON-mode intent for hosted providers.
///
/// # Returns
/// The generated summary text or an error message
#[allow(clippy::too_many_arguments)] // mirrors generate_summary's full input surface
pub async fn generate_summary_with_options(
    client: &Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: Option<&CancellationToken>,
    options: &GenerationOptions,
) -> Result<String, String> {
    // Check if cancelled before starting
    if let Some(token) = cancellation_token {
        if token.is_cancelled() {
            return Err("Summary generation was cancelled".to_string());
        }
    }

    // Handle BuiltInAI provider separately (uses local sidecar, no HTTP API)
    if provider == &LLMProvider::BuiltInAI {
        let app_data_dir = app_data_dir
            .ok_or_else(|| "app_data_dir is required for BuiltInAI provider".to_string())?;

        // specs/0053 W1: previously this returned before max_tokens/temperature
        // were read at all. Merge the positional arguments into the options so
        // the caller's intent reaches the sidecar.
        let mut effective = options.clone();
        if effective.temperature.is_none() {
            effective.temperature = temperature;
        }
        if effective.max_tokens.is_none() {
            effective.max_tokens = max_tokens.map(|m| m as i32);
        }

        // specs/0056 W2: priority-wait behind interactive work (no preemption — the sidecar
        // cannot cancel a request short of killing the process).
        return llm_gate::run_gated(
            provider,
            cancellation_token,
            || "Summary generation was cancelled".to_string(),
            || async {
                crate::summary::summary_engine::generate_with_builtin(
                    app_data_dir,
                    model_name,
                    system_prompt,
                    user_prompt,
                    cancellation_token,
                    &effective,
                )
                .await
                .map_err(|e| e.to_string())
            },
        )
        .await;
    }

    let (api_url, mut headers) = match provider {
        LLMProvider::OpenAI => (
            "https://api.openai.com/v1/chat/completions".to_string(),
            header::HeaderMap::new(),
        ),
        LLMProvider::Groq => (
            "https://api.groq.com/openai/v1/chat/completions".to_string(),
            header::HeaderMap::new(),
        ),
        LLMProvider::OpenRouter => (
            "https://openrouter.ai/api/v1/chat/completions".to_string(),
            header::HeaderMap::new(),
        ),
        LLMProvider::Ollama => {
            let host = ollama_endpoint
                .map(|s| s.to_string())
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            (
                format!("{}/v1/chat/completions", host),
                header::HeaderMap::new(),
            )
        }
        LLMProvider::CustomOpenAI => {
            let endpoint = custom_openai_endpoint
                .ok_or_else(|| "Custom OpenAI endpoint not configured".to_string())?;
            (
                format!("{}/chat/completions", endpoint.trim_end_matches('/')),
                header::HeaderMap::new(),
            )
        }
        LLMProvider::Claude => {
            let mut header_map = header::HeaderMap::new();
            header_map.insert(
                "x-api-key",
                api_key
                    .parse()
                    .map_err(|_| "Invalid API key format".to_string())?,
            );
            header_map.insert(
                "anthropic-version",
                "2023-06-01"
                    .parse()
                    .map_err(|_| "Invalid anthropic version".to_string())?,
            );
            (
                "https://api.anthropic.com/v1/messages".to_string(),
                header_map,
            )
        }
        LLMProvider::BuiltInAI => {
            // This case is handled earlier with early returns
            unreachable!("BuiltInAI is handled before this match statement")
        }
    };

    // Add authorization header for non-Claude providers
    if provider != &LLMProvider::Claude {
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", api_key)
                .parse()
                .map_err(|_| "Invalid authorization header".to_string())?,
        );
    }
    headers.insert(
        header::CONTENT_TYPE,
        "application/json"
            .parse()
            .map_err(|_| "Invalid content type".to_string())?,
    );

    let request_body = build_chat_request_body(
        provider,
        model_name,
        system_prompt,
        user_prompt,
        max_tokens,
        temperature,
        top_p,
        wants_json_mode(options),
    );

    info!(
        "🐞 LLM Request to {}: model={}",
        provider_name(provider),
        model_name
    );

    // specs/0056 W2: schedule through the gate. On a local model a background call waits
    // for (and can be preempted by) interactive work; the closure rebuilds the request so a
    // preempted call is re-issued from scratch. Cloud providers pass straight through.
    let endpoint = ollama_endpoint.map(str::to_string);
    llm_gate::run_gated(
        provider,
        cancellation_token,
        || "Summary generation was cancelled".to_string(),
        || {
            send_and_parse(
                client,
                api_url.clone(),
                headers.clone(),
                request_body.clone(),
                provider,
                model_name,
                endpoint.as_deref(),
                cancellation_token,
            )
        },
    )
    .await
}

/// One HTTP exchange: send the built request (with the client timeout and cancellation
/// race), then parse the provider's reply into the trimmed, non-empty content.
#[allow(clippy::too_many_arguments)] // the exchange's full input surface
async fn send_and_parse(
    client: &Client,
    api_url: String,
    headers: header::HeaderMap,
    request_body: serde_json::Value,
    provider: &LLMProvider,
    model_name: &str,
    ollama_endpoint: Option<&str>,
    cancellation_token: Option<&CancellationToken>,
) -> Result<String, String> {
    // Send request with timeout and cancellation support
    let request_future = client
        .post(api_url)
        .headers(headers)
        .json(&request_body)
        .timeout(REQUEST_TIMEOUT_DURATION)
        .send();

    // Use tokio::select to race between cancellation and request completion
    let response = if let Some(token) = cancellation_token {
        tokio::select! {
            result = request_future => {
                result.map_err(|e| {
                    if e.is_timeout() {
                        "LLM request timed out after 300 seconds".to_string()
                    } else {
                        format!("Failed to send request to LLM: {}", e)
                    }
                })?
            }
            _ = token.cancelled() => {
                return Err("Summary generation was cancelled".to_string());
            }
        }
    } else {
        request_future.await.map_err(|e| {
            if e.is_timeout() {
                "LLM request timed out after 300 seconds".to_string()
            } else {
                format!("Failed to send request to LLM: {}", e)
            }
        })?
    };

    if !response.status().is_success() {
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("LLM API request failed: {}", error_body));
    }

    // Parse response based on provider
    if provider == &LLMProvider::Claude {
        let chat_response = response
            .json::<ClaudeChatResponse>()
            .await
            .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

        info!("🐞 LLM Response received from Claude");

        let content = chat_response
            .content
            .first()
            .ok_or("No content in LLM response")?
            .text
            .as_str();
        non_empty_reply(Some(content), provider)
    } else {
        let chat_response = response
            .json::<ChatResponse>()
            .await
            .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

        info!("🐞 LLM Response received from {}", provider_name(provider));

        // specs/0052: the model is guaranteed loaded right now, so this is the one moment
        // `/api/ps` reliably reports the served context. Refresh opportunistically here
        // rather than forcing a cold load (~11s) just to probe.
        if provider == &LLMProvider::Ollama {
            crate::ollama::served_context::refresh_served_context(model_name, ollama_endpoint)
                .await;
        }

        let content = chat_response
            .choices
            .first()
            .ok_or("No content in LLM response")?
            .message
            .content
            .as_deref();
        non_empty_reply(content, provider)
    }
}

/// Unchanged signature for every existing caller; no grammar, no JSON mode.
#[allow(clippy::too_many_arguments)] // cohesive param set; refactor deferred
pub async fn generate_summary(
    client: &Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: Option<&CancellationToken>,
) -> Result<String, String> {
    generate_summary_with_options(
        client,
        provider,
        model_name,
        api_key,
        system_prompt,
        user_prompt,
        ollama_endpoint,
        custom_openai_endpoint,
        max_tokens,
        temperature,
        top_p,
        app_data_dir,
        cancellation_token,
        &GenerationOptions::default(),
    )
    .await
}
