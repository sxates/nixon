//! Provider wire format for the summary LLM client (specs/0056 W1 split out of
//! `llm_client.rs`, which the file-size ratchet caps at 800 lines).
//!
//! Everything here is pure data + policy with no I/O: the output-cap / temperature /
//! reasoning defaults, the request-body builder every OpenAI-compatible provider and
//! Claude go through, the request/response structs, and the reply validation that turns
//! an empty `content` into an error. `llm_client` re-exports the public items so existing
//! `llm_client::…` paths keep working.

use crate::summary::llm_client::LLMProvider;
use crate::summary::summary_engine::options::GenerationOptions;
use serde::{Deserialize, Serialize};

/// Low default temperature applied to EVERY summarization/translation/normalization
/// pass (all providers) when the caller does not specify one. Summaries must be
/// faithful and reproducible, not creative — a low temperature restores the
/// anti-hallucination/determinism intent that was previously only honored for the
/// CustomOpenAI provider. Callers (e.g. a user-configured CustomOpenAI temperature)
/// can still override it.
pub const DEFAULT_SUMMARY_TEMPERATURE: f32 = 0.15;

/// Fallback max output tokens for the Claude Messages API (which *requires*
/// `max_tokens`) when the caller does not size it from the template/expected
/// output. Raised from the previous hardcoded 2048 — which truncated long reports
/// and translations — to a realistic cap. Modern Claude models support far more,
/// but 8192 comfortably fits a full meeting report without risking an over-large
/// allocation.
pub const DEFAULT_CLAUDE_MAX_TOKENS: u32 = 8192;

/// Fallback output cap for Ollama (specs/0052). `provider_config.rs` hardcodes `None` for
/// Ollama's max_tokens and `ChatRequest` skips the field when `None`, so without this every
/// Ollama request was UNBOUNDED and ran until `REQUEST_TIMEOUT_DURATION` (300s) killed it —
/// a 1,619-token prompt was observed generating 17k+ degenerate tokens.
///
/// 4096 at ~60 tok/s is ~70s: generous enough not to truncate a real report, tight enough
/// that a degenerate loop is a blip rather than a five-minute GPU burn.
pub const DEFAULT_OLLAMA_MAX_TOKENS: u32 = 4096;

/// Prep briefs synthesize 2 prior summaries; p90 of healthy runs was ~1,800 tokens.
pub const PREP_BRIEF_MAX_TOKENS: u32 = 2000;

/// Per-chunk map summaries are short by construction.
pub const CHUNK_MAP_MAX_TOKENS: u32 = 1500;

/// specs/0056 W1: Ollama's OpenAI-compatible endpoint runs "thinking" models (gemma4, qwen3,
/// gpt-oss, …) with reasoning ON by default. The reasoning comes back in a separate
/// `message.reasoning` field this client never reads, and every reasoning token counts
/// against `max_tokens`. Measured on the owner's `gemma4:26b` with the real person-roll-up
/// prompt: 4,096 tokens of reasoning, `finish_reason: length`, `content` EMPTY (59 s) — the
/// "Nothing notable to summarize" screen. With this field: 156 tokens, a correct answer, 3.4 s.
///
/// Every Nixon prompt is extraction, summarization or grounded QA with a tight output
/// budget, so reasoning is switched off for ALL Ollama requests. `think: false` is ignored by
/// the `/v1/chat/completions` layer; `reasoning_effort` is honoured, and non-thinking models
/// (gemma3 verified) accept it without error. Not sent to other providers: CustomOpenAI may
/// point at a server that rejects unknown fields, and the cloud providers size their own
/// reasoning budgets.
pub const OLLAMA_REASONING_EFFORT: &str = "none";

