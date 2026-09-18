//! The full background summary run: `process_transcript_background` (chunking, provider
//! calls, caching, persistence) plus its queue reporting (resolving the LLM-activity
//! registry, starting a `MeetingSummary` task, reporting its outcome).
//!
//! This whole ~500-line orchestration used to live in [`crate::summary::service`], but
//! that file sat at its file-size-ratchet ceiling (specs/0042 WS6) with no headroom for
//! the queue-reporting code this module adds, so the run moved here in one piece.
//! `service.rs` now holds what is left: cancellation tokens, context-budget resolution,
//! and template/cache/language helpers shared by callers outside the run itself. This is
//! the SAME split `diarization::launch` already made for the same reason (specs/0063 W3).
//!
//! `impl SummaryService` is declared a second time here — Rust allows multiple inherent
//! impl blocks for a type across modules in the same crate — so call sites
//! (`summary/commands.rs`) keep calling `SummaryService::process_transcript_background`
//! unchanged.
//!
//! The caller supplies an [`Origin`] on every call, because Rust has no way to tell an
//! interactive summary from an automatic one — that distinction lives entirely in which
//! frontend hook invoked the underlying `api_process_transcript` / `api_generate_summary`
//! command: `useSummaryGeneration.ts` (the Generate/Regenerate button, watched live via
//! `ChunkProgressDisplay`) wants `Origin::Foreground` so it never double-reports next to
//! that progress UI, while `useAutoGenerateSummary.ts` (auto-summary after a recording
//! stops) and `useDeferredBacklog.ts` (the backlog drain) want `Origin::Background` so the
//! footer queue shows them. `summary/commands.rs` is the actual source of truth for which
//! Rust call site gets which origin today.

use std::sync::Arc;
use std::time::Instant;

use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};
use tracing::{error, info, warn};

use crate::database::repositories::meeting_note::MeetingNotesRepository;
use crate::database::repositories::summary_outline::SummaryOutlineRepository;
use crate::database::repositories::{
    meeting::MeetingsRepository, people::PeopleRepository, summary::SummaryProcessesRepository,
};
use crate::llm_activity::{LlmActivityState, Origin, TaskKind};
use crate::summary::cache_key::{
    build_summary_cache_source, stable_text_fingerprint, strip_title_if_present,
    template_cache_fingerprint,
};
use crate::summary::llm_client::LLMProvider;
use crate::summary::outline::{Outline, TemplateChoice, AUTO_TEMPLATE_ID};
use crate::summary::processor::{
    build_role_preamble, extract_meeting_name_from_markdown, generate_meeting_summary,
};
use crate::summary::provider_config::resolve_provider_config;
use crate::summary::service::{
    build_summary_result_json, extract_cached_english_markdown, resolve_context_budget,
    SummaryService,
};
use crate::summary::templates::Template;

impl SummaryService {
    /// Processes transcript in the background and generates summary
    ///
    /// This function is designed to be spawned as an async task and does not block
    /// the main thread. It updates the database with progress and results.
    ///
    /// specs/0063 W3: also registers a `MeetingSummary` task on the LLM-activity
    /// registry for the duration of the run, tagged with the caller-supplied `origin`,
    /// so background runs (auto-summary-after-recording, the deferred-backlog drain)
    /// show up in the footer queue — previously none did, despite `TaskKind::
    /// MeetingSummary` existing since specs/0052 — while a foreground run (the user
    /// watching Generate/Regenerate with `ChunkProgressDisplay` on screen) is recorded
    /// too but filtered out of `view()`, so it is never double-reported. A missing
    /// registry (`try_state` returns `None`, e.g. in a test harness with no managed
    /// state) degrades to "no queue row"; it never blocks or fails the summary.
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
    /// * `origin` - `Origin::Foreground` for a run the user is watching live (has its
    ///   own progress UI); `Origin::Background` for anything unattended. The caller
    ///   decides — see `summary/commands.rs`'s two call sites.
    #[allow(clippy::too_many_arguments)] // cohesive param set; refactor deferred
    pub async fn process_transcript_background<R: tauri::Runtime>(
        app: AppHandle<R>,
        pool: SqlitePool,
        meeting_id: String,
        text: String,
        model_provider: String,
        model_name: String,
        custom_prompt: String,
        template_id: String,
        summary_language: Option<String>,
        origin: Origin,
    ) {
        // specs/0063 W3: register as work on the footer queue, tagged with the
        // caller's origin. `view()` filters to `Origin::Background`, so a foreground
        // run (already shown via `ChunkProgressDisplay`) is recorded for history/
        // failure-badge purposes but never rendered as a second running row.
        let registry = app
            .try_state::<LlmActivityState>()
            .map(|state| Arc::clone(&state.0));
        let task = match registry {
            Some(registry) => {
                let title = match MeetingsRepository::get_meeting_metadata(&pool, &meeting_id).await
                {
                    Ok(Some(meta)) => format!("Summarizing — {}", meta.title),
                    _ => "Summarizing".to_string(),
                };
                Some(registry.start_for(
                    TaskKind::MeetingSummary,
                    origin,
                    title,
                    Some(meeting_id.clone()),
                ))
            }
            None => None,
        };

        let result = Self::process_transcript_background_inner(
            app,
            pool,
            meeting_id,
            text,
            model_provider,
            model_name,
            custom_prompt,
            template_id,
            summary_language,
        )
        .await;

        if let Some(t) = task {
            // `Ok(Some(reason))` = deliberately not completed (user cancellation) —
            // distinct from both success and failure (registry.rs's `TaskOutcome::
            // Skipped`), so it must not paint a green "Success" row over a run the DB
            // recorded as cancelled, and must not raise the failure badge either.
            match result {
                Ok(Some(reason)) => t.finish_skipped(&reason),
                Ok(None) => t.finish(Ok(())),
                Err(e) => t.finish(Err(e)),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_transcript_background_inner<R: tauri::Runtime>(
        app: AppHandle<R>,
        pool: SqlitePool,
        meeting_id: String,
        mut text: String,
        model_provider: String,
        model_name: String,
        custom_prompt: String,
        template_id: String,
        summary_language: Option<String>,
    ) -> Result<Option<String>, String> {
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
                    let msg = format!("{e:#}");
                    Self::update_process_failed(&pool, &meeting_id, &msg).await;
                    return Err(msg);
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
            // Every fixed template: unchanged path, except a dangling id now
            // degrades to the default (specs/0061 W6) — see
            // `Self::resolve_fixed_template`. If even the default fails to resolve
            // (specs/0061 review, I4 — e.g. a corrupt custom override of
            // `standard_meeting`), fail the run the same way the pre-fallback code
            // did rather than panicking inside this spawned background task.
            (None, id) => match Self::resolve_fixed_template(&meeting_id, id) {
                Ok(template) => template,
                Err(e) => {
                    Self::update_process_failed(&pool, &meeting_id, &e).await;
                    return Err(e);
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
                    Err(format!("failed to save completed summary: {e}"))
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
                    Ok(None)
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
                    // A user-initiated cancellation is neither success nor failure — the
                    // caller reports it via `finish_skipped`, distinct from both.
                    Ok(Some("cancelled by the user".to_string()))
                } else {
                    Self::update_process_failed(&pool, &meeting_id, &e).await;
                    Err(e)
                }
            }
        }
    }
}
