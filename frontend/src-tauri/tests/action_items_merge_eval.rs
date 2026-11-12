//! specs/0088 W2 (Task 7): merge precision/recall eval against a real model.
//!
//! `#[ignore]`d: the normal suite stays offline. Run with
//! `cargo test --features metal --test action_items_merge_eval -- --ignored --nocapture`.
//!
//! Env: `NIXON_EVAL_PROVIDER` (default `ollama`), `NIXON_EVAL_MODEL` (default
//! `gemma4:26b`), `NIXON_EVAL_ENDPOINT` (default `http://127.0.0.1:11434`; never
//! `localhost`, which can hang). Fixtures are invented. Each case has one tracked list
//! from "another meeting" and a one-task summary; `expect_same_as` is the 1-based tracked
//! number the candidate should merge into, or null. Two scores are reported: the raw
//! model `same_as`, and the routed outcome (which adds the Jaccard backstop). Precision
//! is gated at 0.9 (a false merge hides a task); recall is reported only.

use app_lib::action_items::extractor::{extract_candidates, resolve_candidates, OwnerIdentity};
use app_lib::action_items::merge::{route_candidates, TrackedItem};
use app_lib::summary::llm_client::LLMProvider;
use app_lib::summary::provider_config::ProviderConfig;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    tracked: Vec<String>,
    summary: String,
    expect_same_as: Vec<Option<u32>>,
}

#[derive(Default)]
struct Score {
    predicted: u32,
    correct: u32,
    expected: u32,
}

impl Score {
    fn add(&mut self, expected: Option<u32>, predicted: Option<u32>) {
        self.expected += expected.is_some() as u32;
        self.predicted += predicted.is_some() as u32;
        self.correct += (expected.is_some() && expected == predicted) as u32;
    }
    fn precision(&self) -> f64 {
        if self.predicted == 0 {
            1.0
        } else {
            self.correct as f64 / self.predicted as f64
        }
    }
    fn recall(&self) -> f64 {
        if self.expected == 0 {
            1.0
        } else {
            self.correct as f64 / self.expected as f64
        }
    }
}

#[tokio::test]
#[ignore = "needs a reachable model; see module docs"]
async fn merge_precision_recall() {
    let provider = std::env::var("NIXON_EVAL_PROVIDER").unwrap_or_else(|_| "ollama".into());
    let model = std::env::var("NIXON_EVAL_MODEL").unwrap_or_else(|_| "gemma4:26b".into());
    let endpoint =
        std::env::var("NIXON_EVAL_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let config = ProviderConfig {
        provider: LLMProvider::from_str(&provider).expect("provider"),
        model_provider: provider.clone(),
        model_name: model.clone(),
        api_key: std::env::var("NIXON_EVAL_API_KEY").unwrap_or_default(),
        ollama_endpoint: Some(endpoint),
        custom_openai_endpoint: None,
        custom_openai_max_tokens: None,
        custom_openai_temperature: None,
        custom_openai_top_p: None,
    };
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/merge_eval/pairs.json")).unwrap();
    let client = reqwest::Client::new();
    let owner = OwnerIdentity::default();
    let anchor = chrono::NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();

    let (mut raw, mut routed_score) = (Score::default(), Score::default());
    for case in &cases {
        let tracked: Vec<TrackedItem> = case
            .tracked
            .iter()
            .enumerate()
            .map(|(i, d)| TrackedItem {
                id: format!("t{}", i + 1),
                meeting_id: Some("prior".into()),
                description: d.clone(),
            })
            .collect();
        let cands = extract_candidates(
            &client,
            &config,
            None,
            &case.summary,
            None,
            &tracked,
            &[],
            &owner,
        )
        .await
        .unwrap_or_else(|e| panic!("{}: extraction failed: {e:#}", case.name));
        let resolved = resolve_candidates(cands, &[], &owner, anchor);
        let routed = route_candidates("current", resolved.clone(), &tracked, &[]);

        for (i, expected) in case.expect_same_as.iter().enumerate() {
            let cand = resolved.get(i);
            // Raw: only a valid in-range number counts as a merge.
            let raw_pred = cand
                .and_then(|c| c.same_as)
                .filter(|n| (1..=tracked.len() as u32).contains(n));
            // Routed: which tracked index (1-based) received an attach for this phrasing.
            let routed_pred = cand.and_then(|c| {
                routed
                    .attaches
                    .iter()
                    .find(|a| a.phrasing == c.description)
                    .and_then(|a| tracked.iter().position(|t| t.id == a.item_id))
                    .map(|p| p as u32 + 1)
            });
            println!(
                "{:<30} expect={:?} raw={:?} routed={:?} cand={:?}",
                case.name,
                expected,
                raw_pred,
                routed_pred,
                cand.map(|c| c.description.as_str())
            );
            raw.add(*expected, raw_pred);
            routed_score.add(*expected, routed_pred);
        }
    }
    println!("model={provider}/{model} cases={}", cases.len());
    println!(
        "RAW    precision={:.3} recall={:.3} ({}/{} predicted, {} expected)",
        raw.precision(),
        raw.recall(),
        raw.correct,
        raw.predicted,
        raw.expected
    );
    println!(
        "ROUTED precision={:.3} recall={:.3} ({}/{} predicted, {} expected)",
        routed_score.precision(),
        routed_score.recall(),
        routed_score.correct,
        routed_score.predicted,
        routed_score.expected
    );
    assert!(
        routed_score.precision() >= 0.9,
        "routed merge precision below 0.9"
    );
}
