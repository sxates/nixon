//! The mention detector's LLM-call half (specs/0089): one deterministic, schema-constrained
//! call asking which open topics a meeting discussed, with a single retry on an unparseable
//! reply. Replies derive from the transcript, so nothing here logs them.

use std::future::Future;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

use super::evidence::Evidence;
use super::model::Topic;
use super::prompts::{mention_prompt, mention_schema, parse_mentions};
use crate::summary::provider_config::ProviderConfig;
use crate::summary::summary_engine::options::GenerationOptions;

const RETRY_REMINDER: &str = "\n\nIMPORTANT: Your previous reply could not be parsed. Return ONLY the JSON object, starting with { and ending with }, whose single key \"mentions\" holds the array, with no other text.";

/// Deterministic JSON generation plus the mention schema.
pub fn mention_options() -> GenerationOptions {
    GenerationOptions {
        json_schema: Some(mention_schema()),
        ..GenerationOptions::deterministic_json()
    }
}

/// Returns (topic, one-sentence summary, cited time). No topics, or no evidence to show,
/// means no call at all.
pub async fn detect_with_llm<F, Fut>(
    llm: F,
    topics: &[Topic],
    summary: &str,
    evidence: &Evidence,
) -> Result<Vec<(Topic, String, f64)>>
where
    F: Fn(String, String) -> Fut,
    Fut: Future<Output = std::result::Result<String, String>>,
{
    if topics.is_empty() || evidence.text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let (system, user) = mention_prompt(topics, summary, evidence);
    let first = llm(system.clone(), user.clone())
        .await
        .map_err(|e| anyhow!("Topic mention call failed: {e}"))?;
    match parse_mentions(&first, evidence, topics) {
        Ok(found) => Ok(found),
        Err(err) => {
            log::warn!("Topic mention reply was unparseable; retrying once: {err:#}");
            let second = llm(system, format!("{user}{RETRY_REMINDER}"))
                .await
                .map_err(|e| anyhow!("Topic mention retry call failed: {e}"))?;
            parse_mentions(&second, evidence, topics)
                .context("The model returned unreadable topic mentions twice")
        }
    }
}

/// [`detect_with_llm`] over the summary's provider config.
pub async fn detect(
    client: &reqwest::Client,
    config: &ProviderConfig,
    app_data_dir: Option<&PathBuf>,
    topics: &[Topic],
    summary: &str,
    evidence: &Evidence,
) -> Result<Vec<(Topic, String, f64)>> {
    let options = mention_options();
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
                None,
                Some(0.0),
                None,
                app_data_dir,
                None,
                options,
            )
            .await
        }
    };
    detect_with_llm(llm, topics, summary, evidence).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detector_returns_empty_for_no_topics_without_calling_llm() {
        let llm = |_s: String, _u: String| async {
            panic!("the model must not be called without topics");
            #[allow(unreachable_code)]
            Ok::<String, String>(String::new())
        };
        let evidence = Evidence::default();
        let out = detect_with_llm(llm, &[], "summary", &evidence)
            .await
            .unwrap();
        assert!(out.is_empty());
    }
}
