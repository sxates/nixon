use crate::summary::llm_client::{generate_summary, LLMProvider};
use crate::summary::outline::{Outline, TemplateChoice};
use crate::summary::prompts::{
    build_chunk_summary_user_prompt, build_combine_summary_user_prompt,
    build_final_synthesis_system_prompt, build_user_notes_block,
    english_normalization_system_prompt, translation_system_prompt,
};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Client;
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Max number of *additional* attempts for a retryable LLM failure on a single
/// pass (chunk map, combine, final synthesis, translate, normalize). So a chunk
/// is tried up to `MAX_LLM_RETRIES + 1` times total before it is given up on.
const MAX_LLM_RETRIES: u32 = 2;

/// Base backoff between retryable attempts; doubled each attempt (0.5s, 1s, 2s…),
/// capped at [`MAX_LLM_BACKOFF`].
const BASE_LLM_BACKOFF: Duration = Duration::from_millis(500);
const MAX_LLM_BACKOFF: Duration = Duration::from_secs(8);

/// Recursion depth cap for the hierarchical combine reduce, so a pathological set
/// of summaries can't loop forever.
const MAX_COMBINE_DEPTH: usize = 6;

/// Outcome accounting for the map (per-chunk) pass of a multi-level summary.
///
/// The old code counted only *successful* chunks and silently `continue`d past a
/// failed chunk, then presented the partial report as if complete. This struct is
/// carried out of [`generate_meeting_summary`] so the service/UI can tell the user
/// the summary is **partial** (some transcript content was dropped) rather than
/// silently lying. Single-pass and cached paths report `single_pass()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkAccounting {
    /// Chunks attempted in the map pass (1 for single-pass / cached).
    pub total: i64,
    /// Chunks that were summarized successfully.
    pub processed: i64,
    /// Chunks given up on after exhausting retries (dropped from the summary).
    pub failed: i64,
}

impl ChunkAccounting {
    /// A single-pass (or cached) summary: one unit of work, no partial loss.
    pub fn single_pass() -> Self {
        Self {
            total: 1,
            processed: 1,
            failed: 0,
        }
    }

    /// True when no chunk was dropped — the report covers the whole transcript.
    pub fn is_complete(&self) -> bool {
        self.failed == 0
    }
}

/// Classifies an LLM error string as transient (worth retrying) vs terminal.
///
/// Conservative on purpose: only clearly-transient conditions (timeouts, network
/// send failures, provider overload / rate-limit / 5xx) are retried. Everything
/// else — auth failures, malformed requests, cancellation — is treated as terminal
/// so we don't hammer a provider on a request that can never succeed. We only have
/// the error *string* (no status code) here, so we match on well-known markers.
fn is_retryable_llm_error(e: &str) -> bool {
    if e.contains("cancelled") {
        return false;
    }
    let lower = e.to_lowercase();
    // Terminal client-side conditions: never retry.
    let terminal_markers = [
        "invalid api key",
        "api key not found",
        "authentication",
        "unauthorized",
        "invalid_request",
        "invalid request",
        "not configured",
        "unsupported llm provider",
        "unknown model",
    ];
    if terminal_markers.iter().any(|m| lower.contains(m)) {
        return false;
    }
    let transient_markers = [
        "timed out",
        "timeout",
        "failed to send request",
        "error sending request",
        "connection",
        "connect",
        "reset by peer",
        "overloaded",
        "rate limit",
        "rate_limit",
        "too many requests",
        "429",
        "500",
        "502",
        "503",
        "504",
        "server_error",
        "internal server error",
        "temporarily unavailable",
        "service unavailable",
    ];
    transient_markers.iter().any(|m| lower.contains(m))
}

/// Sleeps for `delay`, but returns early (`false`) if the cancellation token fires
/// during the wait. Returns `true` if the full delay elapsed.
async fn backoff_sleep(delay: Duration, cancellation_token: Option<&CancellationToken>) -> bool {
    match cancellation_token {
        Some(token) => {
            tokio::select! {
                _ = tokio::time::sleep(delay) => true,
                _ = token.cancelled() => false,
            }
        }
        None => {
            tokio::time::sleep(delay).await;
            true
        }
    }
}

/// Computes the backoff for retry attempt `attempt` (0-based), capped.
fn backoff_for_attempt(attempt: u32) -> Duration {
    let scaled = BASE_LLM_BACKOFF.saturating_mul(1u32 << attempt.min(6));
    scaled.min(MAX_LLM_BACKOFF)
}

/// Wraps [`generate_summary`] with bounded retry + backoff for retryable failures.
///
/// Cancellation and terminal errors return immediately. Only the pass content is
/// retried — never logged (privacy: we log attempt counts, not transcript text).
///
/// `pub(crate)`: also used by the aggregation engine's LLM closure (specs/0035) so
/// map/reduce calls get the same bounded-retry behavior summaries get.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn generate_summary_with_retry(
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
    let mut attempt: u32 = 0;
    loop {
        if let Some(token) = cancellation_token {
            if token.is_cancelled() {
                return Err("Summary generation was cancelled".to_string());
            }
        }

        let result = generate_summary(
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
        )
        .await;

        match result {
            Ok(text) => return Ok(text),
            Err(e) => {
                if attempt >= MAX_LLM_RETRIES || !is_retryable_llm_error(&e) {
                    return Err(e);
                }
                let delay = backoff_for_attempt(attempt);
                warn!(
                    "LLM pass failed (attempt {}/{}), retrying after {:?}",
                    attempt + 1,
                    MAX_LLM_RETRIES + 1,
                    delay
                );
                if !backoff_sleep(delay, cancellation_token).await {
                    return Err("Summary generation was cancelled".to_string());
                }
                attempt += 1;
            }
        }
    }
}

// Compile regex once and reuse (significant performance improvement for repeated calls)
// Matches common "reasoning" wrappers various local models emit before the answer:
// <think>…</think>, <thinking>…</thinking>, <reasoning>…</reasoning>, <reflection>…</reflection>.
// Case-insensitive and dot-matches-newline so multi-line reasoning blocks are removed.
static THINKING_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?is)<(think|thinking|reason|reasoning|reflection)>.*?</\s*(think|thinking|reason|reasoning|reflection)\s*>").unwrap()
});

pub(crate) const ENGLISH_BASE_SUMMARY_INSTRUCTION: &str =
    "**Write the summary/report in English regardless of transcript language; non-English prose is invalid.**";

