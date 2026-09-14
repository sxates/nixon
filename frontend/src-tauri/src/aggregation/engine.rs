//! The reusable aggregation primitive (specs/0035): budget-bounded packing +
//! map-reduce over per-meeting docs, with cancellation and stage progress.
//!
//! The engine makes **zero direct provider calls** — the LLM is an injected
//! async closure `(system, user) -> Result<String, String>`, which is the whole
//! "consumers are just prompts" contract and what makes map-reduce logic unit
//! testable without HTTP. Ask-AI (task 4/5), topic roll-ups (0013 3b), and
//! pre-call prep (0013 4b) are each an [`AggregationPrompt`] over this same
//! `run`.

use crate::aggregation::gather::MeetingDoc;
use crate::summary::processor::{chunk_text, clean_llm_markdown_output, rough_token_count};
use serde::{Deserialize, Serialize};
use std::future::Future;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Token headroom reserved for prompt scaffolding beyond what we count
/// explicitly — mirrors the summary pipeline's 300-token reserve.
const PROMPT_OVERHEAD_RESERVE: usize = 300;

/// Chunk overlap (tokens) for the oversized-single-doc pre-reduce, matching the
/// summary pipeline's chunk loop.
const CHUNK_OVERLAP_TOKENS: usize = 100;

/// A consumer of the engine IS this struct plus a scope: the map prompt runs
/// once per meeting doc (extraction), the reduce prompt runs once over the
/// numbered map outputs (or the numbered raw docs when everything fits in a
/// single pass). Boxed closures so consumers can capture the question or other
/// runtime state (a bare `fn` pointer couldn't).
pub struct AggregationPrompt {
    pub map_system: String,
    /// Builds the map-stage user prompt for one meeting doc. The engine tags
    /// each map *output* with its `[M#]` meeting number — the closure doesn't
    /// need to (and shouldn't) number anything itself.
    pub map_user: Box<dyn Fn(&MeetingDoc) -> String + Send + Sync>,
    pub reduce_system: String,
    /// Builds the reduce-stage user prompt from the numbered doc block
    /// (single-pass) or the numbered map outputs (map-reduce).
    pub reduce_user: Box<dyn Fn(&str) -> String + Send + Sync>,
}

/// Stage-level progress, surfaced to the frontend as `ask-ai-progress` events
/// by the IPC layer (task 5). `Gathering` is emitted by the command wrapper
/// before `run` is entered (gather happens outside the engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Gathering,
    Mapping { current: usize, total: usize },
    Reducing,
}

/// One meeting that was sent to the model, with whether the answer cited it.
/// Uncited sources still surface in the UI ("searched but not cited") so egress
/// is fully accounted for. Serialized camelCase for the `ask-ai-complete` event
/// payload and the egress preview (specs/0035 IPC).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMeeting {
    pub meeting_id: String,
    pub title: String,
    pub created_at: String,
    pub cited: bool,
}

impl SourceMeeting {
    /// The one doc→source mapping: `sources[i]` mirrors `docs[i]` (and thereby
    /// marker `[M{i+1}]`).
    pub fn from_doc(doc: &MeetingDoc, cited: bool) -> Self {
        Self {
            meeting_id: doc.meeting_id.clone(),
            title: doc.title.clone(),
            created_at: doc.created_at.clone(),
            cited,
        }
    }
}

/// The engine's raw output: the reduce stage's markdown (with `[M#]` markers as
/// the model emitted them — validation/stripping is the consumer's citation
/// post-processing, task 4) plus the doc-numbered source list. `sources[i]`
/// corresponds to marker `[M{i+1}]`; `cited` is a simple marker-presence check
/// the consumer may refine.
#[derive(Debug, Clone)]
pub struct AggregationAnswer {
    pub markdown: String,
    pub sources: Vec<SourceMeeting>,
}

