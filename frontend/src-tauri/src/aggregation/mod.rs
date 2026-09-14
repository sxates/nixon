//! Cross-meeting aggregation engine (specs/0035).
//!
//! One reusable pipeline: scope selection → gather (FTS5 relevance + metadata
//! filters, summaries-first source policy) → budget-bounded packing →
//! map-reduce prompting → answer with per-meeting `[M#]` sources.
//!
//! Ask-AI is the first consumer; topic roll-ups (0013 3b) and pre-call prep
//! (0013 4b) later become new [`AggregationPrompt`] sets over this same engine
//! with zero new gather/chunk code. The engine performs no egress itself — the
//! LLM call is injected by the consumer (see `engine::run`), and gather is
//! read-only over existing tables (no migrations, nothing written).

pub mod commands;
pub mod engine;
pub mod gather;
pub mod prep;
pub mod prep_commands;
pub mod prep_jobs;
pub mod prompts;
pub mod rollup;
pub mod scope;

pub use commands::execute_ask_ai;
pub use engine::{
    estimate_run_tokens, run, AggregationAnswer, AggregationPrompt, SourceMeeting, Stage,
};
pub use gather::{gather, DocSource, GatherResult, MeetingDoc, DEFAULT_MAX_MEETINGS};
pub use prep::execute_pre_call_prep;
pub use prompts::{
    ask_ai_prompt, person_rollup_prompt, postprocess_citations, pre_call_prep_prompt,
};
pub use rollup::execute_person_rollup;
pub use scope::AggregationScope;