fn resolve_cached_english<'a>(
    cached: Option<&'a str>,
    summary_language: Option<&str>,
) -> Option<&'a str> {
    let cached_clean = cached.filter(|s| !s.trim().is_empty())?;
    let target_is_translation = summary_language
        .and_then(language_name_from_code)
        .is_some_and(|n| n != "English");
    if target_is_translation {
        Some(cached_clean)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalLanguageAction {
    ReturnEnglish,
    NormalizeEnglish,
    Translate(&'static str),
}

fn resolve_final_language_action(
    summary_language: Option<&str>,
    detected_transcript_language: Option<&str>,
) -> FinalLanguageAction {
    match summary_language.and_then(language_name_from_code) {
        Some(name) if name != "English" => FinalLanguageAction::Translate(name),
        _ => match detected_transcript_language.and_then(language_name_from_code) {
            Some("English") => FinalLanguageAction::ReturnEnglish,
            _ => FinalLanguageAction::NormalizeEnglish,
        },
    }
}

fn english_markdown_after_normalization_result(
    original_markdown: &str,
    normalization_result: Result<String, String>,
) -> Result<String, String> {
    match normalization_result {
        Ok(normalized) => Ok(normalized),
        Err(e) if e.contains("cancelled") => Err(e),
        Err(e) => {
            error!(
                "English normalization pass failed; returning pass-1 markdown without hard fail: {}",
                e
            );
            Ok(original_markdown.to_string())
        }
    }
}

/// Maps a BCP-47 tag to the English language name used inside LLM prompts.
///
/// LLMs respond far more reliably to "in Spanish" than to "in es". Regional
/// tags (`pt-BR`, `en_GB`) are normalised to their base language; Chinese
/// variants are disambiguated. Unknown codes return None so the caller falls
/// back to English rather than injecting a literal ISO code into the prompt.
pub(crate) fn language_name_from_code(code: &str) -> Option<&'static str> {
    let normalised = code.to_ascii_lowercase().replace('_', "-");
    let lookup: &str = match normalised.as_str() {
        "zh-cn" => "zh",
        "zh-tw" => return Some("Traditional Chinese"),
        other => other.split('-').next().unwrap_or(other),
    };
    match lookup {
        "en" => Some("English"),
        "zh" => Some("Chinese"),
        "de" => Some("German"),
        "es" => Some("Spanish"),
        "ru" => Some("Russian"),
        "ko" => Some("Korean"),
        "fr" => Some("French"),
        "ja" => Some("Japanese"),
        "pt" => Some("Portuguese"),
        "it" => Some("Italian"),
        "nl" => Some("Dutch"),
        "pl" => Some("Polish"),
        "ar" => Some("Arabic"),
        "hi" => Some("Hindi"),
        "ta" => Some("Tamil"),
        "tr" => Some("Turkish"),
        "vi" => Some("Vietnamese"),
        "th" => Some("Thai"),
        "id" => Some("Indonesian"),
        "sv" => Some("Swedish"),
        "cs" => Some("Czech"),
        "da" => Some("Danish"),
        "fi" => Some("Finnish"),
        "el" => Some("Greek"),
        "he" => Some("Hebrew"),
        "hu" => Some("Hungarian"),
        "no" => Some("Norwegian"),
        "ro" => Some("Romanian"),
        "uk" => Some("Ukrainian"),
        _ => None,
    }
}

// specs/0044 WS2 + 0042 ratchet: sizing lives in summary/length.rs; re-exported
// here so existing `processor::rough_token_count` callers keep their path.
pub use crate::summary::length::{length_guidance, rough_token_count};


/// Builds the participant-role preamble prepended to the speaker-attributed transcript
/// when at least one speaker has a role (specs/0012 Task 5). Returns `None` for an empty
/// list so the caller takes the byte-identical no-role path.
///
/// A compact block (less token-noise and more reliable than per-line annotation) listing
/// each labeled speaker once: `- <display_name> — <role>`. The caller prepends this to
/// the transcript and enables [`ROLE_WEIGHTING_INSTRUCTIONS`] in the system prompt.
pub fn build_role_preamble(roles: &[(String, String)]) -> Option<String> {
    let lines: Vec<String> = roles
        .iter()
        .filter_map(|(name, role)| {
            let name = name.trim();
            let role = role.trim();
            if name.is_empty() || role.is_empty() {
                return None;
            }
            Some(format!("- {name} — {role}"))
        })
        .collect();

    if lines.is_empty() {
        return None;
    }

    Some(format!(
        "Participant roles (for weighting, not for filtering):\n{}",
        lines.join("\n")
    ))
}

/// Builds the speaker-attributed transcript text fed to the summary when
/// diarization data exists. Each segment becomes one `Display Name: text` line,
/// consecutive lines from the same speaker are NOT merged (kept one-per-segment
/// for simple, deterministic alignment with the stored rows).
///
/// Returns `(assembled_text, any_speaker_present)`:
/// - `any_speaker_present` is true iff at least one segment resolved to a speaker
///   display name. The caller uses this to decide whether to switch to the
///   attributed transcript and turn on the attribution prompt instruction.
/// - Segments whose `speaker_name` is `None`/blank are emitted **without** a
///   prefix (no `Unknown:` noise), exactly as the speaker-less path renders them.
///
/// Empty/whitespace-only segment text is dropped and the rest joined with `\n`,
/// mirroring [`crate::database::repositories::transcript::TranscriptsRepository::get_full_transcript`]
/// so a meeting with zero resolved speakers produces byte-identical output to today.
pub fn build_speaker_attributed_transcript(
    segments: &[(Option<String>, String)],
) -> (String, bool) {
    let mut any_speaker_present = false;
    let lines: Vec<String> = segments
        .iter()
        .filter_map(|(speaker_name, text)| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            match speaker_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
            {
                Some(name) => {
                    any_speaker_present = true;
                    Some(format!("{name}: {trimmed}"))
                }
                None => Some(trimmed.to_string()),
            }
        })
        .collect();

    (lines.join("\n"), any_speaker_present)
}

/// Whether to summarize in a single pass instead of multi-level chunking.
///
/// Single-pass iff the content fits under the caller-supplied context threshold —
/// **for every provider**. Previously cloud providers (OpenAI/Claude/Groq/OpenRouter/
/// CustomOpenAI) were forced single-pass unconditionally against a bogus "effectively
/// unlimited" 100k threshold, so a long meeting on a small-context cloud model (many
/// Groq / custom-OpenAI-compatible models are 8k–32k) silently overflowed and got
/// truncated. Now the *service* sizes `token_threshold` to each model's real/conservative
/// context window, and any provider whose content exceeds it takes the chunking path.
///
/// Notes-only meetings (spec 0015) have an empty transcript, so `total_tokens`
/// is 0 — strictly below any positive threshold — which still routes them to the
/// single-pass branch for EVERY provider (feeding the empty transcript straight into
/// the notes-grounding final synthesis pass, never calling `chunk_text`).
fn use_single_pass(total_tokens: usize, token_threshold: usize) -> bool {
    total_tokens < token_threshold
}