/// Build the provider request body. Split out of [`generate_summary`] so the token-cap and
/// temperature policy is unit-testable without a live endpoint.
#[allow(clippy::too_many_arguments)] // cohesive param set mirroring generate_summary's inputs
pub(crate) fn build_chat_request_body(
    provider: &LLMProvider,
    model_name: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    json_mode: bool,
) -> serde_json::Value {
    // A low temperature is applied to EVERY provider's summarization pass (not just
    // CustomOpenAI) unless the caller explicitly set one — restores the determinism /
    // anti-hallucination intent for OpenAI/Groq/Ollama/OpenRouter/Claude too.
    let effective_temperature = Some(temperature.unwrap_or(DEFAULT_SUMMARY_TEMPERATURE));

    // Ollama has no settings path for max_tokens, so apply the default here rather than
    // leaving the field absent — which means "unbounded".
    let effective_max_tokens = match provider {
        LLMProvider::Ollama => Some(max_tokens.unwrap_or(DEFAULT_OLLAMA_MAX_TOKENS)),
        _ => max_tokens,
    };

    if provider != &LLMProvider::Claude {
        // Forward max_tokens / temperature / top_p for ALL OpenAI-compatible providers.
        // `top_p` stays None (provider default) unless the caller supplied it; temperature
        // falls back to the low default above.
        serde_json::json!(ChatRequest {
            model: model_name.to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user_prompt.to_string(),
                }
            ],
            max_tokens: effective_max_tokens,
            temperature: effective_temperature,
            top_p,
            response_format: json_mode.then(|| serde_json::json!({ "type": "json_object" })),
            reasoning_effort: (provider == &LLMProvider::Ollama)
                .then(|| OLLAMA_REASONING_EFFORT.to_string()),
        })
    } else {
        // Claude requires `max_tokens`; size from the caller when provided, else a
        // realistic cap (was a truncating hardcoded 2048).
        serde_json::json!(ClaudeRequest {
            system: system_prompt.to_string(),
            model: model_name.to_string(),
            max_tokens: max_tokens.unwrap_or(DEFAULT_CLAUDE_MAX_TOKENS),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_prompt.to_string(),
            }],
            temperature: effective_temperature,
        })
    }
}

// Generic structure for OpenAI-compatible API chat messages
#[derive(Debug, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

// Generic structure for OpenAI-compatible API chat requests
#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
    /// Ollama only — see [`OLLAMA_REASONING_EFFORT`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

// Generic structure for OpenAI-compatible API chat responses
#[derive(Deserialize, Debug)]
pub struct ChatResponse {
    pub choices: Vec<Choice>,
}

#[derive(Deserialize, Debug)]
pub struct Choice {
    pub message: MessageContent,
}

#[derive(Deserialize, Debug)]
pub struct MessageContent {
    /// `null` when the model produced only reasoning / tool calls (Ollama emits it that way),
    /// which [`non_empty_reply`] turns into the same error as an empty string.
    #[serde(default)]
    pub content: Option<String>,
}

// Claude-specific request structure
#[derive(Debug, Serialize)]
pub struct ClaudeRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    // Claude supports a temperature field too; forward a low one for determinism.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

// Claude-specific response structure
#[derive(Deserialize, Debug)]
pub struct ClaudeChatResponse {
    pub content: Vec<ClaudeChatContent>,
}

#[derive(Deserialize, Debug)]
pub struct ClaudeChatContent {
    pub text: String,
}

/// Whether this call site's options imply JSON mode for providers that support
/// `response_format` (everything except Claude). Explicit `json_mode`, not derived from
/// `grammar.is_some()` — `grammar` cannot be safely set today (see its doc comment on
/// [`GenerationOptions`]; it crashes the BuiltInAI sidecar), and JSON mode on hosted
/// providers must not be coupled to that broken path.
pub(crate) fn wants_json_mode(options: &GenerationOptions) -> bool {
    options.json_mode
}

/// specs/0056 W1: an empty reply is a FAILURE, never a blank success. Before this, an empty
/// `content` flowed to every caller as `Ok("")`: the person roll-up rendered a blank card
/// ("Nothing notable to summarize"), Ask AI rendered a blank answer, and action-item
/// extraction failed with a misleading "no JSON array" parse cause. The message names the
/// usual reason — a thinking model spending its whole output budget on reasoning — so a user
/// on a non-Ollama thinking model (where we cannot switch reasoning off) gets something
/// actionable rather than silence.
pub(crate) fn non_empty_reply(
    content: Option<&str>,
    provider: &LLMProvider,
) -> Result<String, String> {
    match content.map(str::trim).filter(|c| !c.is_empty()) {
        Some(text) => Ok(text.to_string()),
        None => Err(format!(
            "The model ({}) returned an empty reply. If it is a \"thinking\" model, its \
             reasoning probably used up the whole output budget — try a model without \
             extended thinking, or a larger max_tokens.",
            provider_name(provider)
        )),
    }
}

