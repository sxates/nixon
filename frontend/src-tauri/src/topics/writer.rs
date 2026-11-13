//! The entry writer's LLM-call half (specs/0089): one deterministic, schema-constrained call
//! with a single retry on an unparseable reply. Prompts and parsing live in `prompts.rs`;
//! persistence is the job's concern. Replies derive from the transcript, so nothing here
//! logs them.

use std::future::Future;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

use super::evidence::Evidence;
use super::ledger::OpenDecision;
use super::model::Topic;
use super::prompts::{entry_prompt, entry_schema, parse_entry, ParsedEntry};
use crate::summary::provider_config::ProviderConfig;
use crate::summary::summary_engine::options::GenerationOptions;

const RETRY_REMINDER: &str = "\n\nIMPORTANT: Your previous reply could not be parsed. Return ONLY the JSON object, starting with { and ending with }, with the keys \"bullets\", \"decisions\" and \"revises\" and no other text.";

/// Deterministic JSON generation plus the entry schema.
pub fn entry_options() -> GenerationOptions {
    GenerationOptions {
        json_schema: Some(entry_schema()),
        ..GenerationOptions::deterministic_json()
    }
}

/// Drafts one meeting's entry through `llm(system, user)`; retries once if the reply doesn't
/// parse. A transport error is final.
pub async fn write_entry_with_llm<F, Fut>(
    llm: F,
    topic: &Topic,
    summary: &str,
    notes: Option<&str>,
    evidence: &Evidence,
    decisions: &[OpenDecision],
) -> Result<ParsedEntry>
where
    F: Fn(String, String) -> Fut,
    Fut: Future<Output = std::result::Result<String, String>>,
{
    let (system, user) = entry_prompt(topic, summary, notes, evidence, decisions);
    let first = llm(system.clone(), user.clone())
        .await
        .map_err(|e| anyhow!("Topic entry call failed: {e}"))?;
    match parse_entry(&first, evidence, decisions) {
        Ok(entry) => Ok(entry),
        Err(err) => {
            log::warn!("Topic entry reply was unparseable; retrying once: {err:#}");
            let second = llm(system, format!("{user}{RETRY_REMINDER}"))
                .await
                .map_err(|e| anyhow!("Topic entry retry call failed: {e}"))?;
            parse_entry(&second, evidence, decisions)
                .context("The model returned an unreadable topic entry twice")
        }
    }
}

/// [`write_entry_with_llm`] over the summary's provider config.
#[allow(clippy::too_many_arguments)] // the call's full input surface; callers pass it once
pub async fn write_entry(
    client: &reqwest::Client,
    config: &ProviderConfig,
    app_data_dir: Option<&PathBuf>,
    topic: &Topic,
    summary: &str,
    notes: Option<&str>,
    evidence: &Evidence,
    decisions: &[OpenDecision],
) -> Result<ParsedEntry> {
    let options = entry_options();
    let llm = |system: String, user: String| {
        let options = &options;
        async move {
            crate::summary::llm_client::generate_summary_with_options(
                client,
                &config.provider,
                &config.model_name,
                &config.api_key,
                &system,
                &user,
                config.ollama_endpoint.as_deref(),
                config.custom_openai_endpoint.as_deref(),
                None,      // max_tokens: provider default (a low cap truncates JSON)
                Some(0.0), // temperature: deterministic
                None,
                app_data_dir,
                None,
                options,
            )
            .await
        }
    };
    write_entry_with_llm(llm, topic, summary, notes, evidence, decisions).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn topic() -> Topic {
        Topic {
            id: "topic-1".into(),
            name: "Atlas CRM rebuild".into(),
            description: String::new(),
            status: "open".into(),
            notes_markdown: None,
            notes_json: None,
            created_at: String::new(),
            updated_at: String::new(),
            closed_at: None,
        }
    }

    #[tokio::test]
    async fn writer_retries_once_on_bad_json() {
        let calls = AtomicUsize::new(0);
        let llm = |_s: String, _u: String| {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Ok("sorry, no json".to_string())
                } else {
                    Ok(r#"{"bullets":[{"text":"Scoped the import","t":null}],"decisions":[],"revises":[]}"#.to_string())
                }
            }
        };
        let entry = write_entry_with_llm(llm, &topic(), "s", None, &Evidence::default(), &[])
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(entry.body.bullets.len(), 1);

        // Two bad replies: an error after exactly two calls.
        let calls = AtomicUsize::new(0);
        let llm = |_s: String, _u: String| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok("nope".to_string()) }
        };
        assert!(
            write_entry_with_llm(llm, &topic(), "s", None, &Evidence::default(), &[])
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
