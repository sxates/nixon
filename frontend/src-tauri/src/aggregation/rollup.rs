//! Person roll-up over the aggregation engine (specs/0038 WS5.b).
//!
//! A third consumer of the shared gather→pack→map-reduce engine, after Ask-AI
//! ([`execute_ask_ai`](crate::aggregation::commands::execute_ask_ai)) and pre-call
//! prep ([`execute_pre_call_prep`](crate::aggregation::prep::execute_pre_call_prep)).
//! Scoped to ONE person: it gathers that person's recent meetings (union of roster
//! and actually-spoke, via the person-scoped [`AggregationScope`]) term-less and
//! newest-first, then runs [`person_rollup_prompt`] to synthesize a short "recent
//! themes / open threads / last few meetings with {person}" brief with `[M#]`
//! citations.
//!
//! **On-demand only.** Nothing here is pre-generated or scheduled — briefs are
//! pre-generated only for imminent calendar events (specs/0036). This runs when the
//! user opens the panel.
//!
//! Provider posture matches Ask-AI: follow the configured summary provider, no
//! silent local override. `execute_person_rollup` keeps the LLM injected (testable
//! with a fake); the command awaits the result — a bounded roll-up over ≤10
//! summaries — rather than standing up a third streaming run-registry.

use crate::aggregation::commands::configured_summary_model;
use crate::aggregation::engine::{self, AggregationAnswer, SourceMeeting, Stage};
use crate::aggregation::gather::{gather, DEFAULT_MAX_MEETINGS};
use crate::aggregation::prompts::{person_rollup_prompt, postprocess_citations};
use crate::aggregation::scope::AggregationScope;
use crate::database::repositories::meeting::{MeetingsRepository, RecentPersonMeeting};
use crate::database::repositories::people::PeopleRepository;
use crate::state::AppState;
use crate::summary::llm_gate::{with_priority, Priority};
use crate::summary::processor::generate_summary_with_retry;
use crate::summary::provider_config::resolve_provider_config;
use crate::summary::resolve_context_budget;
use serde::Serialize;
use sqlx::SqlitePool;
use std::future::Future;
use tauri::{AppHandle, Manager, Runtime};
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Cap on the People directory "Recent with {person}" plain list.
const RECENT_MEETINGS_LIMIT: i64 = 20;

/// A synthesized person roll-up: the brief markdown plus its `[M#]` source mapping.
/// Same shape as the Ask-AI/prep answers, so the frontend reuses
/// `AskAI/AnswerMarkdown.tsx`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonRollup {
    pub markdown: String,
    pub sources: Vec<SourceMeeting>,
}