/// Chunks text into overlapping segments based on token count
/// Uses character-based chunking for proper Unicode support
///
/// # Arguments
/// * `text` - The text to chunk
/// * `chunk_size_tokens` - Maximum tokens per chunk
/// * `overlap_tokens` - Number of overlapping tokens between chunks
///
/// # Returns
/// Vector of text chunks with smart word-boundary splitting
pub fn chunk_text(text: &str, chunk_size_tokens: usize, overlap_tokens: usize) -> Vec<String> {
    info!(
        "Chunking text with token-based chunk_size: {} and overlap: {}",
        chunk_size_tokens, overlap_tokens
    );

    if text.is_empty() || chunk_size_tokens == 0 {
        return vec![];
    }

    // Collect characters for indexing (needed for proper Unicode support)
    let chars: Vec<char> = text.chars().collect();
    let total_chars = chars.len();

    // Convert token-based sizes to character-based sizes using THIS text's actual
    // chars-per-token ratio (derived from the script-aware `rough_token_count`), so a
    // dense-script (CJK/Thai) transcript packs ~1 char/token instead of the Latin
    // ~2.85 char/token — otherwise the byte window would hold ~3x the intended tokens
    // and overflow the model's context. Falls back to the Latin ratio if the estimate
    // is degenerate.
    let estimated_tokens = rough_token_count(text).max(1);
    let chars_per_token = (total_chars as f64 / estimated_tokens as f64).max(1.0);
    let chunk_size_chars = (chunk_size_tokens as f64 * chars_per_token).ceil() as usize;
    let overlap_chars = (overlap_tokens as f64 * chars_per_token).ceil() as usize;

    if total_chars <= chunk_size_chars {
        info!("Text is shorter than chunk size, returning as a single chunk.");
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start_char = 0;
    // Step is the size of the non-overlapping part of the window
    let step = chunk_size_chars.saturating_sub(overlap_chars).max(1);

    while start_char < total_chars {
        let end_char = (start_char + chunk_size_chars).min(total_chars);

        // Convert character indices to byte indices for string slicing
        let start_byte: usize = chars[..start_char].iter().map(|c| c.len_utf8()).sum();
        let mut end_byte: usize = chars[..end_char].iter().map(|c| c.len_utf8()).sum();

        // Try to break at sentence or word boundary for cleaner chunks
        if end_char < total_chars {
            let slice = &text[start_byte..end_byte];
            // Look for sentence boundary (period followed by space)
            if let Some(last_period) = slice.rfind(". ") {
                end_byte = start_byte + last_period + 2;
            } else if let Some(last_space) = slice.rfind(' ') {
                // Fall back to word boundary (space)
                end_byte = start_byte + last_space + 1;
            }
        }

        // Extract chunk
        chunks.push(text[start_byte..end_byte].to_string());

        if end_char >= total_chars {
            break;
        }

        // Move to next chunk with overlap (in character units)
        start_char += step;
    }

    info!("Created {} chunks from text", chunks.len());
    chunks
}

/// Cleans markdown output from LLM by removing thinking tags and code fences
///
/// # Arguments
/// * `markdown` - Raw markdown output from LLM
///
/// # Returns
/// Cleaned markdown string
pub fn clean_llm_markdown_output(markdown: &str) -> String {
    // Remove reasoning blocks (<think>/<thinking>/<reasoning>/<reflection>) using cached regex
    let without_thinking = THINKING_TAG_REGEX.replace_all(markdown, "");

    let trimmed = without_thinking.trim();

    strip_wrapping_code_fence(trimmed)
}

/// Strips a single Markdown code fence that wraps the ENTIRE output, tolerating any
/// language tag (```markdown, ```md, ```, ```json …) and trailing whitespace after the
/// closing fence. Only unwraps when the text both opens with a fence line and ends with
/// a closing fence, so a document that merely *contains* a fenced snippet is left intact.
fn strip_wrapping_code_fence(text: &str) -> String {
    let trimmed = text.trim();

    // Must start with a fence marker and contain at least a closing fence somewhere after it.
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }

    // Split off the opening fence line (```<optional-lang>).
    let Some(first_newline) = trimmed.find('\n') else {
        // Single line starting with ``` — nothing meaningful to unwrap.
        return trimmed.to_string();
    };
    let after_open = &trimmed[first_newline + 1..];

    // The closing fence is the last ``` in the remaining text; require it to be there
    // and require the tail after it to be only whitespace (i.e. the fence really wraps
    // the whole document, not an inner snippet).
    if let Some(close_idx) = after_open.rfind("```") {
        let (body, tail) = after_open.split_at(close_idx);
        let tail_after_fence = &tail[3..];
        if tail_after_fence.trim().is_empty() {
            return body.trim().to_string();
        }
    }

    // Couldn't confidently unwrap — return the trimmed text unchanged.
    trimmed.to_string()
}

/// Extracts meeting name from the first heading in markdown
///
/// # Arguments
/// * `markdown` - Markdown content
///
/// # Returns
/// Meeting name if found, None otherwise
pub fn extract_meeting_name_from_markdown(markdown: &str) -> Option<String> {
    let candidate = markdown
        .lines()
        .find(|line| line.starts_with("# "))
        .map(|line| line.trim_start_matches("# ").trim().to_string())?;

    if is_valid_meeting_title(&candidate) {
        Some(candidate)
    } else {
        None
    }
}

/// Upper bound on an auto-generated meeting title length. Anything longer is almost
/// certainly the model dumping a sentence/paragraph into the H1, not a title.
const MAX_MEETING_TITLE_LEN: usize = 120;

/// Guards the auto-rename path: a summary must not rename a meeting to a template
/// placeholder or other non-title junk. Rejects the literal `<Add Title here>`
/// placeholder, empty/whitespace titles, over-long strings, and anything still
/// carrying an unfilled template angle-bracket token (e.g. `<...>`, `[AI-Generated
/// Title]`) which signals the model echoed the template instead of writing a title.
fn is_valid_meeting_title(title: &str) -> bool {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().count() > MAX_MEETING_TITLE_LEN {
        return false;
    }
    // Unfilled template tokens: `<...>` (e.g. "<Add Title here>") or `[...]`
    // (e.g. "[AI-Generated Title]"). A real title has no angle-bracket placeholder.
    if trimmed.contains('<') && trimmed.contains('>') {
        return false;
    }
    let lower = trimmed.to_lowercase();
    const PLACEHOLDER_MARKERS: &[&str] = &[
        "add title here",
        "ai-generated title",
        "add a title",
        "meeting title here",
        "title here",
    ];
    if PLACEHOLDER_MARKERS.iter().any(|m| lower.contains(m)) {
        return false;
    }
    true
}

