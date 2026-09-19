use crate::database::repositories::{
    meeting::MeetingsRepository, summary::SummaryProcessesRepository,
};
use crate::ollama::metadata::ModelMetadataCache;
use crate::summary::cache_key::SummaryCacheSource;
use crate::summary::language_detection::detect_summary_language;
use crate::summary::llm_client::LLMProvider;
use crate::summary::metadata::read_detected_summary_language_from_metadata;
use crate::summary::processor::language_name_from_code;
use crate::summary::templates;
use crate::summary::templates::Template;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

// Global cache for model metadata (5 minute TTL)
static METADATA_CACHE: Lazy<ModelMetadataCache> =
    Lazy::new(|| ModelMetadataCache::new(Duration::from_secs(300)));

// Global registry for cancellation tokens (thread-safe)
static CANCELLATION_REGISTRY: Lazy<Arc<Mutex<HashMap<String, CancellationToken>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Conservative context-window token budget for cloud providers, used only to decide
/// single-pass vs. chunking (the map/reduce path). These are deliberately conservative:
/// the previous single `100000` "effectively unlimited" value forced single-pass for
/// every cloud provider, so a long meeting on a small-context model (many Groq /
/// OpenRouter / custom OpenAI-compatible endpoints are 8k–32k) silently overflowed and
/// got truncated. OpenAI/Claude keep their genuinely-large windows; the rest fall back
/// to a safe 32k so long meetings chunk instead of truncating.
///
/// Note: `rough_token_count` intentionally over-estimates, so the effective trigger point
/// is even more conservative than these raw numbers.
fn cloud_context_threshold(provider: &LLMProvider) -> usize {
    match provider {
        LLMProvider::Claude => 180_000,    // Claude 3.x family: ~200k context
        LLMProvider::OpenAI => 120_000,    // GPT-4o / 4.1: 128k context
        LLMProvider::Groq => 32_000,       // hosted models vary 8k–128k; stay conservative
        LLMProvider::OpenRouter => 32_000, // routes to many models; conservative
        LLMProvider::CustomOpenAI => 32_000, // unknown self-hosted server; conservative
        // Local providers are sized from model metadata before this is reached.
        LLMProvider::Ollama | LLMProvider::BuiltInAI => 8_000,
    }
}

/// Resolves the usable context-token budget for a provider/model pair.
///
/// - **Ollama:** live model metadata via the shared `METADATA_CACHE` (5-minute TTL),
///   minus a 300-token prompt-overhead reserve; falls back to a safe 4000 when the
///   metadata fetch fails.
/// - **BuiltInAI:** the bundled model registry's context size, minus the same reserve;
///   falls back to 1748 (2048 − 300) for an unknown model.
/// - **Cloud providers:** the conservative per-provider [`cloud_context_threshold`].
///
/// Shared by the summary service (single-pass vs. chunking decision in
/// [`SummaryService::process_transcript_background`]) and the aggregation engine
/// (specs/0035 budget packing). The summary path's behavior must stay identical —
/// this is a verbatim extraction of the sizing logic that used to be inlined there.
pub async fn resolve_context_budget(
    provider: &LLMProvider,
    model_name: &str,
    ollama_endpoint: Option<&str>,
) -> usize {
    if *provider == LLMProvider::Ollama {
        match METADATA_CACHE
            .get_or_fetch(model_name, ollama_endpoint)
            .await
        {
            Ok(metadata) => {
                // specs/0052: clamp to what the server SERVES — see `context_budget`.
                crate::summary::context_budget::ollama_context_budget(
                    metadata.context_size,
                    model_name,
                    ollama_endpoint,
                )
                .await
            }
            Err(e) => {
                warn!(
                    "Failed to fetch context for {}: {}. Using default 4000",
                    model_name, e
                );
                4000 // Fallback to safe default
            }
        }
    } else if *provider == LLMProvider::BuiltInAI {
        // Get model's context size from registry
        use crate::summary::summary_engine::models;
        let model = models::get_model_by_name(model_name)
            .ok_or_else(|| format!("Unknown model: {}", model_name));

        match model {
            Ok(model_def) => {
                // Reserve 300 tokens for prompt overhead
                let optimal = model_def.context_size.saturating_sub(300) as usize;
                info!(
                    "✓ Using BuiltInAI context size: {} tokens (chunk size: {})",
                    model_def.context_size, optimal
                );
                optimal
            }
            Err(e) => {
                warn!("{}, using default 2048", e);
                1748 // 2048 - 300 for overhead
            }
        }
    } else {
        // Cloud providers get a REAL/conservative context budget per model class
        // instead of a bogus "100000 = effectively unlimited". Small-context cloud
        // models (many Groq / OpenRouter / custom OpenAI-compatible endpoints are
        // 8k–32k) would otherwise silently truncate a long meeting; now anything over
        // the budget takes the chunking path.
        let threshold = cloud_context_threshold(provider);
        info!(
            "Using conservative cloud context threshold for {:?}: {} tokens",
            provider, threshold
        );
        threshold
    }
}

