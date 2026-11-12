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

use app_lib::action_items::diff::content_key;
use app_lib::action_items::extractor::{extract_candidates, resolve_candidates, OwnerIdentity};
use app_lib::action_items::merge::{route_candidates, TrackedItem};
use app_lib::database::repositories::action_item::ActionItem;
use app_lib::summary::llm_client::LLMProvider;
use app_lib::summary::provider_config::ProviderConfig;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
struct Expect {
    /// Lowercase substring of the candidate description this expectation is for; empty
    /// matches any candidate (single-task cases).
    hint: String,
    same_as: Option<u32>,
}

/// `tracked` entries prefixed `@current:` belong to the meeting being extracted: they are
/// also passed as that meeting's own rows and must never receive an attach.
#[derive(Deserialize)]
struct Case {
    name: String,
    tracked: Vec<String>,
    summary: String,
    expect: Vec<Expect>,
}

#[derive(Default)]
struct Score {
    predicted: u32,
    correct: u32,
    expected: u32,
}

impl Score {
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

/// The expectation a candidate is judged against: the first whose hint it contains;
/// none means an extra candidate, which is expected not to merge.
fn expect_index(case: &Case, description: &str) -> Option<usize> {
    let d = description.to_lowercase();
    case.expect.iter().position(|e| d.contains(&e.hint))
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
        let mut tracked: Vec<TrackedItem> = Vec::new();
        let mut own: Vec<ActionItem> = Vec::new();
        for (i, d) in case.tracked.iter().enumerate() {
            let (meeting, text) = match d.strip_prefix("@current:") {
                Some(t) => ("current", t),
                None => ("prior", d.as_str()),
            };
            let id = format!("t{}", i + 1);
            if meeting == "current" {
                own.push(own_row(&id, text));
            }
            tracked.push(TrackedItem {
                id,
                meeting_id: Some(meeting.into()),
                description: text.into(),
            });
        }
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
        let routed = route_candidates("current", resolved.clone(), &tracked, &own);
        if resolved.len() != case.expect.len() {
            println!(
                "WARNING {}: {} candidates, {} expected",
                case.name,
                resolved.len(),
                case.expect.len()
            );
        }

        // Expected positives: raw counts every positive expectation; routed counts
        // distinct targets (an item takes at most one attach per run).
        raw.expected += case.expect.iter().filter(|e| e.same_as.is_some()).count() as u32;
        let targets: HashSet<u32> = case.expect.iter().filter_map(|e| e.same_as).collect();
        routed_score.expected += targets.len() as u32;

        // Raw and routed are scored per prediction. A prediction is correct only if it
        // fills a not-yet-filled expected slot; repeats and unmatched phrasings are false
        // positives.
        let mut raw_filled: HashSet<usize> = HashSet::new();
        for cand in &resolved {
            let slot = expect_index(case, &cand.description);
            let want = slot.and_then(|i| case.expect[i].same_as);
            // Raw: a valid in-range number pointing at another meeting's item. A `same_as`
            // at the current meeting's own row is a legitimate forced own-row match (spec
            // ruling R10), not a cross-meeting merge, so it is not scored here.
            let raw_pred = cand.same_as.filter(|n| {
                tracked
                    .get((*n as usize).wrapping_sub(1))
                    .is_some_and(|t| t.meeting_id.as_deref() != Some("current"))
            });
            if let Some(n) = raw_pred {
                raw.predicted += 1;
                if want == Some(n) && raw_filled.insert(slot.unwrap()) {
                    raw.correct += 1;
                }
            }
            println!(
                "{:<40} want={:?} raw={:?} cand={:?}",
                case.name, want, raw_pred, cand.description
            );
        }
        // Routed: every attach is one prediction.
        let mut filled: HashSet<u32> = HashSet::new();
        for a in &routed.attaches {
            routed_score.predicted += 1;
            let target = tracked
                .iter()
                .position(|t| t.id == a.item_id)
                .map(|p| p as u32 + 1);
            let want = expect_index(case, &a.phrasing).and_then(|i| case.expect[i].same_as);
            let ok = target.is_some() && target == want && filled.insert(target.unwrap());
            routed_score.correct += ok as u32;
            println!(
                "{:<40} attach -> {:?} want={:?} ok={}",
                case.name, target, want, ok
            );
        }
        println!(
            "{:<40} attaches={} for_diff={} kept={}",
            case.name,
            routed.attaches.len(),
            routed.for_diff.len(),
            routed.kept.len()
        );
        match case.name.as_str() {
            "hard-many-to-one" => {
                assert_eq!(routed.attaches.len(), 1, "many-to-one: exactly one attach");
                assert_eq!(routed.attaches[0].item_id, "t1");
                assert_eq!(routed.for_diff.len() + routed.kept.len(), 1);
            }
            "hard-current-meeting-row" => {
                assert!(
                    routed.attaches.is_empty(),
                    "own-meeting row must not attach"
                );
                assert_eq!(routed.kept.len(), 1, "own-meeting row is a forced match");
            }
            _ => {}
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
    assert!(raw.correct <= raw.expected, "raw correct exceeds expected");
    assert!(
        routed_score.correct <= routed_score.expected,
        "routed correct exceeds expected"
    );
    assert!(
        routed_score.expected == 0 || routed_score.correct > 0,
        "routed recall is 0: a model that never merges must not pass"
    );
    assert!(
        routed_score.precision() >= 0.9,
        "routed merge precision below 0.9"
    );
}

/// A minimal own-meeting row for the `own` argument of routing.
fn own_row(id: &str, description: &str) -> ActionItem {
    ActionItem {
        id: id.into(),
        meeting_id: Some("current".into()),
        description: description.into(),
        assignee_person_id: None,
        assignee_is_self: false,
        assignee_raw: None,
        due_hint: None,
        due_date: None,
        status: "open".into(),
        source: "extracted".into(),
        user_edited: false,
        content_key: content_key(description),
        created_at: String::new(),
        updated_at: String::new(),
        completed_at: None,
        sort_order: None,
        triage: "suggested".into(),
        adopted_at: None,
        notes_markdown: None,
    }
}
