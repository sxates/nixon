use crate::database::repositories::meeting_note::MeetingNotesRepository;
use crate::database::repositories::summary_outline::SummaryOutlineRepository;
use crate::database::repositories::{
    meeting::MeetingsRepository, people::PeopleRepository, summary::SummaryProcessesRepository,
};
use crate::ollama::metadata::ModelMetadataCache;
use crate::summary::cache_key::{
    build_summary_cache_source, stable_text_fingerprint, strip_title_if_present,
    template_cache_fingerprint, SummaryCacheSource,
};
use crate::summary::language_detection::detect_summary_language;
use crate::summary::llm_client::LLMProvider;
use crate::summary::metadata::read_detected_summary_language_from_metadata;
use crate::summary::outline::{Outline, TemplateChoice, AUTO_TEMPLATE_ID};
use crate::summary::processor::{
    build_role_preamble, extract_meeting_name_from_markdown, generate_meeting_summary,
    language_name_from_code,
};
use crate::summary::provider_config::resolve_provider_config;
use crate::summary::templates;
use crate::summary::templates::Template;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
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
fn build_summary_result_json(
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
fn extract_cached_english_markdown(
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
    fn register_cancellation_token(meeting_id: &str) -> CancellationToken {
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
    fn cleanup_cancellation_token(meeting_id: &str) {
        if let Ok(mut registry) = CANCELLATION_REGISTRY.lock() {
            if registry.remove(meeting_id).is_some() {
                info!("Cleaned up cancellation token for meeting: {}", meeting_id);
            }
        }
    }

    async fn read_detected_summary_language(pool: &SqlitePool, meeting_id: &str) -> Option<String> {
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

    fn detect_summary_language_from_text(text: &str) -> Option<String> {
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

    /// Processes transcript in the background and generates summary
    ///
    /// This function is designed to be spawned as an async task and does not block
    /// the main thread. It updates the database with progress and results.
    ///
    /// # Arguments
    /// * `app` - Tauri app handle (BuiltInAI data dir + the post-completion
    ///   action-item extraction spawn, specs/0034)
    /// * `pool` - SQLx connection pool
    /// * `meeting_id` - Unique identifier for the meeting
    /// * `text` - Full transcript text
    /// * `model_provider` - LLM provider name (e.g., "ollama", "openai")
    /// * `model_name` - Specific model (e.g., "gpt-4", "llama3.2:latest")
    /// * `custom_prompt` - Optional user-provided context
    /// * `template_id` - Template identifier (e.g., "daily_standup", "standard_meeting")
    #[allow(clippy::too_many_arguments)] // cohesive param set; refactor deferred
    pub async fn process_transcript_background<R: tauri::Runtime>(
        app: AppHandle<R>,
        pool: SqlitePool,
        meeting_id: String,
        mut text: String,
        model_provider: String,
        model_name: String,
        custom_prompt: String,
        template_id: String,
        summary_language: Option<String>,
    ) {
        let start_time = Instant::now();
        info!(
            "Starting background processing for meeting_id: {}",
            meeting_id
        );

        // Register cancellation token for this meeting
        let cancellation_token = Self::register_cancellation_token(&meeting_id);

        // Diarization-aware transcript (specs/0010 P2 Task 9) + the resolved-name
        // fingerprint (specs/0044 WS3), from one shared loader in summary::refresh:
        // `Name: text` prefixes when speakers resolve (else `text` is untouched and
        // byte-identical to the no-diarization path), and the names hash the
        // post-naming trigger later compares against to detect stale summaries.
        let (attributed, transcript_has_speakers, speaker_names_hash) =
            crate::summary::refresh::load_attribution(&pool, &meeting_id).await;
        if let Some(attributed) = attributed {
            info!(
                "Speaker-attributed transcript assembled for meeting_id {} ({} chars)",
                meeting_id,
                attributed.len()
            );
            text = attributed;
        }

        // Role-weighted summary (specs/0012 Task 5): when the transcript is speaker-
        // attributed, resolve each speaker's role (speakers.person_id → people.role) and
        // build a compact "Participant roles" preamble. Roles weight EMPHASIS/ownership in
        // the summary, never inclusion (see ROLE_WEIGHTING_INSTRUCTIONS). When no speaker
        // has a role — or the transcript isn't attributed — this stays `None` and the
        // summary prompt is byte-identical to the no-role path. A DB error here is
        // non-fatal: we degrade to the un-weighted (but still attributed) summary.
        let role_preamble = if transcript_has_speakers {
            match PeopleRepository::get_meeting_speaker_roles(&pool, &meeting_id).await {
                Ok(roles) => {
                    let preamble = build_role_preamble(&roles);
                    if preamble.is_some() {
                        info!(
                            "Role-weighted summary: {} speaker role(s) resolved for meeting_id {}",
                            roles.len(),
                            meeting_id
                        );
                    }
                    preamble
                }
                Err(e) => {
                    warn!(
                        "Failed to load speaker roles for meeting_id={} ({}); summarizing without role weighting",
                        meeting_id, e
                    );
                    None
                }
            }
        } else {
            None
        };

        // Resolve provider + credentials/endpoints through the shared resolver (also used
        // by the manual action-item extraction command, which mirrors this run's provider
        // posture). Missing key / missing CustomOpenAI config / unknown provider all
        // surface here as one user-actionable failure.
        let provider_config =
            match resolve_provider_config(&pool, &model_provider, &model_name).await {
                Ok(config) => config,
                Err(e) => {
                    Self::update_process_failed(&pool, &meeting_id, &format!("{e:#}")).await;
                    return;
                }
            };
        let provider = provider_config.provider.clone();
        let ollama_endpoint = provider_config.ollama_endpoint.clone();
        let custom_openai_endpoint = provider_config.custom_openai_endpoint.clone();
        let custom_openai_max_tokens = provider_config.custom_openai_max_tokens;
        let custom_openai_temperature = provider_config.custom_openai_temperature;
        let custom_openai_top_p = provider_config.custom_openai_top_p;
        if provider == LLMProvider::CustomOpenAI {
            if let Some(endpoint) = &custom_openai_endpoint {
                info!("✓ Using custom OpenAI endpoint: {}", endpoint);
            }
        }
        let final_api_key = provider_config.api_key.clone();

        // Dynamically fetch context size based on provider and model (shared with the
        // aggregation engine, specs/0035 — behavior here is unchanged by the extraction).
        let token_threshold =
            resolve_context_budget(&provider, &model_name, ollama_endpoint.as_deref()).await;

        // Get app data directory for BuiltInAI provider
        let app_data_dir = app.path().app_data_dir().ok();

        if let Some(code) = &summary_language {
            info!("📝 Summary language preference: {}", code);
        }

        // Notes-aware summary (spec 0003 pivot): load the user's own manual notes
        // for this meeting and inject them into the summary prompt as high-priority
        // grounding. Read server-side at generation time (no new IPC args). A
        // missing notes row or a load error is non-fatal — we just summarize the
        // transcript alone.
        //
        // Loaded here (before language detection) so a notes-only meeting (spec
        // 0015: empty transcript) can fall back to detecting the summary language
        // from the notes instead of an empty transcript.
        // Load live notes AND (specs/0036) pre-call PREP notes in one read. Prep is the
        // "intended agenda" written before the meeting; when present it's prepended as a
        // clearly-labeled block so the summary can reflect what was covered vs. skipped.
        // When there are no prep notes the value is byte-identical to before, so summaries
        // for meetings without prep are unchanged.
        let (user_notes, prep_notes) = match MeetingNotesRepository::get_notes(&pool, &meeting_id)
            .await
        {
            Ok(Some(note)) => {
                let clean =
                    |v: Option<String>| v.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
                (clean(note.notes_markdown), clean(note.prep_markdown))
            }
            Ok(None) => (None, None),
            Err(e) => {
                warn!(
                    "Failed to load user notes for meeting_id={} ({}); summarizing transcript only",
                    meeting_id, e
                );
                (None, None)
            }
        };
        let user_notes = match (prep_notes, user_notes) {
            (Some(prep), Some(notes)) => Some(format!(
                "## Intended agenda (what I planned to cover, written before the meeting)\n\n{prep}\n\n\
                 ## Notes taken during the meeting\n\n{notes}"
            )),
            (Some(prep), None) => Some(format!(
                "## Intended agenda (what I planned to cover, written before the meeting)\n\n{prep}"
            )),
            (None, notes) => notes,
        };
        if user_notes.is_some() {
            info!(
                "Notes-aware summary: user notes / prep agenda found for meeting_id {}",
                meeting_id
            );
        }

        // Detected summary language: prefer the recording's stored metadata, then
        // the transcript text. For a notes-only meeting the transcript is empty
        // (and there is no metadata folder), so fall back to the notes' language;
        // if everything is empty/undetectable this stays `None` and the default
        // language applies. For a recorded meeting WITH a transcript, the
        // transcript-text detection short-circuits before the notes branch, so
        // output is unchanged.
        let detected_summary_language = Self::read_detected_summary_language(&pool, &meeting_id)
            .await
            .or_else(|| Self::detect_summary_language_from_text(&text))
            .or_else(|| {
                user_notes
                    .as_deref()
                    .and_then(Self::detect_summary_language_from_text)
            });

        if let Some(code) = &detected_summary_language {
            info!("📝 Detected transcript summary language: {}", code);
        }

        // specs/0053 W3: `auto` is a reserved id, never a file. A meeting with an
        // already-derived outline behaves like a fixed template (same fingerprint,
        // same cache path); otherwise this run derives one and bypasses the cache.
        let stored_outline = if template_id == AUTO_TEMPLATE_ID {
            SummaryOutlineRepository::get(&pool, &meeting_id)
                .await
                .unwrap_or_else(|e| {
                    warn!("Failed to read the stored outline for {meeting_id}: {e:#}");
                    None
                })
        } else {
            None
        };

        let auto_template: Option<Template> = stored_outline.as_ref().and_then(|s| {
            match serde_json::from_str::<Outline>(&s.outline_json) {
                Ok(outline) => Some(crate::summary::outline::to_template(&outline)),
                Err(e) => {
                    // Privacy: never `e`'s Display — for `invalid type`/`unknown
                    // variant`/`unknown field` it embeds the offending meeting-derived value verbatim (specs/0053 C2).
                    warn!("Stored outline for {meeting_id} did not deserialize (category={:?}, line={}, column={}); re-deriving", e.classify(), e.line(), e.column());
                    None
                }
            }
        });

        let template = match (&auto_template, template_id.as_str()) {
            // Auto with a usable stored outline.
            (Some(t), _) => t.clone(),
            // Auto with nothing stored: a placeholder that is never rendered —
            // processor.rs derives the real one. Its fingerprint deliberately
            // will not match any cached summary.
            (None, id) if id == AUTO_TEMPLATE_ID => {
                crate::summary::outline::to_template(&crate::summary::outline::fallback_outline())
            }
            // Every fixed template: unchanged path.
            (None, id) => match templates::get_template(id) {
                Ok(template) => template,
                Err(e) => {
                    let err_msg = format!("Failed to load template '{}': {}", id, e);
                    Self::update_process_failed(&pool, &meeting_id, &err_msg).await;
                    return;
                }
            },
        };

        let will_derive = template_id == AUTO_TEMPLATE_ID && auto_template.is_none();

        let template_fingerprint = if will_derive {
            // Not yet known — force a cache miss for this run.
            format!("auto:underived:{meeting_id}")
        } else {
            template_cache_fingerprint(&template)
        };

        let cache_source = build_summary_cache_source(
            &text,
            &custom_prompt,
            &template_id,
            &template_fingerprint,
            token_threshold,
            &model_provider,
            &model_name,
            ollama_endpoint.as_deref(),
            custom_openai_endpoint.as_deref(),
            custom_openai_max_tokens,
            custom_openai_temperature,
            custom_openai_top_p,
        );

        let cached_english = match SummaryProcessesRepository::get_summary_data(&pool, &meeting_id).await {
            Err(e) => {
                warn!(
                    "Failed to load prior summary row for cache lookup (meeting_id={}): {}. Falling back to full pass-1 generation.",
                    meeting_id, e
                );
                None
            }
            Ok(None) => None,
            Ok(Some(process)) => process.result.and_then(|raw| {
                match extract_cached_english_markdown(
                    &raw,
                    &cache_source,
                    summary_language.as_deref(),
                ) {
                    Ok(opt) => opt,
                    Err(e) => {
                        warn!(
                            "Cached summary result for meeting_id={} is not valid JSON ({}); ignoring cache.",
                            meeting_id, e
                        );
                        None
                    }
                }
            }),
        };

        let client = reqwest::Client::new();
        let result = generate_meeting_summary(
            &client,
            &provider,
            &model_name,
            &final_api_key,
            &text,
            &custom_prompt,
            &template_id,
            if will_derive {
                TemplateChoice::DeriveAuto
            } else {
                TemplateChoice::Fixed(&template)
            },
            token_threshold,
            ollama_endpoint.as_deref(),
            custom_openai_endpoint.as_deref(),
            custom_openai_max_tokens,
            custom_openai_temperature,
            custom_openai_top_p,
            app_data_dir.as_ref(),
            Some(&cancellation_token),
            summary_language.as_deref(),
            detected_summary_language.as_deref(),
            cached_english.as_deref(),
            user_notes.as_deref(),
            transcript_has_speakers,
            role_preamble.as_deref(),
        )
        .await;

        let duration = start_time.elapsed().as_secs_f64();

        // Clean up cancellation token regardless of outcome
        Self::cleanup_cancellation_token(&meeting_id);

        match result {
            Ok((final_markdown, english_markdown, accounting, derived_outline)) => {
                // specs/0053 W3: persist before anything else consumes it — the
                // extraction gate reads has_commitments, and the next
                // regeneration reads the outline back to keep the shape stable.
                if let Some(outline) = &derived_outline {
                    match serde_json::to_string(outline) {
                        Ok(json) => {
                            if let Err(e) = SummaryOutlineRepository::upsert(
                                &pool,
                                &meeting_id,
                                &json,
                                outline.has_commitments,
                            )
                            .await
                            {
                                warn!("Failed to persist the outline for {meeting_id}: {e:#}");
                            }
                        }
                        Err(e) => warn!("Failed to serialize the outline for {meeting_id}: {e}"),
                    }
                }

                // `num_chunks` persisted to the process row = chunks that actually made it
                // into the report (successfully processed), preserving prior semantics.
                let num_chunks = accounting.processed;
                if accounting.is_complete() {
                    info!(
                        "✓ Successfully processed {} chunk(s) for meeting_id: {}. Duration: {:.2}s",
                        accounting.processed, meeting_id, duration
                    );
                } else {
                    warn!(
                        "⚠ PARTIAL summary for meeting_id: {}: {}/{} chunks in report, {} dropped after retries. Duration: {:.2}s",
                        meeting_id, accounting.processed, accounting.total, accounting.failed, duration
                    );
                }
                info!("Final markdown generated ({} chars)", final_markdown.len());

                // The summary must NOT rename a title the user owns. Two cases (specs/0024 WS6.1):
                //  (a) a meeting that adopted a calendar event's identity (specs/0015 Join &
                //      Record) — its title is the event's name, e.g. "Test event"; and
                //  (b) a title the user manually edited (`title_manually_set`).
                // Everything else (an ad-hoc recording still carrying a date-stamp/default title)
                // IS auto-named from the summary. Best-effort: a metadata-load failure falls back
                // to the previous always-rename behavior.
                let title_is_authoritative =
                    match MeetingsRepository::get_meeting_metadata(&pool, &meeting_id).await {
                        Ok(Some(m)) => {
                            let has_calendar = m
                                .calendar_event_id
                                .as_deref()
                                .map(|id| !id.trim().is_empty())
                                .unwrap_or(false);
                            has_calendar || m.title_manually_set
                        }
                        _ => false,
                    };

                if title_is_authoritative {
                    info!(
                        "Preserving user-owned title for {} (calendar-adopted or manually set; skipping summary rename)",
                        meeting_id
                    );
                } else if let Some(name) =
                    extract_meeting_name_from_markdown(&final_markdown).filter(|n| !n.is_empty())
                {
                    info!("Extracted meeting name from summary: '{}'", name);
                    if let Err(e) =
                        MeetingsRepository::update_meeting_name(&pool, &meeting_id, &name).await
                    {
                        error!("Failed to update meeting name for {}: {}", meeting_id, e);
                    } else {
                        info!("Successfully updated meeting name for {}", meeting_id);
                    }
                }

                // Strip the title ONCE: this exact string is (a) the stored `$.markdown`
                // the UI renders and `api_extract_action_items` reads back, and (b) the
                // input the background action-item extraction fingerprints below. Using
                // the same representation on both paths keeps the extraction ledger's
                // fingerprint stable across auto and manual runs (specs/0034 acceptance
                // #4 — a raw-vs-stripped mismatch would make every "Scan again" a full
                // re-extraction).
                let stored_summary_markdown = strip_title_if_present(&final_markdown);
                let mut result_json = build_summary_result_json(
                    &stored_summary_markdown,
                    &english_markdown,
                    cache_source,
                    summary_language.as_deref(),
                );

                // Surface chunk-outcome accounting into the stored result so the UI can
                // warn the user when a summary is PARTIAL (some transcript content was
                // dropped after retries) instead of presenting it as complete. Additive
                // top-level field — the frontend can read `summary_status.complete` and
                // `failed_chunks`; older clients ignore it. See the report's "frontend
                // contract" note.
                if let Some(obj) = result_json.as_object_mut() {
                    obj.insert(
                        "summary_status".to_string(),
                        serde_json::json!({
                            "complete": accounting.is_complete(),
                            "total_chunks": accounting.total,
                            "processed_chunks": accounting.processed,
                            "failed_chunks": accounting.failed,
                        }),
                    );
                }

                // specs/0041 WS2: persist whether this generation was speaker-attributed
                // (the `transcript_has_speakers` / any_speaker computation above) plus the
                // fingerprint of the stored markdown. The post-diarization trigger
                // (summary::refresh) auto-regenerates only speakerless AND pristine
                // summaries; a regenerated run has speakers, sets 1, and can't loop.
                let generated_markdown_hash = stable_text_fingerprint(&stored_summary_markdown);

                // Update database with completed status
                if let Err(e) = SummaryProcessesRepository::update_process_completed(
                    &pool,
                    &meeting_id,
                    result_json,
                    num_chunks,
                    duration,
                    transcript_has_speakers,
                    &generated_markdown_hash,
                    &speaker_names_hash,
                )
                .await
                {
                    error!("Failed to save completed process for {}: {}", meeting_id, e);
                } else {
                    info!("Summary saved successfully for meeting_id: {}", meeting_id);

                    // Action-item extraction (specs/0034): triggered off the summary
                    // WRITE — the artifact it extracts from — so record-only meetings
                    // transcribed days later extract on their late summary, and no
                    // summary means no extraction. Spawned fire-and-forget: it never
                    // delays or fails the (already persisted) summary. Uses the EXACT
                    // provider/model/key/endpoints this run used (decided 2026-07-03:
                    // no separate extraction provider setting) and the STORED (title-
                    // stripped) markdown, so the manual command's re-read fingerprints
                    // identically.
                    tauri::async_runtime::spawn(async move {
                        crate::action_items::run_background_extraction(
                            &app,
                            &pool,
                            &meeting_id,
                            &template_id,
                            &stored_summary_markdown,
                            user_notes.as_deref(),
                            &provider_config,
                        )
                        .await;
                    });
                }
            }
            Err(e) => {
                // Check if error is due to cancellation
                if e.contains("cancelled") {
                    info!(
                        "Summary generation was cancelled for meeting_id: {}",
                        meeting_id
                    );
                    if let Err(db_err) =
                        SummaryProcessesRepository::update_process_cancelled(&pool, &meeting_id)
                            .await
                    {
                        error!(
                            "Failed to update DB status to cancelled for {}: {}",
                            meeting_id, db_err
                        );
                    }
                } else {
                    Self::update_process_failed(&pool, &meeting_id, &e).await;
                }
            }
        }
    }

    /// Updates the summary process status to failed with error message
    ///
    /// # Arguments
    /// * `pool` - SQLx connection pool
    /// * `meeting_id` - Meeting identifier
    /// * `error_msg` - Error message to store
    async fn update_process_failed(pool: &SqlitePool, meeting_id: &str, error_msg: &str) {
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
mod tests {
    use super::*;
    use crate::summary::templates::Template;

    fn sample_cache_source() -> SummaryCacheSource {
        let template_fingerprint = stable_text_fingerprint("standard template prompt");
        build_summary_cache_source(
            "transcript body",
            "custom prompt",
            "standard_meeting",
            &template_fingerprint,
            3700,
            "ollama",
            "gemma3:1b",
            Some("http://localhost:11434"),
            None,
            None,
            None,
            None,
        )
    }

    fn test_template(section_title: &str) -> Template {
        Template {
            name: "Test".to_string(),
            description: "Test template".to_string(),
            sections: vec![crate::summary::templates::TemplateSection {
                title: section_title.to_string(),
                instruction: "Summarize this section".to_string(),
                format: "paragraph".to_string(),
                item_format: None,
                example_item_format: None,
            }],
        }
    }

    #[test]
    fn test_template_cache_fingerprint_changes_with_rendered_template() {
        assert_ne!(
            template_cache_fingerprint(&test_template("Summary")),
            template_cache_fingerprint(&test_template("Decisions"))
        );
    }

    #[test]
    fn test_legacy_english_markdown_field_is_cache_miss() {
        let raw = serde_json::json!({
            "markdown": "translated",
            "english_markdown": "# Old English\nBody"
        })
        .to_string();

        assert_eq!(
            extract_cached_english_markdown(&raw, &sample_cache_source(), Some("de")).unwrap(),
            None
        );
    }

    #[test]
    fn test_matching_source_changed_translation_target_reuses_cache() {
        let source = sample_cache_source();
        let raw = build_summary_result_json(
            "# Reunion\n## Points\nBonjour",
            "# Meeting\n## Points\nHello",
            source.clone(),
            Some("fr"),
        )
        .to_string();

        assert_eq!(
            extract_cached_english_markdown(&raw, &source, Some("de")).unwrap(),
            Some("# Meeting\n## Points\nHello".to_string())
        );
    }

    #[test]
    fn test_same_language_regeneration_rejects_cache() {
        let source = sample_cache_source();
        let raw = build_summary_result_json(
            "# Reunion\n## Points\nBonjour",
            "# Meeting\n## Points\nHello",
            source.clone(),
            Some("fr"),
        )
        .to_string();

        assert_eq!(
            extract_cached_english_markdown(&raw, &source, Some("fr")).unwrap(),
            None
        );
    }

    #[test]
    fn test_changed_summary_inputs_reject_cache() {
        let source = sample_cache_source();
        let template_fingerprint = source.template_fingerprint.clone();
        let raw = build_summary_result_json(
            "# Reunion\n## Points\nBonjour",
            "# Meeting\n## Points\nHello",
            source,
            Some("fr"),
        )
        .to_string();

        let changed_sources = [
            build_summary_cache_source(
                "changed transcript",
                "custom prompt",
                "standard_meeting",
                &template_fingerprint,
                3700,
                "ollama",
                "gemma3:1b",
                Some("http://localhost:11434"),
                None,
                None,
                None,
                None,
            ),
            build_summary_cache_source(
                "transcript body",
                "changed prompt",
                "standard_meeting",
                &template_fingerprint,
                3700,
                "ollama",
                "gemma3:1b",
                Some("http://localhost:11434"),
                None,
                None,
                None,
                None,
            ),
            build_summary_cache_source(
                "transcript body",
                "custom prompt",
                "daily_standup",
                &template_fingerprint,
                3700,
                "ollama",
                "gemma3:1b",
                Some("http://localhost:11434"),
                None,
                None,
                None,
                None,
            ),
            build_summary_cache_source(
                "transcript body",
                "custom prompt",
                "standard_meeting",
                &template_fingerprint,
                3700,
                "openai",
                "gemma3:1b",
                Some("http://localhost:11434"),
                None,
                None,
                None,
                None,
            ),
            build_summary_cache_source(
                "transcript body",
                "custom prompt",
                "standard_meeting",
                &template_fingerprint,
                3700,
                "ollama",
                "qwen2.5:3b",
                Some("http://localhost:11434"),
                None,
                None,
                None,
                None,
            ),
            build_summary_cache_source(
                "transcript body",
                "custom prompt",
                "standard_meeting",
                &template_fingerprint,
                3700,
                "ollama",
                "gemma3:1b",
                Some("http://localhost:11500"),
                None,
                None,
                None,
                None,
            ),
            build_summary_cache_source(
                "transcript body",
                "custom prompt",
                "standard_meeting",
                &template_fingerprint,
                3700,
                "ollama",
                "gemma3:1b",
                Some("http://localhost:11434"),
                Some("https://custom.example/v1"),
                Some(2048),
                Some(0.2),
                Some(0.9),
            ),
        ];

        for changed_source in changed_sources {
            assert_eq!(
                extract_cached_english_markdown(&raw, &changed_source, Some("de")).unwrap(),
                None
            );
        }
    }

    #[test]
    fn test_changed_template_content_rejects_cache() {
        let source = sample_cache_source();
        let raw = build_summary_result_json(
            "# Reunion\n## Points\nBonjour",
            "# Meeting\n## Points\nHello",
            source.clone(),
            Some("fr"),
        )
        .to_string();

        let changed_template = SummaryCacheSource {
            template_fingerprint: stable_text_fingerprint("changed template prompt"),
            ..source
        };

        assert_eq!(
            extract_cached_english_markdown(&raw, &changed_template, Some("de")).unwrap(),
            None
        );
    }

    #[test]
    fn test_changed_token_threshold_rejects_cache() {
        let source = sample_cache_source();
        let raw = build_summary_result_json(
            "# Reunion\n## Points\nBonjour",
            "# Meeting\n## Points\nHello",
            source.clone(),
            Some("fr"),
        )
        .to_string();

        let changed_threshold = SummaryCacheSource {
            token_threshold: 8192,
            ..source
        };

        assert_eq!(
            extract_cached_english_markdown(&raw, &changed_threshold, Some("de")).unwrap(),
            None
        );
    }

    #[test]
    fn test_result_json_stores_stripped_display_markdown_but_keeps_cache_title() {
        // The completion arm strips the title ONCE and hands the stripped string to the
        // builder (which stores it verbatim); the English cache keeps its title.
        let result = build_summary_result_json(
            &strip_title_if_present("# Translated Title\n## Decisions\nDone"),
            "# English Title\n## Decisions\nDone",
            sample_cache_source(),
            Some("fr"),
        );

        assert_eq!(result["markdown"], "## Decisions\nDone");
        assert_eq!(
            result["english_cache"]["markdown"],
            "# English Title\n## Decisions\nDone"
        );
    }

    /// Regression (specs/0034 acceptance #4, cross-path fingerprint): the background
    /// extraction spawn fingerprints the SAME string the completion arm stores at
    /// `$.markdown` — which is exactly what `api_extract_action_items` reads back for a
    /// manual "Scan again". If the two paths ever diverge again (e.g. one fingerprints
    /// the raw titled markdown), an auto-then-manual run on an UNCHANGED summary stops
    /// being a ledger no-op and every scan becomes a full re-extraction.
    #[test]
    fn auto_then_manual_extraction_fingerprints_match_on_unchanged_summary() {
        use crate::action_items::diff::extraction_fingerprint;

        let final_markdown = "# Weekly Sync\n## Action Items\n- send the deck to Alice";
        let user_notes = Some("deck first");

        // The completion arm: strip once, store, and fingerprint the same string.
        let auto_extraction_input = strip_title_if_present(final_markdown);
        let stored = build_summary_result_json(
            &auto_extraction_input,
            final_markdown,
            sample_cache_source(),
            None,
        )
        .to_string();

        // The manual command: read `$.markdown` back from the stored result row.
        let manual_extraction_input: String = serde_json::from_str::<serde_json::Value>(&stored)
            .unwrap()["markdown"]
            .as_str()
            .unwrap()
            .to_string();

        assert_eq!(
            extraction_fingerprint(&auto_extraction_input, user_notes),
            extraction_fingerprint(&manual_extraction_input, user_notes),
            "auto and manual extraction must fingerprint the identical stored representation"
        );
    }

    #[test]
    fn test_extract_cached_english_from_malformed_json_errors() {
        let raw = r#"{ not valid json"#;
        assert!(extract_cached_english_markdown(raw, &sample_cache_source(), Some("de")).is_err());
    }

    /// Shape lock for full-text search (specs/0033): the FTS migration
    /// (`migrations/20260706000000_add_fts5_search.sql`) indexes summaries via a
    /// generated column computed as
    /// `CASE WHEN result IS NOT NULL AND json_valid(result) THEN json_extract(result, '$.markdown') END`,
    /// where `result` is the JSON THIS builder produces. If the result shape
    /// ever moves the display markdown off `$.markdown`, that column silently
    /// becomes NULL and summaries drop out of search with no other test failing
    /// — this test fails first. Keep the SQL expression here byte-identical to
    /// the migration's `summary_text` definition.
    #[tokio::test]
    async fn summary_result_json_markdown_is_extractable_by_fts_migration() {
        use sqlx::{ConnectOptions, Row};

        let markdown = "## Decisions\nadopt the roadmap";
        // No leading H1, so the stored `$.markdown` is byte-identical to the input.
        let result =
            build_summary_result_json(markdown, markdown, sample_cache_source(), None).to_string();

        let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .connect()
            .await
            .expect("in-memory SQLite connection");

        let extracted: Option<String> = sqlx::query(
            "SELECT CASE WHEN ? IS NOT NULL AND json_valid(?) \
                    THEN json_extract(?, '$.markdown') END",
        )
        .bind(&result)
        .bind(&result)
        .bind(&result)
        .fetch_one(&mut conn)
        .await
        .expect("json_extract over the builder's output")
        .get(0);

        assert_eq!(
            extracted.as_deref(),
            Some(markdown),
            "summary display markdown must live at $.markdown in summary_processes.result \
             (the FTS migration's generated column reads it there)"
        );
    }
}