/// Runs the aggregation over the gathered docs.
///
/// Packing: the numbered doc block is counted with `rough_token_count`; when
/// `Σdocs + prompt overhead < budget_tokens` the reduce prompt runs once over
/// the raw docs (single pass). Otherwise each doc gets a map call (extraction),
/// and the reduce runs over the numbered map outputs. A single doc exceeding
/// the budget (the raw-transcript fallback case) is pre-reduced with
/// `chunk_text` + per-chunk map before joining the meeting-level reduce.
///
/// Cancellation is checked between LLM calls (like the summary chunk loop);
/// an in-flight provider request cannot be aborted — same accepted limitation
/// as summary cancellation. Errors are user-readable `anyhow` errors;
/// cancellation errors contain "cancelled" (matching the summary convention).
pub async fn run<F, Fut, P>(
    llm: F,
    prompt: &AggregationPrompt,
    docs: &[MeetingDoc],
    budget_tokens: usize,
    cancel: &CancellationToken,
    on_progress: P,
) -> anyhow::Result<AggregationAnswer>
where
    F: Fn(String, String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
    P: Fn(Stage),
{
    anyhow::ensure!(
        !docs.is_empty(),
        "No meetings matched the question. Broaden the scope (date range, person) or rephrase the question."
    );

    let doc_block = numbered_doc_block(docs);

    let reduce_input = if fits_single_pass(prompt, &doc_block, budget_tokens) {
        info!(
            "aggregation: single-pass over {} doc(s) (budget {} tokens)",
            docs.len(),
            budget_tokens
        );
        doc_block
    } else {
        info!(
            "aggregation: map-reduce over {} doc(s) (budget {} tokens)",
            docs.len(),
            budget_tokens
        );
        map_docs(&llm, prompt, docs, budget_tokens, cancel, &on_progress).await?
    };

    check_cancelled(cancel)?;
    on_progress(Stage::Reducing);

    // specs/0056 W1: strip in-band `<think>…</think>` blocks and a wrapping code fence
    // the way the summary path does, so a model that reasons in-band never leaks it into
    // the answer (or, at map stage below, into the reduce input).
    let answer = clean_llm_markdown_output(
        &llm(
            prompt.reduce_system.clone(),
            (prompt.reduce_user)(&reduce_input),
        )
        .await
        .map_err(|e| map_llm_error("composing the answer", e, cancel))?,
    );
    // The wire layer already rejects an empty raw reply; a reply that was ONLY an in-band
    // `<think>` block or a bare code fence becomes empty here instead, and must fail the
    // same way rather than reach the UI as a blank answer (specs/0056 W1).
    anyhow::ensure!(
        !answer.trim().is_empty(),
        "The model returned only reasoning and no answer. Try again, or pick a model without extended thinking."
    );

    let sources = docs
        .iter()
        .enumerate()
        // Default [M#]-presence `cited` check; refined by the consumer's
        // citation post-processing (task 4). "[M1]" cannot false-positive on
        // "[M10]" because the closing bracket is part of the needle.
        .map(|(i, doc)| SourceMeeting::from_doc(doc, answer.contains(&marker(i))))
        .collect();

    Ok(AggregationAnswer {
        markdown: answer,
        sources,
    })
}

/// Map stage: one LLM call per doc (or per chunk for an oversized doc), each
/// output tagged with its meeting number in doc order.
async fn map_docs<F, Fut, P>(
    llm: &F,
    prompt: &AggregationPrompt,
    docs: &[MeetingDoc],
    budget_tokens: usize,
    cancel: &CancellationToken,
    on_progress: &P,
) -> anyhow::Result<String>
where
    F: Fn(String, String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
    P: Fn(Stage),
{
    let total = docs.len();
    let map_budget = budget_tokens
        .saturating_sub(rough_token_count(&prompt.map_system) + PROMPT_OVERHEAD_RESERVE)
        .max(1);

    let mut map_outputs = Vec::with_capacity(total);

    for (i, doc) in docs.iter().enumerate() {
        check_cancelled(cancel)?;
        on_progress(Stage::Mapping {
            current: i + 1,
            total,
        });

        let full_text = doc.full_text();
        let stage = format!("reading meeting {} of {}", i + 1, total);

        let output = if rough_token_count(&full_text) > map_budget {
            // Oversized single doc (raw-transcript fallback case): pre-reduce
            // with the summary pipeline's chunker + a per-chunk map, then join.
            let chunks = chunk_text(&full_text, map_budget, CHUNK_OVERLAP_TOKENS);
            info!(
                "aggregation: doc {} exceeds the map budget; pre-reducing {} chunk(s)",
                doc.meeting_id,
                chunks.len()
            );
            let mut parts = Vec::with_capacity(chunks.len());
            for chunk in chunks {
                check_cancelled(cancel)?;
                let chunk_doc = MeetingDoc {
                    text: chunk,
                    excerpt: None, // already folded into full_text
                    ..doc.clone()
                };
                let part = llm(prompt.map_system.clone(), (prompt.map_user)(&chunk_doc))
                    .await
                    .map_err(|e| map_llm_error(&stage, e, cancel))?;
                parts.push(clean_llm_markdown_output(&part));
            }
            parts.join("\n")
        } else {
            clean_llm_markdown_output(
                &llm(prompt.map_system.clone(), (prompt.map_user)(doc))
                    .await
                    .map_err(|e| map_llm_error(&stage, e, cancel))?,
            )
        };

        map_outputs.push(numbered_section(i, &doc.title, &doc.created_at, &output));
    }

    Ok(map_outputs.join("\n\n"))
}

/// The `[M#]` marker for the doc at 0-based index `i`.
fn marker(i: usize) -> String {
    format!("[M{}]", i + 1)
}

/// THE definition of the numbered-section wire format: an `[M#] "title" (date)`
/// header line over the body. Both producers ([`numbered_doc_block`] for the
/// single-pass reduce input, [`map_docs`] for map outputs) build sections here,
/// and `prompts::is_marker_header` parses exactly this header shape — change
/// them together.
pub(crate) fn numbered_section(index: usize, title: &str, created_at: &str, body: &str) -> String {
    format!("{} \"{title}\" ({created_at})\n{body}", marker(index))
}

/// Builds the numbered doc block (single-pass reduce input): each doc headed by
/// its `[M#]` marker, title, and date, so citations resolve without map outputs.
fn numbered_doc_block(docs: &[MeetingDoc]) -> String {
    docs.iter()
        .enumerate()
        .map(|(i, doc)| numbered_section(i, &doc.title, &doc.created_at, &doc.full_text()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Tokens the fully-built single-pass reduce prompt (over the numbered raw
/// docs) would occupy, including the fixed overhead reserve.
fn single_pass_tokens(prompt: &AggregationPrompt, doc_block: &str) -> usize {
    rough_token_count(&prompt.reduce_system)
        + rough_token_count(&(prompt.reduce_user)(doc_block))
        + PROMPT_OVERHEAD_RESERVE
}

/// The packing decision: single-pass when the fully-built reduce prompt (over
/// the numbered raw docs) plus reserve fits the budget.
fn fits_single_pass(prompt: &AggregationPrompt, doc_block: &str, budget_tokens: usize) -> bool {
    single_pass_tokens(prompt, doc_block) < budget_tokens
}

/// Rough token estimate for what a `run` over these docs would send: the packed
/// (numbered) docs inside the reduce prompt, plus the same fixed overhead
/// reserve `run` itself budgets with.
///
/// This is exactly the number `fits_single_pass` compares against the budget.
/// When map-reduce kicks in the true total is larger (per-doc map prompts) —
/// accepted roughness (specs/0035). Zero docs estimate zero — nothing would be
/// sent.
pub fn estimate_run_tokens(prompt: &AggregationPrompt, docs: &[MeetingDoc]) -> usize {
    if docs.is_empty() {
        return 0;
    }
    single_pass_tokens(prompt, &numbered_doc_block(docs))
}

fn check_cancelled(cancel: &CancellationToken) -> anyhow::Result<()> {
    anyhow::ensure!(!cancel.is_cancelled(), "Aggregation was cancelled");
    Ok(())
}

/// specs/0056 W2: cancellation is decided by the run's own token, never by the substring
/// "cancelled" in a provider error — that inference let a real failure (e.g. the BuiltInAI
/// "Generation cancelled during sidecar startup") reach the Ask page as a *silent* cancel.
fn map_llm_error(stage: &str, e: String, cancel: &CancellationToken) -> anyhow::Error {
    if cancel.is_cancelled() {
        anyhow::anyhow!("Aggregation was cancelled")
    } else {
        anyhow::anyhow!("The model failed while {stage}: {e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregation::gather::DocSource;
    use std::sync::{Arc, Mutex};

    fn doc(id: &str, title: &str, text: &str) -> MeetingDoc {
        MeetingDoc {
            meeting_id: id.to_string(),
            title: title.to_string(),
            created_at: "2026-07-01T10:00:00Z".to_string(),
            source: DocSource::Summary,
            text: text.to_string(),
            excerpt: None,
        }
    }

    fn test_prompt() -> AggregationPrompt {
        AggregationPrompt {
            map_system: "map-system".to_string(),
            map_user: Box::new(|d: &MeetingDoc| format!("MAP:{}\n{}", d.meeting_id, d.full_text())),
            reduce_system: "reduce-system".to_string(),
            reduce_user: Box::new(|block: &str| format!("REDUCE\n{block}")),
        }
    }

    /// Fake LLM: records every (system, user) call, answers from a canned queue
    /// (last answer repeats), optionally cancelling a token after each call.
    struct FakeLlm {
        calls: Arc<Mutex<Vec<(String, String)>>>,
        answer: String,
        cancel_after_first_call: Option<CancellationToken>,
    }

    impl FakeLlm {
        fn new(answer: &str) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                answer: answer.to_string(),
                cancel_after_first_call: None,
            }
        }

        fn closure(
            &self,
        ) -> impl Fn(String, String) -> std::pin::Pin<Box<dyn Future<Output = Result<String, String>>>>
        {
            let calls = self.calls.clone();
            let answer = self.answer.clone();
            let cancel = self.cancel_after_first_call.clone();
            move |system: String, user: String| {
                let calls = calls.clone();
                let answer = answer.clone();
                let cancel = cancel.clone();
                Box::pin(async move {
                    calls.lock().unwrap().push((system, user));
                    if let Some(token) = &cancel {
                        token.cancel();
                    }
                    Ok(answer)
                })
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn collect_progress() -> (Arc<Mutex<Vec<Stage>>>, impl Fn(Stage)) {
        let stages = Arc::new(Mutex::new(Vec::new()));
        let sink = stages.clone();
        (stages, move |stage| sink.lock().unwrap().push(stage))
    }

    #[test]
    fn packing_decision_single_pass_vs_map_reduce() {
        let prompt = test_prompt();
        let docs = vec![
            doc("m1", "One", "short doc"),
            doc("m2", "Two", "also short"),
        ];
        let block = numbered_doc_block(&docs);

        // Small docs, generous budget → single pass.
        assert!(fits_single_pass(&prompt, &block, 10_000));
        // Same docs, budget below the fixed reserve → map-reduce.
        assert!(!fits_single_pass(&prompt, &block, PROMPT_OVERHEAD_RESERVE));
    }

    #[test]
    fn numbered_doc_block_tags_docs_in_order() {
        let docs = vec![doc("m1", "Kickoff", "alpha"), doc("m2", "Retro", "beta")];
        let block = numbered_doc_block(&docs);
        assert!(block.contains("[M1] \"Kickoff\""));
        assert!(block.contains("[M2] \"Retro\""));
        let m1 = block.find("[M1]").unwrap();
        let m2 = block.find("[M2]").unwrap();
        assert!(m1 < m2, "docs must be numbered in doc order");
    }

    #[tokio::test]
    async fn single_pass_makes_one_reduce_call_and_sets_cited_flags() {
        let fake = FakeLlm::new("Both agreed [M1]. Nothing else.");
        let prompt = test_prompt();
        let docs = vec![doc("m1", "Kickoff", "alpha"), doc("m2", "Retro", "beta")];
        let cancel = CancellationToken::new();
        let (stages, on_progress) = collect_progress();

        let answer = run(
            fake.closure(),
            &prompt,
            &docs,
            100_000,
            &cancel,
            on_progress,
        )
        .await
        .unwrap();

        let calls = fake.calls();
        assert_eq!(calls.len(), 1, "single pass = exactly one LLM call");
        assert_eq!(calls[0].0, "reduce-system");
        assert!(calls[0].1.contains("[M1] \"Kickoff\""));
        assert!(calls[0].1.contains("[M2] \"Retro\""));

        assert_eq!(answer.sources.len(), 2);
        assert!(answer.sources[0].cited);
        assert!(!answer.sources[1].cited);
        assert_eq!(answer.sources[1].meeting_id, "m2");

        assert_eq!(*stages.lock().unwrap(), vec![Stage::Reducing]);
    }

    #[tokio::test]
    async fn map_reduce_maps_each_doc_then_reduces_over_numbered_outputs() {
        let fake = FakeLlm::new("relevant stuff [M2]");
        let prompt = test_prompt();
        // Each doc ~105 tokens: a 450-token budget can't fit both plus the
        // 300-token reserve in one pass, but each doc fits its own map call
        // (map budget ≈ 450 − reserve ≈ 146 tokens) — no chunking.
        let body = "word ".repeat(60);
        let docs = vec![doc("m1", "Kickoff", &body), doc("m2", "Retro", &body)];
        let cancel = CancellationToken::new();
        let (stages, on_progress) = collect_progress();

        let answer = run(fake.closure(), &prompt, &docs, 450, &cancel, on_progress)
            .await
            .unwrap();

        let calls = fake.calls();
        assert_eq!(calls.len(), 3, "two map calls + one reduce call");
        assert_eq!(calls[0].0, "map-system");
        assert!(calls[0].1.starts_with("MAP:m1"));
        assert_eq!(calls[1].0, "map-system");
        assert!(calls[1].1.starts_with("MAP:m2"));
        assert_eq!(calls[2].0, "reduce-system");
        // The reduce input is the numbered map outputs, in doc order.
        assert!(calls[2].1.contains("[M1] \"Kickoff\""));
        assert!(calls[2].1.contains("[M2] \"Retro\""));

        assert!(!answer.sources[0].cited);
        assert!(answer.sources[1].cited);

        assert_eq!(
            *stages.lock().unwrap(),
            vec![
                Stage::Mapping {
                    current: 1,
                    total: 2
                },
                Stage::Mapping {
                    current: 2,
                    total: 2
                },
                Stage::Reducing,
            ]
        );
    }

    #[tokio::test]
    async fn oversized_doc_is_pre_reduced_with_chunked_map_calls() {
        let fake = FakeLlm::new("chunk output");
        let prompt = test_prompt();
        // One doc far over the budget → chunk_text pre-reduce path.
        let body = "word ".repeat(2000); // ~3500 tokens
        let docs = vec![doc("m1", "Marathon", &body)];
        let cancel = CancellationToken::new();

        let answer = run(fake.closure(), &prompt, &docs, 450, &cancel, |_| {})
            .await
            .unwrap();

        let calls = fake.calls();
        let map_calls = calls.iter().filter(|(s, _)| s == "map-system").count();
        let reduce_calls = calls.iter().filter(|(s, _)| s == "reduce-system").count();
        assert!(
            map_calls > 1,
            "oversized doc must be chunked into several map calls, got {map_calls}"
        );
        assert_eq!(reduce_calls, 1);
        assert_eq!(answer.sources.len(), 1);
    }

    #[tokio::test]
    async fn cancellation_between_map_calls_stops_the_run() {
        let mut fake = FakeLlm::new("mapped");
        let cancel = CancellationToken::new();
        fake.cancel_after_first_call = Some(cancel.clone());

        let prompt = test_prompt();
        let body = "word ".repeat(60);
        let docs = vec![doc("m1", "Kickoff", &body), doc("m2", "Retro", &body)];

        let err = run(fake.closure(), &prompt, &docs, 450, &cancel, |_| {})
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("cancelled"),
            "cancellation must surface as a 'cancelled' error, got: {err}"
        );
        assert_eq!(
            fake.calls().len(),
            1,
            "no further LLM calls after cancellation"
        );
    }

    #[tokio::test]
    async fn llm_failure_surfaces_a_user_readable_error() {
        let calls = Arc::new(Mutex::new(0usize));
        let calls2 = calls.clone();
        let llm = move |_system: String, _user: String| {
            let calls = calls2.clone();
            async move {
                *calls.lock().unwrap() += 1;
                Err::<String, String>("connection refused".to_string())
            }
        };
        let prompt = test_prompt();
        let docs = vec![doc("m1", "Kickoff", "alpha")];
        let cancel = CancellationToken::new();

        let err = run(llm, &prompt, &docs, 100_000, &cancel, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("connection refused"));
        assert!(err.to_string().contains("The model failed"));
    }

    #[tokio::test]
    async fn empty_docs_is_a_user_actionable_error() {
        let fake = FakeLlm::new("unused");
        let prompt = test_prompt();
        let cancel = CancellationToken::new();

        let err = run(fake.closure(), &prompt, &[], 100_000, &cancel, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("No meetings matched"));
        assert!(fake.calls().is_empty(), "no LLM call for an empty gather");
    }

    #[tokio::test]
    async fn cited_marker_check_does_not_false_positive_on_double_digits() {
        let fake = FakeLlm::new("only [M10] was relevant");
        let prompt = test_prompt();
        let docs: Vec<MeetingDoc> = (1..=10)
            .map(|i| doc(&format!("m{i}"), &format!("Meeting {i}"), "tiny"))
            .collect();
        let cancel = CancellationToken::new();

        let answer = run(fake.closure(), &prompt, &docs, 1_000_000, &cancel, |_| {})
            .await
            .unwrap();

        assert!(!answer.sources[0].cited, "[M1] must not match inside [M10]");
        assert!(answer.sources[9].cited);
    }
}
