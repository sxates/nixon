//! Pre-call prep pipeline over the aggregation engine (specs/0036).
//!
//! The prep brief is a second consumer of the exact same gather→pack→map-reduce
//! engine that powers Ask-AI ([`execute_ask_ai`](crate::aggregation::commands::execute_ask_ai)):
//! it pins a KNOWN set of prior occurrence ids (a recurring series) into the
//! scope and swaps in the [`pre_call_prep_prompt`]. There is no FTS relevance
//! search — a term-less question makes `gather` do a pure newest-first metadata
//! fetch over the pinned set (summaries-first doc policy still applies).
//!
//! Like the engine itself, this performs zero direct provider calls — the LLM is
//! the injected async closure — so the whole pipeline is testable with a fake.

use crate::aggregation::engine::{self, AggregationAnswer, Stage};
use crate::aggregation::gather::{gather, DEFAULT_MAX_MEETINGS};
use crate::aggregation::prompts::{postprocess_citations, pre_call_prep_prompt};
use crate::aggregation::scope::AggregationScope;
use std::future::Future;
use tokio_util::sync::CancellationToken;

/// Generate a pre-call-prep brief over a fixed set of prior occurrence ids.
///
/// Mirrors `execute_ask_ai`: `Gathering` progress tick → gather (pinned scope,
/// term-less) → [`pre_call_prep_prompt`] → [`engine::run`] → citation
/// post-processing. `prior_ids` should be the recurring series' recent prior
/// occurrences (newest first) from
/// [`MeetingsRepository::find_prior_series_occurrences`](crate::database::repositories::meeting::MeetingsRepository::find_prior_series_occurrences).
/// An empty `prior_ids` yields the engine's "no meetings" error — callers should
/// treat that as "no brief" and not invoke this.
pub async fn execute_pre_call_prep<F, Fut, P>(
    pool: &sqlx::SqlitePool,
    prior_ids: &[String],
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

    // Pinned scope: nothing outside `prior_ids` is ever selected. The term-less
    // question degrades gather to a pure newest-first metadata fetch over the set
    // (no FTS ranking), which is what we want for a fixed series.
    let scope = AggregationScope {
        meeting_ids: Some(prior_ids.to_vec()),
        ..Default::default()
    };
    let gathered = gather(pool, "", &scope, DEFAULT_MAX_MEETINGS).await?;

    let prompt = pre_call_prep_prompt();
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