/// Generates a complete meeting summary with conditional chunking strategy
///
/// # Arguments
/// * `client` - Reqwest HTTP client
/// * `provider` - LLM provider to use
/// * `model_name` - Specific model name
/// * `api_key` - API key for the provider
/// * `text` - Full transcript text to summarize
/// * `custom_prompt` - Optional user-provided context
/// * `template_id` - Template identifier (e.g., "daily_standup", "standard_meeting"),
///   used for logging and cache identity even when `template_choice` is `DeriveAuto`.
/// * `template_choice` - Either a resolved [`Template`](crate::summary::templates::Template)
///   (`Fixed`) or a request to derive one from content mid-pipeline (`DeriveAuto`,
///   specs/0053 W3). Resolved AFTER the map/reduce collapse so a long meeting's
///   derivation reads the reduced text, not the raw transcript.
/// * `token_threshold` - Token limit for single-pass processing (default 4000)
/// * `ollama_endpoint` - Optional custom Ollama endpoint
/// * `custom_openai_endpoint` - Optional custom OpenAI-compatible endpoint
/// * `max_tokens` - Optional max tokens for completion (CustomOpenAI provider)
/// * `temperature` - Optional temperature (CustomOpenAI provider)
/// * `top_p` - Optional top_p (CustomOpenAI provider)
/// * `app_data_dir` - Optional app data directory (BuiltInAI provider)
/// * `cancellation_token` - Optional cancellation token to stop processing
/// * `summary_language` - Optional BCP-47 tag (e.g. "en-GB") to force summary output language
/// * `detected_transcript_language` - Optional detected transcript language BCP-47 tag
/// * `cached_english` - Optional previously-generated English summary to skip pass 1 when translating
/// * `user_notes` - Optional manual notes the user took during the meeting; when
///   present and non-empty they are injected into the FINAL synthesis pass as
///   high-priority, authoritative grounding (notes inform content; the template
///   still governs structure). Kept whole — never chunked per-transcript-chunk.
/// * `transcript_has_speakers` - true when `text` is speaker-attributed (each line
///   prefixed with a resolved display name from diarization, specs/0010). When set,
///   the FINAL synthesis pass also gets [`SPEAKER_ATTRIBUTION_INSTRUCTIONS`] so the
///   model attributes points/action items to who said them; when false the prompt
///   is unchanged from today (no speaker mention, no hallucination risk).
/// * `role_preamble` - Optional "Participant roles" block (built by
///   [`build_role_preamble`] from `speakers.person_id → people.role`, specs/0012 Task 5).
///   When `Some` and non-empty it is prepended to the final transcript block AND
///   [`ROLE_WEIGHTING_INSTRUCTIONS`] is appended to the FINAL system prompt so the model
///   weights emphasis/ownership by role (never inclusion). When `None`/blank the prompt
///   is byte-identical to the no-role path. Only meaningful when
///   `transcript_has_speakers` is true (roles key on speaker display names).
///
/// # Returns
/// Tuple of `(final_summary_markdown, english_summary_markdown, chunk_accounting, derived_outline)`:
/// - `english_summary_markdown` is the canonical AI-generated English summary
///   (equals `final_summary_markdown` when the target language is English).
/// - `chunk_accounting` reports how many map-pass chunks were attempted, processed,
///   and **failed** (dropped after retries). When `failed > 0` the report is
///   *partial* — the caller MUST surface that to the user rather than presenting it
///   as complete. Single-pass and cached summaries report `ChunkAccounting::single_pass()`.
/// - `derived_outline` is `Some` only for `DeriveAuto` when reached (not the cached-English
///   fast path) and derivation succeeded (`Outline::derived`) — never on fallback (specs/0053 W3, I2).
#[allow(clippy::too_many_arguments)]
pub async fn generate_meeting_summary(
    client: &Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    text: &str,
    custom_prompt: &str,
    template_id: &str,
    template_choice: TemplateChoice<'_>,
    token_threshold: usize,
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: Option<&CancellationToken>,
    summary_language: Option<&str>,
    detected_transcript_language: Option<&str>,
    cached_english: Option<&str>,
    user_notes: Option<&str>,
    transcript_has_speakers: bool,
    role_preamble: Option<&str>,
) -> Result<(String, String, ChunkAccounting, Option<Outline>), String> {
    if let Some(token) = cancellation_token {
        if token.is_cancelled() {
            return Err("Summary generation was cancelled".to_string());
        }
    }
    info!(
        "Starting summary generation with provider: {:?}, model: {}",
        provider, model_name
    );

    let total_tokens = rough_token_count(text);
    info!("Transcript length: {} tokens", total_tokens);

    let (mut english_markdown, accounting, derived_outline) = if let Some(cached) =
        resolve_cached_english(cached_english, summary_language)
    {
        info!(
            "✓ Using cached English summary ({} chars), skipping pass 1",
            cached.len()
        );
        (cached.to_string(), ChunkAccounting::single_pass(), None)
    } else {
        let content_to_summarize: String;
        let accounting: ChunkAccounting;

        // Strategy: single-pass when the transcript fits the provider's context
        // threshold (sized per-model by the service); otherwise chunk + reduce. This
        // now applies to EVERY provider — a long meeting on a small-context cloud model
        // takes the chunking path instead of silently overflowing.
        if use_single_pass(total_tokens, token_threshold) {
            info!(
                "Using single-pass summarization (tokens: {}, threshold: {})",
                total_tokens, token_threshold
            );
            content_to_summarize = text.to_string();
            accounting = ChunkAccounting::single_pass();
        } else {
            info!(
                "Using multi-level summarization (tokens: {} exceeds threshold: {})",
                total_tokens, token_threshold
            );

            // Reserve 300 tokens for prompt overhead
            let chunks = chunk_text(text, token_threshold - 300, 100);
            let num_chunks = chunks.len();
            info!("Split transcript into {} chunks", num_chunks);

            let mut chunk_summaries = Vec::new();
            let mut failed_chunks: i64 = 0;
            let system_prompt_chunk = "You are an expert meeting summarizer.";

            for (i, chunk) in chunks.iter().enumerate() {
                // Check for cancellation before processing each chunk
                if let Some(token) = cancellation_token {
                    if token.is_cancelled() {
                        info!(
                            "Summary generation cancelled during chunk {}/{}",
                            i + 1,
                            num_chunks
                        );
                        return Err("Summary generation was cancelled".to_string());
                    }
                }

                info!("Processing chunk {}/{}", i + 1, num_chunks);
                let user_prompt_chunk = build_chunk_summary_user_prompt(chunk);

                // Bounded retry with backoff for retryable failures before giving up.
                match generate_summary_with_retry(
                    client,
                    provider,
                    model_name,
                    api_key,
                    system_prompt_chunk,
                    &user_prompt_chunk,
                    ollama_endpoint,
                    custom_openai_endpoint,
                    max_tokens.or(Some(crate::summary::llm_client::CHUNK_MAP_MAX_TOKENS)),
                    temperature,
                    top_p,
                    app_data_dir,
                    cancellation_token,
                )
                .await
                {
                    Ok(summary) => {
                        chunk_summaries.push(summary);
                        info!("✓ Chunk {}/{} processed successfully", i + 1, num_chunks);
                    }
                    Err(e) => {
                        // Cancellation is terminal — propagate it, don't count as a drop.
                        if e.contains("cancelled") {
                            return Err(e);
                        }
                        // A failed chunk is NOT silently skipped: count it so the outcome
                        // reflects that this slice of transcript is missing from the report.
                        failed_chunks += 1;
                        error!(
                            "Gave up on chunk {}/{} after retries: {} (this section will be missing from the summary)",
                            i + 1, num_chunks, e
                        );
                    }
                }
            }

            if chunk_summaries.is_empty() {
                return Err(
                    "Multi-level summarization failed: No chunks were processed successfully."
                        .to_string(),
                );
            }

            let processed = chunk_summaries.len() as i64;
            accounting = ChunkAccounting {
                total: num_chunks as i64,
                processed,
                failed: failed_chunks,
            };
            if failed_chunks > 0 {
                warn!(
                    "Partial summary: {}/{} chunks processed, {} dropped after retries",
                    processed, num_chunks, failed_chunks
                );
            } else {
                info!("Successfully processed all {} chunks", num_chunks);
            }

            // Combine chunk summaries with a hierarchical (recursive) reduce so the
            // combined text can't itself overflow a small context window.
            content_to_summarize = if chunk_summaries.len() > 1 {
                info!(
                    "Combining {} chunk summaries into cohesive summary",
                    chunk_summaries.len()
                );
                combine_chunk_summaries_recursive(
                    client,
                    provider,
                    model_name,
                    api_key,
                    chunk_summaries,
                    token_threshold,
                    ollama_endpoint,
                    custom_openai_endpoint,
                    max_tokens,
                    temperature,
                    top_p,
                    app_data_dir,
                    cancellation_token,
                    0,
                )
                .await?
            } else {
                chunk_summaries.remove(0)
            };
        }

        // specs/0053 W3: Auto derives its section list HERE, after the map/reduce collapse
        // (a long meeting reads the reduced text, not the transcript) — below is identical either way.
        let (template, derived_outline) = match template_choice {
            TemplateChoice::Fixed(t) => (t.clone(), None),
            TemplateChoice::DeriveAuto => {
                let outline = crate::summary::outline::derive_outline(
                    client,
                    provider,
                    model_name,
                    api_key,
                    &content_to_summarize,
                    ollama_endpoint,
                    custom_openai_endpoint,
                    app_data_dir,
                    cancellation_token,
                )
                .await;
                let template = crate::summary::outline::to_template(&outline);
                // specs/0053 I2: don't persist a fallback as if it were real —
                // that would pin the meeting to it forever; retry next run.
                (template, outline.derived.then_some(outline))
            }
        };

        info!(
            "Generating final markdown report with template: {}",
            template_id
        );

        // Generate markdown structure and section instructions using template methods
        let clean_template_markdown = template.to_markdown_structure();
        let section_instructions = template.to_section_instructions();

        // Notes-aware summary (spec 0003 pivot): when the user took their own
        // notes, fold the anti-hallucination grounding instructions into the
        // FINAL system prompt and include the notes WHOLE in the final user
        // prompt. The notes are deliberately NOT injected per-chunk — they
        // belong to the synthesis pass so the model sees them against the full
        // (combined) transcript content, mirroring the retired enhance flow.
        let user_notes = user_notes.filter(|n| !n.trim().is_empty());

        // Role-weighting (specs/0012 Task 5): only active when a non-blank preamble was
        // supplied. Normalizing to None here guarantees the no-role path is byte-identical
        // to today (no system-prompt append, no transcript prepend).
        let role_preamble = role_preamble.filter(|p| !p.trim().is_empty());

        if user_notes.is_some() {
            info!("Notes-aware summary: injecting user notes into final synthesis pass");
        }
        if transcript_has_speakers {
            info!("Speaker-aware summary: transcript carries speaker labels; enabling attribution");
        }
        if role_preamble.is_some() {
            info!("Role-weighted summary: per-speaker roles present; enabling role weighting");
        }
        // specs/0044 WS2: depth guidance keyed on the ORIGINAL transcript size
        // (not the chunk-reduced text), so a long meeting that went through
        // map-reduce still asks for a detailed report.
        let final_system_prompt = build_final_synthesis_system_prompt(
            &section_instructions,
            &clean_template_markdown,
            &length_guidance(total_tokens),
            user_notes.is_some(),
            transcript_has_speakers,
            role_preamble.is_some(),
        );

        // Prepend the participant-role preamble to the transcript block so the model sees
        // who-is-who before reading the attributed lines (specs/0012 Task 5). The no-role
        // path skips this entirely, keeping the transcript block byte-identical to today.
        let content_to_summarize = match role_preamble {
            Some(preamble) => format!("{preamble}\n\n{content_to_summarize}"),
            None => content_to_summarize,
        };

        let mut final_user_prompt =
            format!("<transcript_chunks>\n{content_to_summarize}\n</transcript_chunks>\n");

        if let Some(notes) = user_notes {
            final_user_prompt.push_str(&build_user_notes_block(notes));
        }

        if !custom_prompt.is_empty() {
            final_user_prompt.push_str("\n\nUser Provided Context:\n\n<user_context>\n");
            final_user_prompt.push_str(custom_prompt);
            final_user_prompt.push_str("\n</user_context>");
        }

        // Check cancellation before final summary generation
        if let Some(token) = cancellation_token {
            if token.is_cancelled() {
                info!("Summary generation cancelled before final summary");
                return Err("Summary generation was cancelled".to_string());
            }
        }

        let raw_markdown = generate_summary_with_retry(
            client,
            provider,
            model_name,
            api_key,
            &final_system_prompt,
            &final_user_prompt,
            ollama_endpoint,
            custom_openai_endpoint,
            max_tokens,
            temperature,
            top_p,
            app_data_dir,
            cancellation_token,
        )
        .await?;

        let english_markdown = clean_llm_markdown_output(&raw_markdown);
        info!("Summary pass completed ({} chars)", english_markdown.len());

        (english_markdown, accounting, derived_outline)
    };

    let final_markdown = match resolve_final_language_action(
        summary_language,
        detected_transcript_language,
    ) {
        FinalLanguageAction::Translate(name) => {
            match translate_markdown(
                client,
                provider,
                model_name,
                api_key,
                &english_markdown,
                name,
                ollama_endpoint,
                custom_openai_endpoint,
                max_tokens,
                temperature,
                top_p,
                app_data_dir,
                cancellation_token,
            )
            .await
            {
                Ok(translated) => translated,
                Err(e) => return Err(format!("Translation to {} failed: {}", name, e)),
            }
        }
        FinalLanguageAction::NormalizeEnglish => {
            info!(
                "English target with detected transcript language {:?}; running soft English normalization",
                detected_transcript_language
            );
            let normalized = english_markdown_after_normalization_result(
                &english_markdown,
                normalize_markdown_to_english(
                    client,
                    provider,
                    model_name,
                    api_key,
                    &english_markdown,
                    ollama_endpoint,
                    custom_openai_endpoint,
                    max_tokens,
                    temperature,
                    top_p,
                    app_data_dir,
                    cancellation_token,
                )
                .await,
            )?;
            english_markdown = normalized.clone();
            normalized
        }
        FinalLanguageAction::ReturnEnglish => english_markdown.clone(),
    };

    if accounting.is_complete() {
        info!("Summary generation completed successfully");
    } else {
        warn!(
            "Summary generation completed PARTIALLY: {}/{} chunks in report ({} dropped)",
            accounting.processed, accounting.total, accounting.failed
        );
    }
    Ok((final_markdown, english_markdown, accounting, derived_outline))
}