const ENGLISH_CACHE_FIELD: &str = "english_cache";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EnglishSummaryCache {
    markdown: String,
    source: SummaryCacheSource,
    output_language: Option<String>,
}

fn normalise_summary_language_for_cache(summary_language: Option<&str>) -> Option<String> {
    language_name_from_code(summary_language?.trim()).map(str::to_string)
}

/// `display_markdown` must already be title-stripped (`strip_title_if_present`) — the
/// caller strips ONCE and reuses the same string as the action-item extraction input, so
/// the extraction ledger fingerprint always matches what `api_extract_action_items`
/// later reads back from `$.markdown` (cross-path idempotence, specs/0034 acceptance #4).
pub(super) fn build_summary_result_json(
    display_markdown: &str,
    english_markdown: &str,
    source: SummaryCacheSource,
    output_language: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "markdown": display_markdown,
        ENGLISH_CACHE_FIELD: EnglishSummaryCache {
            markdown: english_markdown.to_string(),
            source,
            output_language: normalise_summary_language_for_cache(output_language),
        },
    })
}

/// Parses a `summary_processes.result` JSON blob and extracts a cached English
/// summary only when it was produced from exactly the same source inputs and
/// the user is switching to a different non-English target language.
pub(super) fn extract_cached_english_markdown(
    raw: &str,
    expected_source: &SummaryCacheSource,
    requested_language: Option<&str>,
) -> Result<Option<String>, serde_json::Error> {
    let requested_language = match normalise_summary_language_for_cache(requested_language) {
        Some(language) if language != "English" => language,
        _ => return Ok(None),
    };

    let value: serde_json::Value = serde_json::from_str(raw)?;
    let Some(cache_value) = value.get(ENGLISH_CACHE_FIELD) else {
        return Ok(None);
    };

    let cache: EnglishSummaryCache = match serde_json::from_value(cache_value.clone()) {
        Ok(cache) => cache,
        Err(_) => return Ok(None),
    };

    if cache.source != *expected_source {
        return Ok(None);
    }

    if cache.output_language.as_deref() == Some(requested_language.as_str()) {
        return Ok(None);
    }

    let markdown = cache.markdown.trim();
    if markdown.is_empty() {
        Ok(None)
    } else {
        Ok(Some(cache.markdown))
    }
}

/// Summary service - handles all summary generation logic
pub struct SummaryService;

impl SummaryService {
    /// Registers a new cancellation token for a meeting
    pub(super) fn register_cancellation_token(meeting_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        if let Ok(mut registry) = CANCELLATION_REGISTRY.lock() {
            registry.insert(meeting_id.to_string(), token.clone());
            info!("Registered cancellation token for meeting: {}", meeting_id);
        }
        token
    }

    /// Cancels the summary generation for a meeting
    pub fn cancel_summary(meeting_id: &str) -> bool {
        if let Ok(registry) = CANCELLATION_REGISTRY.lock() {
            if let Some(token) = registry.get(meeting_id) {
                info!("Cancelling summary generation for meeting: {}", meeting_id);
                token.cancel();
                return true;
            }
        }
        warn!(
            "No active summary generation found for meeting: {}",
            meeting_id
        );
        false
    }

    /// Cleans up the cancellation token after processing completes
    pub(super) fn cleanup_cancellation_token(meeting_id: &str) {
        if let Ok(mut registry) = CANCELLATION_REGISTRY.lock() {
            if registry.remove(meeting_id).is_some() {
                info!("Cleaned up cancellation token for meeting: {}", meeting_id);
            }
        }
    }