/// Generate a person roll-up over the person's recent meetings, using an injected
/// LLM closure.
///
/// Mirrors [`execute_ask_ai`](crate::aggregation::commands::execute_ask_ai) and
/// [`execute_pre_call_prep`](crate::aggregation::prep::execute_pre_call_prep):
/// `Gathering` progress tick → gather (person scope, term-less → pure newest-first
/// metadata fetch, summaries-first doc policy) → [`person_rollup_prompt`] →
/// [`engine::run`] → citation post-processing. No meetings with summarizable
/// content ⇒ a friendly "nothing to roll up yet" answer (short-circuited before
/// the engine, so its question-oriented empty-docs error never reaches the user).
pub async fn execute_person_rollup<F, Fut, P>(
    pool: &SqlitePool,
    person_id: &str,
    person_name: &str,
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

    // Person scope (union: on the roster OR actually spoke). The term-less question
    // degrades gather to a pure newest-first metadata fetch over the person's set.
    let scope = AggregationScope {
        person_id: Some(person_id.to_string()),
        ..Default::default()
    };
    let gathered = gather(pool, "", &scope, DEFAULT_MAX_MEETINGS).await?;

    // Nothing to summarize (the person may be on rosters but their meetings have
    // no summaries/notes to draw from). Short-circuit with a friendly empty state
    // instead of letting engine::run's empty-docs guard surface a question-oriented
    // "rephrase / broaden the scope" error that makes no sense for a person roll-up.
    if gathered.docs.is_empty() {
        return Ok(AggregationAnswer {
            markdown: format!(
                "Nothing recent to roll up for {person_name} yet — their recent meetings don't have summaries or notes to draw from."
            ),
            sources: Vec::new(),
        });
    }

    let prompt = person_rollup_prompt(person_name);
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
// Tauri commands
// ---------------------------------------------------------------------------

/// Synthesize the "recent themes / open threads / last few meetings with {person}"
/// roll-up on demand. Awaits the result (bounded over ≤10 summaries) rather than
/// streaming; the frontend shows a spinner while it runs. Follows the configured
/// summary provider (same posture as Ask-AI) — configuration problems (no model,
/// missing API key) surface as the command's error.
#[tauri::command]
pub async fn api_person_rollup<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    person_id: String,
) -> Result<PersonRollup, String> {
    let pool = state.db_manager.pool().clone();

    // The prompt is scoped by the person's display name.
    let person = PeopleRepository::get(&pool, &person_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Person not found".to_string())?;

    // Resolve the full provider config up-front (same resolver the summary run and
    // Ask-AI use) so "no model chosen" / "no API key" surface as the command error.
    let (provider_name, model) = configured_summary_model(&pool)
        .await
        .map_err(|e| format!("{e:#}"))?;
    let provider_config = resolve_provider_config(&pool, &provider_name, &model)
        .await
        .map_err(|e| format!("{e:#}"))?;

    let budget_tokens = resolve_context_budget(
        &provider_config.provider,
        &provider_config.model_name,
        provider_config.ollama_endpoint.as_deref(),
    )
    .await;

    info!(
        "person-rollup: person {} ({}), provider {:?}, model {}",
        person_id, person.display_name, provider_config.provider, provider_config.model_name
    );

    // BuiltInAI needs the app-data dir to find its sidecar models.
    let app_data_dir = app.path().app_data_dir().ok();
    let client = reqwest::Client::new();
    let cancel = CancellationToken::new();

    // The injected LLM closure: the real provider path with the same bounded retry
    // + cancellation behavior summaries and Ask-AI get. Clones are per-call (cheap:
    // Arc-backed client, small strings) so the closure stays `Fn`.
    let llm_cancel = cancel.clone();
    let llm = move |system: String, user: String| {
        let cfg = provider_config.clone();
        let client = client.clone();
        let cancel = llm_cancel.clone();
        let app_data_dir = app_data_dir.clone();
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

    // specs/0056 W2: the user is looking at a spinner — interactive priority.
    let answer = with_priority(
        Priority::Interactive,
        execute_person_rollup(
            &pool,
            &person_id,
            &person.display_name,
            llm,
            budget_tokens,
            &cancel,
            |_| {},
        ),
    )
    .await
    .map_err(|e| format!("{e:#}"))?;

    info!(
        "person-rollup: person {} complete ({} source(s), {} cited)",
        person_id,
        answer.sources.len(),
        answer.sources.iter().filter(|s| s.cited).count()
    );

    Ok(PersonRollup {
        markdown: answer.markdown,
        sources: answer.sources,
    })
}

/// The People directory "Recent with {person}" plain list — a cheap, instant metadata
/// read (no LLM), separate from the on-demand [`api_person_rollup`] synthesis.
#[tauri::command]
pub async fn api_recent_meetings_with_person(
    state: tauri::State<'_, AppState>,
    person_id: String,
) -> Result<Vec<RecentPersonMeeting>, String> {
    MeetingsRepository::recent_meetings_with_person(
        state.db_manager.pool(),
        &person_id,
        RECENT_MEETINGS_LIMIT,
    )
    .await
    .map_err(|e| format!("{e}"))
}
