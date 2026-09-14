//! Ask-AI IPC (specs/0035 task 5): the run command, the run registry, and the
//! `ask-ai-*` events.
//!
//! The command is a thin shell over a plain async function ([`execute_ask_ai`])
//! so the pipeline is testable from `tests/aggregation_engine.rs` without a
//! Tauri runtime. Provider posture (spec Design, decided 2026-07-03): Ask-AI
//! **follows the configured summary provider** — no silent local override.
//! There is no pre-send preview/consent step (removed 2026-07-04 by product
//! decision — configuring a cloud summary provider IS the egress consent); the
//! run's `sources` list gives post-hoc accountability for what was sent. The
//! LLM closure is the same retry/cancellation path summaries use
//! (`generate_summary_with_retry`).

use crate::aggregation::engine::{self, AggregationAnswer, SourceMeeting, Stage};
use crate::aggregation::gather::{gather, DEFAULT_MAX_MEETINGS};
use crate::aggregation::prompts::{ask_ai_prompt, postprocess_citations};
use crate::aggregation::scope::AggregationScope;
use crate::database::repositories::ask_ai_history::{AskAiHistoryEntry, AskAiHistoryRepository};
use crate::database::repositories::saved_question::{SavedQuestion, SavedQuestionsRepository};
use crate::database::repositories::setting::SettingsRepository;
use crate::llm_activity::{LlmActivityState, Origin, TaskKind};
use crate::state::AppState;
use crate::summary::llm_gate::{with_priority, Priority};
use crate::summary::processor::generate_summary_with_retry;
use crate::summary::provider_config::resolve_provider_config;
use crate::summary::resolve_context_budget;
use anyhow::{anyhow, Context};
use once_cell::sync::Lazy;
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Run registry (mirrors the summary service's cancellation registry)
// ---------------------------------------------------------------------------

/// Active Ask-AI runs by `run_id`. Same shape as `SummaryService`'s
/// `CANCELLATION_REGISTRY` (summary/service.rs) — a process-lifetime static
/// rather than an `AppState` field, matching the `api_cancel_summary` pattern.
static ASK_AI_RUNS: Lazy<Mutex<HashMap<String, CancellationToken>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Registers a fresh token for `run_id` and returns it.
fn register_run(run_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    ASK_AI_RUNS
        .lock()
        .expect("ask-ai run registry poisoned")
        .insert(run_id.to_string(), token.clone());
    token
}

/// Drops the registry entry once a run finishes (success, failure, or after a
/// cancellation was observed). Idempotent.
fn finish_run(run_id: &str) {
    ASK_AI_RUNS
        .lock()
        .expect("ask-ai run registry poisoned")
        .remove(run_id);
}

/// Fires the run's cancellation token and cleans the registry entry.
/// Returns whether an active run was found.
pub fn cancel_ask_ai_run(run_id: &str) -> bool {
    let token = ASK_AI_RUNS
        .lock()
        .expect("ask-ai run registry poisoned")
        .remove(run_id);
    match token {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Provider resolution (the decided posture: follow the summary provider)
// ---------------------------------------------------------------------------

/// The configured summary provider/model pair Ask-AI follows (spec Design:
/// "Ask-AI follows the configured summary provider — no silent local override").
pub(crate) async fn configured_summary_model(
    pool: &SqlitePool,
) -> anyhow::Result<(String, String)> {
    let setting = SettingsRepository::get_model_config(pool)
        .await
        .context("Failed to read the summary model settings")?
        .ok_or_else(|| {
            anyhow!(
                "No summary model is configured — choose a provider and model in \
                 Settings → Summary, then ask again."
            )
        })?;
    Ok((setting.provider, setting.model))
}

// ---------------------------------------------------------------------------
// Run pipeline
// ---------------------------------------------------------------------------

/// The full Ask-AI pipeline over an injected LLM closure: gather (with a
/// `Gathering` progress tick) → Ask-AI prompt pair → [`engine::run`] →
/// citation post-processing. The LLM stays injected so the integration tests
/// drive the whole pipeline with a fake; `api_ask_ai_run` injects the real
/// provider path.
pub async fn execute_ask_ai<F, Fut, P>(
    pool: &SqlitePool,
    question: &str,
    scope: &AggregationScope,
    llm: F,
    budget_tokens: usize,
    cancel: &CancellationToken,
    on_progress: P,
) -> anyhow::Result<AggregationAnswer>
where
    F: Fn(String, String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
    P: Fn(Stage),
{
    on_progress(Stage::Gathering);
    let gathered = gather(pool, question, scope, DEFAULT_MAX_MEETINGS).await?;

    let prompt = ask_ai_prompt(question);
    let mut answer = engine::run(
        llm,
        &prompt,
        &gathered.docs,
        budget_tokens,
        cancel,
        on_progress,
    )
    .await?;

    answer.markdown = postprocess_citations(&answer.markdown, &mut answer.sources);
    Ok(answer)
}

// ---------------------------------------------------------------------------
// Events (kebab-case names, camelCase payloads — transcription-* conventions)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AskAiProgressPayload {
    run_id: String,
    /// "gathering" | "mapping" | "reducing".
    stage: &'static str,
    /// 1-based doc index during "mapping"; 0 for the other stages.
    current: usize,
    /// Total docs during "mapping"; 0 for the other stages.
    total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AskAiCompletePayload {
    run_id: String,
    answer_markdown: String,
    sources: Vec<SourceMeeting>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AskAiErrorPayload {
    run_id: String,
    message: String,
    /// True when the failure was a user cancellation, so the UI returns to
    /// idle instead of showing an error state.
    cancelled: bool,
}

fn progress_payload(run_id: &str, stage: Stage) -> AskAiProgressPayload {
    let (stage, current, total) = match stage {
        Stage::Gathering => ("gathering", 0, 0),
        Stage::Mapping { current, total } => ("mapping", current, total),
        Stage::Reducing => ("reducing", 0, 0),
    };
    AskAiProgressPayload {
        run_id: run_id.to_string(),
        stage,
        current,
        total,
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Starts an Ask-AI run and returns its `run_id` immediately. Progress and the
/// result arrive as events keyed by that id:
/// - `ask-ai-progress` `{ runId, stage, current, total }`
/// - `ask-ai-complete` `{ runId, answerMarkdown, sources }`
/// - `ask-ai-error`    `{ runId, message, cancelled }`
///
/// Configuration problems (no model chosen, missing API key) fail the command
/// synchronously — before a run id exists or anything is spawned.
#[tauri::command]
pub async fn api_ask_ai_run<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    question: String,
    scope: AggregationScope,
) -> Result<String, String> {
    let pool = state.db_manager.pool().clone();

    // Resolve the full provider config up-front (same resolver the summary run
    // uses) so "no API key configured" etc. surface as the command's error.
    let (provider_name, model) = configured_summary_model(&pool)
        .await
        .map_err(|e| format!("{e:#}"))?;
    let provider_config = resolve_provider_config(&pool, &provider_name, &model)
        .await
        .map_err(|e| format!("{e:#}"))?;

    // Captured for the fire-and-forget history write (WS2.a) — the provider/model this run
    // followed. Cloned here because `provider_name`/`model` are not otherwise moved into the
    // task, and `provider_config` is.
    let history_provider = provider_name.clone();
    let history_model = model.clone();

    let run_id = uuid::Uuid::new_v4().to_string();
    let cancel = register_run(&run_id);
    // BuiltInAI needs the app-data dir to find its sidecar models.
    let app_data_dir = app.path().app_data_dir().ok();

    info!(
        "ask-ai: run {} starting (provider {:?}, model {})",
        run_id, provider_config.provider, provider_config.model_name
    );

    let task_run_id = run_id.clone();
    // specs/0056 W2: a person is waiting — every LLM call in this task preempts background
    // work on the local model instead of queueing behind it.
    tauri::async_runtime::spawn(with_priority(Priority::Interactive, async move {
        // Budget from the shared sizing logic (Ollama metadata / BuiltInAI
        // registry / conservative cloud thresholds).
        let budget_tokens = resolve_context_budget(
            &provider_config.provider,
            &provider_config.model_name,
            provider_config.ollama_endpoint.as_deref(),
        )
        .await;

        let client = reqwest::Client::new();

        // The injected LLM closure: the real provider path with the same
        // bounded retry + cancellation behavior summaries get. Clones are
        // per-call (cheap: Arc-backed client, small strings) because the
        // closure must stay `Fn`, not `FnOnce`.
        let llm_config = provider_config.clone();
        let llm_client = client.clone();
        let llm_cancel = cancel.clone();
        let llm_app_data_dir = app_data_dir.clone();
        let llm = move |system: String, user: String| {
            let cfg = llm_config.clone();
            let client = llm_client.clone();
            let cancel = llm_cancel.clone();
            let app_data_dir = llm_app_data_dir.clone();
            async move {
                generate_summary_with_retry(
                    &client,
                    &cfg.provider,
                    &cfg.model_name,
                    &cfg.api_key,
                    &system,
                    &user,
                    cfg.ollama_endpoint.as_deref(),
                    cfg.custom_openai_endpoint.as_deref(),
                    cfg.custom_openai_max_tokens,
                    cfg.custom_openai_temperature,
                    cfg.custom_openai_top_p,
                    app_data_dir.as_ref(),
                    Some(&cancel),
                )
                .await
            }
        };

        let progress_app = app.clone();
        let progress_run_id = task_run_id.clone();
        let on_progress = move |stage: Stage| {
            if let Err(e) =
                progress_app.emit("ask-ai-progress", progress_payload(&progress_run_id, stage))
            {
                warn!("ask-ai: failed to emit progress for run {progress_run_id}: {e}");
            }
        };

        // specs/0056 W2: visible to diagnostics as a foreground task (never surfaced in the
        // sidebar — the Ask page has its own progress UI), so a stuck run shows up somewhere.
        let activity = app
            .try_state::<LlmActivityState>()
            .map(|state| Arc::clone(&state.0))
            .map(|registry| registry.start(TaskKind::AskAI, Origin::Foreground, "Ask AI"));

        let result = execute_ask_ai(
            &pool,
            &question,
            &scope,
            llm,
            budget_tokens,
            &cancel,
            on_progress,
        )
        .await;

        if let Some(task) = activity {
            task.finish(result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
        }

        match result {
            Ok(answer) => {
                info!(
                    "ask-ai: run {} complete ({} source(s), {} cited)",
                    task_run_id,
                    answer.sources.len(),
                    answer.sources.iter().filter(|s| s.cited).count()
                );

                // WS2.a: persist this completed run to `ask_ai_history` — FIRE-AND-FORGET.
                // Spawned onto its own task so it can NEVER block or fail the run: a DB error
                // is logged and swallowed; the answer event below fires regardless. Serialize
                // before the payload move consumes `answer`. (0034 background-write discipline.)
                let history_scope_json =
                    serde_json::to_string(&scope).unwrap_or_else(|_| "{}".to_string());
                let history_sources_json =
                    serde_json::to_string(&answer.sources).unwrap_or_else(|_| "[]".to_string());
                let history_pool = pool.clone();
                let history_question = question.clone();
                let history_answer = answer.markdown.clone();
                let history_provider = history_provider.clone();
                let history_model = history_model.clone();
                let history_run_id = task_run_id.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = AskAiHistoryRepository::insert(
                        &history_pool,
                        &history_question,
                        &history_scope_json,
                        &history_answer,
                        &history_sources_json,
                        Some(&history_provider),
                        Some(&history_model),
                    )
                    .await
                    {
                        warn!("ask-ai: failed to persist history for run {history_run_id} (non-fatal): {e}");
                    }
                });

                let payload = AskAiCompletePayload {
                    run_id: task_run_id.clone(),
                    answer_markdown: answer.markdown,
                    sources: answer.sources,
                };
                if let Err(e) = app.emit("ask-ai-complete", payload) {
                    warn!("ask-ai: failed to emit completion for run {task_run_id}: {e}");
                }
            }
            Err(e) => {
                let message = format!("{e:#}");
                // specs/0056 W2: only the token decides — see `engine::map_llm_error`.
                let cancelled = cancel.is_cancelled();
                if cancelled {
                    info!("ask-ai: run {} cancelled", task_run_id);
                } else {
                    warn!("ask-ai: run {} failed: {}", task_run_id, message);
                }
                let payload = AskAiErrorPayload {
                    run_id: task_run_id.clone(),
                    message,
                    cancelled,
                };
                if let Err(e) = app.emit("ask-ai-error", payload) {
                    warn!("ask-ai: failed to emit error for run {task_run_id}: {e}");
                }
            }
        }

        finish_run(&task_run_id);
    }));

    Ok(run_id)
}

/// Fires the run's cancellation token and cleans the registry. Returns whether
/// an active run was found (an unknown/finished id is not an error — the run
/// may have completed while the click was in flight).
#[tauri::command]
pub async fn api_cancel_ask_ai(run_id: String) -> Result<bool, String> {
    let cancelled = cancel_ask_ai_run(&run_id);
    if cancelled {
        info!("ask-ai: cancellation requested for run {run_id}");
    } else {
        info!("ask-ai: no active run {run_id} to cancel");
    }
    Ok(cancelled)
}

// ---------------------------------------------------------------------------
// Ask-AI history + saved questions (specs/0038 WS2.a / WS2.b)
// ---------------------------------------------------------------------------

/// Most-recent-first Ask-AI history (WS2.a). `limit` caps the page (repo default when `None`).
#[tauri::command]
pub async fn api_list_ask_ai_history(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<AskAiHistoryEntry>, String> {
    let pool = state.db_manager.pool();
    AskAiHistoryRepository::list(pool, limit)
        .await
        .map_err(|e| format!("Failed to load Ask-AI history: {e}"))
}

/// Deletes one history entry (WS2.a). Unknown ids are not an error (`false` = nothing removed).
#[tauri::command]
pub async fn api_delete_ask_ai_history(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let pool = state.db_manager.pool();
    AskAiHistoryRepository::delete(pool, &id)
        .await
        .map_err(|e| format!("Failed to delete the history entry: {e}"))
}

/// Stars the current question + scope (WS2.b). `scope_json` is the serialized
/// [`AggregationScope`] the frontend already has, stored verbatim so a re-run round-trips it.
#[tauri::command]
pub async fn api_create_saved_question(
    state: tauri::State<'_, AppState>,
    label: String,
    question: String,
    scope_json: String,
) -> Result<SavedQuestion, String> {
    let pool = state.db_manager.pool();
    SavedQuestionsRepository::create(pool, &label, &question, &scope_json)
        .await
        .map_err(|e| format!("Failed to save the question: {e}"))
}

/// All saved questions, most-recently-created first (WS2.b).
#[tauri::command]
pub async fn api_list_saved_questions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SavedQuestion>, String> {
    let pool = state.db_manager.pool();
    SavedQuestionsRepository::list(pool)
        .await
        .map_err(|e| format!("Failed to load saved questions: {e}"))
}

/// Deletes one saved question (WS2.b). Unknown ids are not an error.
#[tauri::command]
pub async fn api_delete_saved_question(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let pool = state.db_manager.pool();
    SavedQuestionsRepository::delete(pool, &id)
        .await
        .map_err(|e| format!("Failed to delete the saved question: {e}"))
}

/// One saved question by id (specs/0038 #4). The saved-question sub-page loads this to render the
/// CACHED answer without re-running the LLM. `None` (returns `null`) when the id is unknown.
#[tauri::command]
pub async fn api_get_saved_question(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<SavedQuestion>, String> {
    let pool = state.db_manager.pool();
    SavedQuestionsRepository::get(pool, &id)
        .await
        .map_err(|e| format!("Failed to load the saved question: {e}"))
}

/// Caches the answer from a Rerun on the saved-question sub-page (specs/0038 #4): stores the
/// answer markdown + sources JSON and stamps `last_run_at`. Unknown ids are not an error.
#[tauri::command]
pub async fn api_update_saved_question_answer(
    state: tauri::State<'_, AppState>,
    id: String,
    answer_markdown: String,
    sources_json: String,
) -> Result<bool, String> {
    let pool = state.db_manager.pool();
    SavedQuestionsRepository::update_answer(pool, &id, &answer_markdown, &sources_json)
        .await
        .map_err(|e| format!("Failed to cache the answer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_registry_register_cancel_finish() {
        let token = register_run("run-1");
        assert!(!token.is_cancelled());

        // Cancel fires the token and removes the entry.
        assert!(cancel_ask_ai_run("run-1"));
        assert!(token.is_cancelled());
        assert!(!cancel_ask_ai_run("run-1"), "second cancel finds nothing");

        // finish_run is idempotent and safe on unknown ids.
        let token = register_run("run-2");
        finish_run("run-2");
        finish_run("run-2");
        assert!(!token.is_cancelled(), "finish never cancels");
        assert!(!cancel_ask_ai_run("run-2"));
    }

    #[test]
    fn progress_payloads_match_the_event_contract() {
        let p = progress_payload("r", Stage::Gathering);
        assert_eq!((p.stage, p.current, p.total), ("gathering", 0, 0));
        let p = progress_payload(
            "r",
            Stage::Mapping {
                current: 3,
                total: 6,
            },
        );
        assert_eq!((p.stage, p.current, p.total), ("mapping", 3, 6));
        let p = progress_payload("r", Stage::Reducing);
        assert_eq!((p.stage, p.current, p.total), ("reducing", 0, 0));

        // camelCase payload keys (frontend consumes these verbatim).
        let json = serde_json::to_value(progress_payload("run-9", Stage::Reducing)).unwrap();
        assert_eq!(json["runId"], "run-9");
        assert_eq!(json["stage"], "reducing");
        assert!(json.get("current").is_some() && json.get("total").is_some());
    }

    #[test]
    fn event_payloads_serialize_camel_case() {
        let complete = AskAiCompletePayload {
            run_id: "r1".into(),
            answer_markdown: "answer [M1]".into(),
            sources: vec![SourceMeeting {
                meeting_id: "m1".into(),
                title: "Kickoff".into(),
                created_at: "2026-07-01T10:00:00Z".into(),
                cited: true,
            }],
        };
        let json = serde_json::to_value(&complete).unwrap();
        assert_eq!(json["runId"], "r1");
        assert_eq!(json["answerMarkdown"], "answer [M1]");
        assert_eq!(json["sources"][0]["meetingId"], "m1");
        assert_eq!(json["sources"][0]["createdAt"], "2026-07-01T10:00:00Z");
        assert_eq!(json["sources"][0]["cited"], true);

        let error = AskAiErrorPayload {
            run_id: "r1".into(),
            message: "boom".into(),
            cancelled: true,
        };
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["runId"], "r1");
        assert_eq!(json["cancelled"], true);
    }
}