/// Provider name for logging and user-facing errors.
pub(crate) fn provider_name(provider: &LLMProvider) -> &str {
    match provider {
        LLMProvider::OpenAI => "OpenAI",
        LLMProvider::Claude => "Claude",
        LLMProvider::Groq => "Groq",
        LLMProvider::Ollama => "Ollama",
        LLMProvider::BuiltInAI => "Built-in AI",
        LLMProvider::OpenRouter => "OpenRouter",
        LLMProvider::CustomOpenAI => "Custom OpenAI",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::assertions_on_constants)] // intentional compile-time regression guard on a const
    #[test]
    fn claude_default_max_tokens_is_realistic_not_2048() {
        // Regression: the old hardcoded 2048 truncated long reports/translations.
        assert!(DEFAULT_CLAUDE_MAX_TOKENS >= 4096);
    }

    #[allow(clippy::assertions_on_constants)] // intentional compile-time regression guard on a const
    #[test]
    fn default_summary_temperature_is_low() {
        // Determinism / anti-hallucination: summarization must run cold.
        assert!(DEFAULT_SUMMARY_TEMPERATURE <= 0.2);
    }

    #[test]
    fn claude_request_serializes_max_tokens_and_temperature() {
        let req = ClaudeRequest {
            model: "claude-x".to_string(),
            max_tokens: DEFAULT_CLAUDE_MAX_TOKENS,
            system: "sys".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            temperature: Some(DEFAULT_SUMMARY_TEMPERATURE),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["max_tokens"], DEFAULT_CLAUDE_MAX_TOKENS);
        assert!(v.get("temperature").is_some());
    }

    /// The regression that started specs/0052: provider_config.rs hardcodes `None` for
    /// Ollama's max_tokens and ChatRequest skips the field when None, so every Ollama
    /// request shipped with NO output cap and ran to the 300s timeout.
    #[test]
    fn ollama_requests_always_carry_a_max_tokens_cap() {
        let body = build_chat_request_body(
            &LLMProvider::Ollama,
            "gemma4:26b",
            "sys",
            "usr",
            None,
            None,
            None,
            false,
        );
        assert_eq!(
            body["max_tokens"], DEFAULT_OLLAMA_MAX_TOKENS,
            "an uncapped Ollama request can generate until the request times out"
        );
    }

    #[test]
    fn an_explicit_cap_wins_over_the_ollama_default() {
        let body = build_chat_request_body(
            &LLMProvider::Ollama,
            "gemma4:26b",
            "sys",
            "usr",
            Some(2000),
            None,
            None,
            false,
        );
        assert_eq!(body["max_tokens"], 2000);
    }

    /// Non-Ollama OpenAI-compatible providers keep provider-default behavior.
    #[test]
    fn non_ollama_openai_providers_still_omit_max_tokens_when_unset() {
        let body = build_chat_request_body(
            &LLMProvider::OpenAI,
            "gpt-4o",
            "sys",
            "usr",
            None,
            None,
            None,
            false,
        );
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn chat_request_omits_none_optionals_but_keeps_temperature() {
        // Non-Claude providers: max_tokens/top_p omitted when None, temperature present.
        let req = ChatRequest {
            model: "m".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            max_tokens: None,
            temperature: Some(DEFAULT_SUMMARY_TEMPERATURE),
            top_p: None,
            response_format: None,
            reasoning_effort: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert!(v.get("max_tokens").is_none());
        assert!(v.get("top_p").is_none());
        assert!(v.get("reasoning_effort").is_none());
        assert_eq!(v["temperature"], DEFAULT_SUMMARY_TEMPERATURE);
    }

    /// specs/0056 W1: every Ollama request switches reasoning off. This is the field that
    /// turned the owner's empty 59 s roll-up into a correct 3.4 s one (see the spec table).
    #[test]
    fn ollama_requests_switch_reasoning_off() {
        for json_mode in [false, true] {
            let body = build_chat_request_body(
                &LLMProvider::Ollama,
                "gemma4:26b",
                "sys",
                "usr",
                None,
                None,
                None,
                json_mode,
            );
            assert_eq!(body["reasoning_effort"], "none", "json_mode={json_mode}");
        }
    }

    /// Only Ollama gets the field: CustomOpenAI may front a server that rejects unknown
    /// fields, cloud providers size their own reasoning, Claude has a different request shape.
    #[test]
    fn reasoning_effort_never_leaks_to_other_providers() {
        for provider in [
            LLMProvider::OpenAI,
            LLMProvider::Groq,
            LLMProvider::OpenRouter,
            LLMProvider::CustomOpenAI,
            LLMProvider::Claude,
        ] {
            let body =
                build_chat_request_body(&provider, "m", "sys", "usr", None, None, None, false);
            assert!(
                body.get("reasoning_effort").is_none(),
                "{provider:?} must not carry reasoning_effort"
            );
        }
    }

    /// specs/0056 W1: an empty or null `content` is an error naming the provider, not `Ok("")`.
    #[test]
    fn empty_or_null_reply_is_an_error_not_a_blank_success() {
        for content in [None, Some(""), Some("   \n\t")] {
            let err = non_empty_reply(content, &LLMProvider::Ollama)
                .expect_err("blank content must fail");
            assert!(err.contains("empty reply"), "{err}");
            assert!(err.contains("Ollama"), "{err}");
        }
        assert_eq!(
            non_empty_reply(Some("  * Apple\n"), &LLMProvider::Ollama).unwrap(),
            "* Apple"
        );
    }

    /// The wire shape Ollama actually emits for a thinking model: `content: null` plus a
    /// `reasoning` field. Must deserialize (not 400 the run) and then fail as an empty reply.
    #[test]
    fn null_content_with_reasoning_deserializes() {
        let raw =
            r#"{"choices":[{"message":{"role":"assistant","content":null,"reasoning":"..."}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.choices[0].message.content.is_none());
    }

    /// Ollama and every OpenAI-compatible provider support response_format;
    /// asking for JSON must actually put it on the wire.
    #[test]
    fn json_mode_sets_response_format_for_openai_compatible_providers() {
        for provider in [
            LLMProvider::Ollama,
            LLMProvider::OpenAI,
            LLMProvider::Groq,
            LLMProvider::OpenRouter,
            LLMProvider::CustomOpenAI,
        ] {
            let body = build_chat_request_body(
                &provider,
                "some-model",
                "sys",
                "user",
                None,
                Some(0.0),
                None,
                true, // json_mode
            );
            assert_eq!(
                body["response_format"]["type"], "json_object",
                "{provider:?} should carry response_format"
            );
        }
    }

    /// Claude uses a different request struct entirely; a stray response_format
    /// field would be a 400 from the API.
    #[test]
    fn json_mode_never_leaks_response_format_into_a_claude_request() {
        let body = build_chat_request_body(
            &LLMProvider::Claude,
            "claude-model",
            "sys",
            "user",
            None,
            Some(0.0),
            None,
            true, // json_mode requested and must be ignored
        );
        assert!(
            body.get("response_format").is_none(),
            "Claude requests must never carry response_format"
        );
    }

    /// Default path: no json_mode requested => the body is what it was before
    /// specs/0053, for every provider.
    #[test]
    fn without_json_mode_no_response_format_is_emitted() {
        for provider in [
            LLMProvider::Ollama,
            LLMProvider::OpenAI,
            LLMProvider::Claude,
        ] {
            let body = build_chat_request_body(
                &provider,
                "m",
                "sys",
                "user",
                None,
                Some(0.5),
                None,
                false,
            );
            assert!(
                body.get("response_format").is_none(),
                "{provider:?} should not carry response_format when not requested"
            );
        }
    }

    /// `deterministic_json`'s explicit `json_mode` flag drives JSON mode for hosted
    /// providers — independent of `grammar`, which stays unset (it crashes the sidecar;
    /// see `GenerationOptions::grammar`'s doc comment).
    #[test]
    fn deterministic_json_options_imply_json_mode() {
        let opts = GenerationOptions::deterministic_json();
        assert!(wants_json_mode(&opts));
        assert!(opts.grammar.is_none());
        assert!(!wants_json_mode(&GenerationOptions::default()));
    }
}