/// Hierarchical (recursive) reduce of chunk summaries into a single combined summary.
///
/// The old combine step joined every chunk summary and ran a single `generate_summary`,
/// which itself could overflow a small context window when there were many chunks. This
/// batches summaries into groups that fit under `token_threshold`, combines each group,
/// then recurses on the (now fewer) group summaries until one remains. Depth is capped by
/// [`MAX_COMBINE_DEPTH`]; if still not reduced to one at the cap, a final combine pass runs
/// over whatever remains (best-effort).
#[allow(clippy::too_many_arguments)]
async fn combine_chunk_summaries_recursive(
    client: &Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    summaries: Vec<String>,
    token_threshold: usize,
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: Option<&CancellationToken>,
    depth: usize,
) -> Result<String, String> {
    if summaries.len() <= 1 {
        return Ok(summaries.into_iter().next().unwrap_or_default());
    }

    if let Some(token) = cancellation_token {
        if token.is_cancelled() {
            return Err("Summary generation was cancelled".to_string());
        }
    }

    let system_prompt_combine = "You are an expert at synthesizing meeting summaries.";

    // Reserve headroom for the combine prompt scaffolding.
    let budget = token_threshold.saturating_sub(300).max(1);

    // Group consecutive summaries into batches whose combined token estimate fits the
    // budget. A single summary that alone exceeds the budget becomes its own batch
    // (it can't be reduced further here; the combine pass will still run on it).
    let mut batches: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut current_tokens = 0usize;
    for summary in summaries {
        let summary_tokens = rough_token_count(&summary);
        if !current.is_empty() && current_tokens + summary_tokens > budget {
            batches.push(std::mem::take(&mut current));
            current_tokens = 0;
        }
        current_tokens += summary_tokens;
        current.push(summary);
    }
    if !current.is_empty() {
        batches.push(current);
    }

    // If everything fit in one batch (or we've hit the depth cap), do the final combine.
    if batches.len() <= 1 || depth >= MAX_COMBINE_DEPTH {
        let combined_text = batches
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n---\n");
        let user_prompt_combine = build_combine_summary_user_prompt(&combined_text);
        return generate_summary_with_retry(
            client,
            provider,
            model_name,
            api_key,
            system_prompt_combine,
            &user_prompt_combine,
            ollama_endpoint,
            custom_openai_endpoint,
            max_tokens,
            temperature,
            top_p,
            app_data_dir,
            cancellation_token,
        )
        .await;
    }

    info!(
        "Hierarchical combine (depth {}): reducing {} summaries in {} batch(es)",
        depth,
        batches.len(),
        batches.len()
    );

    // Reduce each batch to one summary, then recurse on the reduced set.
    let mut reduced: Vec<String> = Vec::with_capacity(batches.len());
    for batch in batches {
        if batch.len() == 1 {
            reduced.push(batch.into_iter().next().unwrap());
            continue;
        }
        let combined_text = batch.join("\n---\n");
        let user_prompt_combine = build_combine_summary_user_prompt(&combined_text);
        let batch_summary = generate_summary_with_retry(
            client,
            provider,
            model_name,
            api_key,
            system_prompt_combine,
            &user_prompt_combine,
            ollama_endpoint,
            custom_openai_endpoint,
            max_tokens,
            temperature,
            top_p,
            app_data_dir,
            cancellation_token,
        )
        .await?;
        reduced.push(batch_summary);
    }

    Box::pin(combine_chunk_summaries_recursive(
        client,
        provider,
        model_name,
        api_key,
        reduced,
        token_threshold,
        ollama_endpoint,
        custom_openai_endpoint,
        max_tokens,
        temperature,
        top_p,
        app_data_dir,
        cancellation_token,
        depth + 1,
    ))
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_markdown_transform(
    client: &Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
    failure_label: &str,
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: Option<&CancellationToken>,
) -> Result<String, String> {
    if let Some(token) = cancellation_token {
        if token.is_cancelled() {
            return Err("Summary generation was cancelled".to_string());
        }
    }

    let raw = generate_summary_with_retry(
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
    )
    .await
    .map_err(|e| format!("{failure_label} failed: {e}"))?;

    Ok(clean_llm_markdown_output(&raw))
}

#[allow(clippy::too_many_arguments)]
async fn translate_markdown(
    client: &Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    english_markdown: &str,
    target_language: &str,
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: Option<&CancellationToken>,
) -> Result<String, String> {
    info!("Translation pass: target language = {}", target_language);

    let system_prompt = translation_system_prompt(target_language);
    let user_prompt = format!(
        "Translate the following Markdown document into {target_language}. Return ONLY the translated Markdown, nothing else.\n\n<document>\n{english_markdown}\n</document>"
    );

    run_markdown_transform(
        client,
        provider,
        model_name,
        api_key,
        &system_prompt,
        &user_prompt,
        "Translation pass",
        ollama_endpoint,
        custom_openai_endpoint,
        max_tokens,
        temperature,
        top_p,
        app_data_dir,
        cancellation_token,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn normalize_markdown_to_english(
    client: &Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    markdown: &str,
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: Option<&CancellationToken>,
) -> Result<String, String> {
    info!("English normalization pass: preserving Markdown structure");

    let user_prompt = format!(
        "Convert the following Markdown document into English. Return ONLY the English Markdown, nothing else.\n\n<document>\n{markdown}\n</document>"
    );

    run_markdown_transform(
        client,
        provider,
        model_name,
        api_key,
        english_normalization_system_prompt(),
        &user_prompt,
        "English normalization pass",
        ollama_endpoint,
        custom_openai_endpoint,
        max_tokens,
        temperature,
        top_p,
        app_data_dir,
        cancellation_token,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    // Notes-only summary degrade (spec 0015 Phase B, Task 6) -------------------

    #[test]
    fn empty_transcript_routes_to_single_pass() {
        // A notes-only meeting has an empty transcript -> 0 tokens, which is below
        // any positive threshold. This must select the single-pass branch (which
        // runs the notes-grounding final synthesis) and NOT the chunking branch
        // (chunk_text on "" returns zero chunks -> the summary would fail).
        let empty_tokens = rough_token_count("");
        assert_eq!(empty_tokens, 0);
        assert!(use_single_pass(empty_tokens, 4000));
    }

    #[test]
    fn long_transcript_chunks_regardless_of_provider() {
        // A transcript over the provider's (service-sized) context threshold takes the
        // chunking path for EVERY provider now — cloud providers are no longer forced
        // single-pass against a bogus "unlimited" threshold.
        let over_threshold = 5000;
        assert!(!use_single_pass(over_threshold, 4000));
        // Under threshold -> single pass.
        assert!(use_single_pass(3000, 4000));
    }

    // Speaker-attributed transcript (specs/0010 P2 Task 9) ---------------------

    /// Mirrors the no-speaker assembly the DB read does today
    /// (`get_full_transcript`): trim each segment, drop blanks, join with "\n".
    /// Used to assert the speaker-less path is byte-identical to today.
    fn legacy_join(texts: &[&str]) -> String {
        texts
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn speaker_attributed_transcript_prefixes_each_segment_when_diarized() {
        let segments = vec![
            (Some("You".to_string()), "Let's start.".to_string()),
            (
                Some("Priya".to_string()),
                "I'll own the report.".to_string(),
            ),
            (Some("Speaker 2".to_string()), "Sounds good.".to_string()),
        ];
        let (text, has_speakers) = build_speaker_attributed_transcript(&segments);

        assert!(has_speakers);
        assert_eq!(
            text,
            "You: Let's start.\nPriya: I'll own the report.\nSpeaker 2: Sounds good."
        );
    }

    #[test]
    fn speaker_less_transcript_is_byte_identical_to_legacy_join() {
        // No segment resolves a speaker -> no diarization. Output must equal the
        // existing speaker-less assembly exactly (no prefixes, no "Unknown:").
        let segments = vec![
            (None, "Let's start.".to_string()),
            (None, "  trailing space  ".to_string()),
            (None, "".to_string()), // blank dropped, like get_full_transcript
            (None, "Final point.".to_string()),
        ];
        let (text, has_speakers) = build_speaker_attributed_transcript(&segments);

        assert!(!has_speakers);
        assert_eq!(
            text,
            legacy_join(&["Let's start.", "  trailing space  ", "", "Final point."])
        );
        // Explicit: no attribution noise leaked in.
        assert!(!text.contains("Unknown"));
        assert!(!text.contains(": "));
    }

    #[test]
    fn partial_diarization_only_prefixes_resolved_segments() {
        // Mixed: some segments labeled, some not (e.g. a turn with no overlap).
        let segments = vec![
            (Some("You".to_string()), "Hello.".to_string()),
            (None, "Unattributed line.".to_string()),
        ];
        let (text, has_speakers) = build_speaker_attributed_transcript(&segments);

        assert!(has_speakers);
        assert_eq!(text, "You: Hello.\nUnattributed line.");
    }

    #[test]
    fn blank_speaker_name_falls_back_to_unprefixed_line() {
        // A whitespace-only display_name must not produce a ": " prefix.
        let segments = vec![(Some("   ".to_string()), "No real name.".to_string())];
        let (text, has_speakers) = build_speaker_attributed_transcript(&segments);

        assert!(!has_speakers);
        assert_eq!(text, "No real name.");
    }

    // Role-weighted summary (specs/0012 Tasks 5 & 6) ---------------------------
    //
    // Eval approach (Task 6): role-weighting is a PROMPT change, so the regression
    // guard is a prompt-assembly assertion, not a live-LLM call. We assert (a) the
    // preamble formats/empties correctly, (b) the ROLE_WEIGHTING_INSTRUCTIONS carry
    // the "emphasis not omission / never omit" guardrail, and (c) the final synthesis
    // system prompt CONTAINS the weighting block iff roles are present — and is
    // byte-identical to the no-role prompt when they are absent (an acceptance
    // criterion). Subjective "does it actually improve the summary" lives in the
    // human eyeball pass on the fixtures, not in CI.

    #[test]
    fn build_role_preamble_empty_returns_none() {
        assert_eq!(build_role_preamble(&[]), None);
    }

    #[test]
    fn build_role_preamble_all_blank_returns_none() {
        // A speaker with a blank name or blank role contributes no line; if none
        // remain, the whole preamble is None (no empty header leaks into the prompt).
        let roles = vec![
            ("   ".to_string(), "CEO".to_string()),
            ("Priya".to_string(), "  ".to_string()),
        ];
        assert_eq!(build_role_preamble(&roles), None);
    }

    #[test]
    fn build_role_preamble_formats_compact_block() {
        let roles = vec![
            ("Priya".to_string(), "CEO".to_string()),
            ("You".to_string(), "meeting owner".to_string()),
        ];
        let preamble = build_role_preamble(&roles).expect("roles present");
        assert_eq!(
            preamble,
            "Participant roles (for weighting, not for filtering):\n- Priya — CEO\n- You — meeting owner"
        );
    }

    #[test]
    fn english_base_instruction_marks_non_english_prose_invalid_without_bloat() {
        assert!(ENGLISH_BASE_SUMMARY_INSTRUCTION.contains("non-English prose is invalid"));
        assert!(ENGLISH_BASE_SUMMARY_INSTRUCTION.len() <= 120);
    }

    #[test]
    fn english_target_with_english_transcript_skips_normalization() {
        assert_eq!(
            resolve_final_language_action(Some("en"), Some("en")),
            FinalLanguageAction::ReturnEnglish
        );
    }

    #[test]
    fn english_target_with_non_english_transcript_normalizes_to_english() {
        assert_eq!(
            resolve_final_language_action(Some("en"), Some("ja")),
            FinalLanguageAction::NormalizeEnglish
        );
    }

    #[test]
    fn english_target_with_unknown_transcript_normalizes_to_english() {
        assert_eq!(
            resolve_final_language_action(Some("en"), None),
            FinalLanguageAction::NormalizeEnglish
        );
    }

    #[test]
    fn non_english_target_uses_translation_flow() {
        assert_eq!(
            resolve_final_language_action(Some("fr"), Some("ja")),
            FinalLanguageAction::Translate("French")
        );
    }

    #[test]
    fn failed_english_normalization_falls_back_to_original_markdown() {
        assert_eq!(
            english_markdown_after_normalization_result(
                "# Original",
                Err("normalization failed".to_string())
            )
            .unwrap(),
            "# Original"
        );
    }

    #[test]
    fn cancelled_english_normalization_is_not_swallowed() {
        assert!(english_markdown_after_normalization_result(
            "# Original",
            Err("Summary generation was cancelled".to_string())
        )
        .is_err());
    }

    // resolve_cached_english matrix -------------------------------------------

    #[test]
    fn no_cache_no_language_returns_none() {
        assert_eq!(resolve_cached_english(None, None), None);
    }

    #[test]
    fn empty_cache_with_translation_target_returns_none() {
        assert_eq!(resolve_cached_english(Some(""), Some("fr")), None);
    }

    #[test]
    fn whitespace_only_cache_returns_none() {
        assert_eq!(resolve_cached_english(Some("   \n"), Some("fr")), None);
    }

    #[test]
    fn valid_cache_no_language_returns_none() {
        assert_eq!(resolve_cached_english(Some("body"), None), None);
    }

    #[test]
    fn valid_cache_english_target_returns_none() {
        assert_eq!(resolve_cached_english(Some("body"), Some("en")), None);
    }

    #[test]
    fn valid_cache_english_variant_returns_none() {
        // "en-GB" normalises to English — cache should not be used (re-run pass 1)
        assert_eq!(resolve_cached_english(Some("body"), Some("en-GB")), None);
    }

    #[test]
    fn valid_cache_french_target_returns_cache() {
        assert_eq!(
            resolve_cached_english(Some("body"), Some("fr")),
            Some("body")
        );
    }

    #[test]
    fn valid_cache_unknown_language_returns_none() {
        // Unknown code -> language_name_from_code returns None -> not a translation
        assert_eq!(
            resolve_cached_english(Some("body"), Some("zz-unknown")),
            None
        );
    }

    #[test]
    fn uppercase_translation_code_returns_cache() {
        assert_eq!(
            resolve_cached_english(Some("body"), Some("FR")),
            Some("body")
        );
    }

    #[test]
    fn uppercase_english_code_returns_none() {
        assert_eq!(resolve_cached_english(Some("body"), Some("EN")), None);
    }

    #[test]
    fn underscore_locale_variant_returns_none() {
        // OS locale APIs (notably macOS) may emit "en_GB" with underscore.
        assert_eq!(resolve_cached_english(Some("body"), Some("en_GB")), None);
    }

    // Script-aware token estimate (spec 0028) ---------------------------------



    #[test]
    fn chunk_text_dense_script_produces_more_chunks_than_latin_bug() {
        // A CJK transcript must be split into chunks whose *token* estimate stays under
        // the requested chunk size — the previous fixed 2.85 chars/token let each chunk
        // hold ~3x too many tokens. Assert every chunk's estimate is within budget.
        let cjk: String = "会議".repeat(2000); // 4000 dense chars
        let chunk_size_tokens = 500;
        let chunks = chunk_text(&cjk, chunk_size_tokens, 50);
        assert!(
            chunks.len() > 1,
            "dense text should split into multiple chunks"
        );
        for chunk in &chunks {
            assert!(
                rough_token_count(chunk) <= chunk_size_tokens + 50,
                "chunk exceeds token budget: {}",
                rough_token_count(chunk)
            );
        }
    }

    // clean_llm_markdown_output robustness (spec 0028) ------------------------

    #[test]
    fn clean_strips_generic_language_fence() {
        assert_eq!(
            clean_llm_markdown_output("```md\n# Title\nbody\n```"),
            "# Title\nbody"
        );
        assert_eq!(
            clean_llm_markdown_output("```\n# Title\nbody\n```"),
            "# Title\nbody"
        );
    }

    #[test]
    fn clean_strips_fence_with_trailing_whitespace() {
        assert_eq!(
            clean_llm_markdown_output("```markdown\n# Title\nbody\n```\n\n  "),
            "# Title\nbody"
        );
    }

    #[test]
    fn clean_strips_reasoning_variants() {
        assert_eq!(
            clean_llm_markdown_output("<reasoning>let me think</reasoning>\n# Title"),
            "# Title"
        );
        assert_eq!(
            clean_llm_markdown_output("<Thinking>\nstep 1\n</Thinking>\n# Title"),
            "# Title"
        );
    }

    #[test]
    fn clean_leaves_inner_fenced_snippet_intact() {
        // A report that merely CONTAINS a fenced code block must not be unwrapped.
        let doc = "# Report\n\nSee this code:\n```rust\nlet x = 1;\n```\n\nDone.";
        assert_eq!(clean_llm_markdown_output(doc), doc);
    }

    // extract_meeting_name_from_markdown title validation (spec 0028) ----------

    #[test]
    fn extract_name_accepts_real_title() {
        assert_eq!(
            extract_meeting_name_from_markdown("# Q3 Planning Sync\n## Notes\nfoo"),
            Some("Q3 Planning Sync".to_string())
        );
    }

    #[test]
    fn extract_name_rejects_placeholder_title() {
        assert_eq!(
            extract_meeting_name_from_markdown("# <Add Title here>\nbody"),
            None
        );
        assert_eq!(
            extract_meeting_name_from_markdown("# [AI-Generated Title]\nbody"),
            None
        );
        assert_eq!(
            extract_meeting_name_from_markdown("# Meeting Title Here\nbody"),
            None
        );
    }

    #[test]
    fn extract_name_rejects_empty_and_overlong() {
        assert_eq!(extract_meeting_name_from_markdown("# \nbody"), None);
        let long_title = format!("# {}", "word ".repeat(60));
        assert_eq!(extract_meeting_name_from_markdown(&long_title), None);
    }

    #[test]
    fn extract_name_rejects_angle_bracket_tokens() {
        assert_eq!(
            extract_meeting_name_from_markdown("# <placeholder>\nbody"),
            None
        );
    }

    // Retry classification + backoff (spec 0028) ------------------------------

    #[test]
    fn retryable_errors_are_transient_only() {
        assert!(is_retryable_llm_error(
            "LLM request timed out after 300 seconds"
        ));
        assert!(is_retryable_llm_error(
            "Failed to send request to LLM: connection refused"
        ));
        assert!(is_retryable_llm_error(
            "LLM API request failed: overloaded_error"
        ));
        assert!(is_retryable_llm_error(
            "LLM API request failed: 503 Service Unavailable"
        ));
    }

    #[test]
    fn terminal_errors_are_not_retried() {
        assert!(!is_retryable_llm_error("Summary generation was cancelled"));
        assert!(!is_retryable_llm_error("Invalid API key format"));
        assert!(!is_retryable_llm_error(
            "LLM API request failed: invalid_request_error"
        ));
        assert!(!is_retryable_llm_error("Unsupported LLM provider: foo"));
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_for_attempt(0), BASE_LLM_BACKOFF);
        assert_eq!(backoff_for_attempt(1), BASE_LLM_BACKOFF * 2);
        assert!(backoff_for_attempt(10) <= MAX_LLM_BACKOFF);
    }

    // ChunkAccounting contract (spec 0028) ------------------------------------

    #[test]
    fn single_pass_accounting_is_complete() {
        let a = ChunkAccounting::single_pass();
        assert_eq!(
            a,
            ChunkAccounting {
                total: 1,
                processed: 1,
                failed: 0
            }
        );
        assert!(a.is_complete());
    }

    #[test]
    fn partial_accounting_is_incomplete() {
        let a = ChunkAccounting {
            total: 10,
            processed: 8,
            failed: 2,
        };
        assert!(!a.is_complete());
    }
}