    pub(super) async fn read_detected_summary_language(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Option<String> {
        let meeting = match MeetingsRepository::get_meeting_metadata(pool, meeting_id).await {
            Ok(Some(meeting)) => meeting,
            Ok(None) => {
                warn!(
                    "Meeting not found while reading detected summary language: {}",
                    meeting_id
                );
                return None;
            }
            Err(e) => {
                warn!(
                    "Failed to read meeting metadata for detected summary language (meeting_id={}): {}",
                    meeting_id, e
                );
                return None;
            }
        };

        let folder_path = meeting.folder_path.filter(|p| !p.trim().is_empty())?;

        match read_detected_summary_language_from_metadata(Path::new(&folder_path)) {
            Ok(language) => language,
            Err(e) => {
                warn!(
                    "Failed to read detected summary language metadata for meeting_id={}: {}",
                    meeting_id, e
                );
                None
            }
        }
    }

    pub(super) fn detect_summary_language_from_text(text: &str) -> Option<String> {
        let transcript_texts = [text.to_string()];
        let detection = detect_summary_language(&transcript_texts);
        match &detection.language {
            Some(language) => {
                info!(
                    "Detected transcript summary language for normalization: {}",
                    language
                );
            }
            None => {
                info!(
                    "Transcript summary language unknown for normalization: {:?}",
                    detection.reason
                );
            }
        }
        detection.language
    }

    /// Resolves a **fixed** (non-Auto) template id for summary generation —
    /// the `(None, id)` arm of [`SummaryService::process_transcript_background`]'s
    /// template match. Falls back to the default fixed template
    /// ([`templates::DEFAULT_TEMPLATE_ID`]) when `id` no longer resolves
    /// (specs/0061 W6): a built-in removed in an app update (e.g. the retired
    /// Psychiatric Session template) or a deleted custom override must not
    /// fail generation for a meeting that persisted that choice — it should
    /// degrade the same way a NULL `template_id` already does, not end the
    /// run in `update_process_failed`.
    ///
    /// The returned [`Template`]'s own content, not `id`, is what downstream
    /// fingerprinting ([`template_cache_fingerprint`]) and generation see, so
    /// a substituted template is cached and generated under a fingerprint
    /// that matches what was actually produced — never a mismatched one.
    ///
    /// Returns `Err` rather than panicking (specs/0061 review, I4) when even the
    /// DEFAULT template fails to resolve — reachable, not theoretical: `get_template`
    /// prefers a custom-directory override, so a corrupt or invalid user override of
    /// `standard_meeting` hits exactly this path. The caller must route `Err` through
    /// `update_process_failed`, same as before this fallback existed, rather than
    /// leaving the meeting stuck in "processing" forever behind a panicked task.
    pub fn resolve_fixed_template(meeting_id: &str, id: &str) -> Result<Template, String> {
        match templates::get_template(id) {
            Ok(template) => Ok(template),
            Err(e) => {
                warn!(
                    "Meeting {}: template '{}' no longer resolves ({}); falling back to the default template '{}'",
                    meeting_id, id, e, templates::DEFAULT_TEMPLATE_ID
                );
                templates::get_template(templates::DEFAULT_TEMPLATE_ID).map_err(|default_err| {
                    format!(
                        "template '{}' failed to resolve ({}), and the default template '{}' also failed to resolve ({})",
                        id, e, templates::DEFAULT_TEMPLATE_ID, default_err
                    )
                })
            }
        }
    }

    /// Updates the summary process status to failed with error message
    ///
    /// # Arguments
    /// * `pool` - SQLx connection pool
    /// * `meeting_id` - Meeting identifier
    /// * `error_msg` - Error message to store
    pub(super) async fn update_process_failed(
        pool: &SqlitePool,
        meeting_id: &str,
        error_msg: &str,
    ) {
        error!(
            "Processing failed for meeting_id {}: {}",
            meeting_id, error_msg
        );
        if let Err(e) =
            SummaryProcessesRepository::update_process_failed(pool, meeting_id, error_msg).await
        {
            error!(
                "Failed to update DB status to failed for {}: {}",
                meeting_id, e
            );
        }
    }
}

#[cfg(test)]
mod tests;
