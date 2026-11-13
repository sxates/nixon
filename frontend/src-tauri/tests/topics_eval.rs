//! specs/0089 W0: entry-writer and mention-detector eval against a real model.
//!
//! `#[ignore]`d: the normal suite stays offline. Run with
//! `cargo test --features metal --test topics_eval -- --ignored --nocapture`.
//!
//! Env: `NIXON_EVAL_PROVIDER` (default `ollama`), `NIXON_EVAL_MODEL` (default `gemma4:26b`),
//! `NIXON_EVAL_ENDPOINT` (default `http://127.0.0.1:11434`; never `localhost`, which can
//! hang), `NIXON_EVAL_API_KEY`. Fixtures are invented. A mention is a true positive when its
//! topic matches an expected mention and its time falls in `[t_from, t_to]`. Only mention
//! precision is gated (0.9: a false mention clutters a topic); the rest is reported.

use app_lib::summary::llm_client::LLMProvider;
use app_lib::summary::provider_config::ProviderConfig;
use app_lib::topics::detector::detect;
use app_lib::topics::evidence::{blocks, for_entry, for_mentions, topic_terms, Segment};
use app_lib::topics::ledger::OpenDecision;
use app_lib::topics::model::Topic;
use app_lib::topics::writer::write_entry;
use serde::Deserialize;
use std::time::Instant;

#[derive(Deserialize)]
struct TopicIn {
    name: String,
    description: String,
}
#[derive(Deserialize)]
struct SegIn {
    t: f64,
    speaker: String,
    text: String,
}
#[derive(Deserialize)]
struct MentionExp {
    topic: String,
    t_from: f64,
    t_to: f64,
}
#[derive(Deserialize)]
struct Expect {
    mentions: Vec<MentionExp>,
    revises: Vec<String>,
    prior_decisions: Vec<String>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    topics: Vec<TopicIn>,
    linked: Option<usize>,
    summary: String,
    segments: Vec<SegIn>,
    expect: Expect,
}

fn topic(i: usize, t: &TopicIn) -> Topic {
    Topic {
        id: format!("topic-{i}"),
        name: t.name.clone(),
        description: t.description.clone(),
        status: "open".into(),
        notes_markdown: None,
        notes_json: None,
        created_at: String::new(),
        updated_at: String::new(),
        closed_at: None,
    }
}

fn ratio(n: usize, d: usize) -> f64 {
    if d == 0 {
        1.0
    } else {
        n as f64 / d as f64
    }
}

#[tokio::test]
#[ignore = "needs a reachable model; see module docs"]
async fn topics_entry_and_mention_eval() {
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
        serde_json::from_str(include_str!("fixtures/topics_eval/cases.json")).unwrap();
    let client = reqwest::Client::new();

    let (mut predicted, mut true_pos, mut expected, mut found) = (0, 0, 0, 0);
    let (mut times, mut times_valid) = (0, 0);
    let (mut rev_expected, mut rev_found, mut rev_spurious) = (0, 0, 0);
    let (mut quiet_cases, mut quiet_detector_ok, mut quiet_writer_ok) = (0, 0, 0);
    let mut call_secs: Vec<f64> = Vec::new();

    for case in &cases {
        let topics: Vec<Topic> = case
            .topics
            .iter()
            .enumerate()
            .map(|(i, t)| topic(i, t))
            .collect();
        let segs: Vec<Segment> = case
            .segments
            .iter()
            .map(|s| Segment {
                t: Some(s.t),
                speaker: Some(s.speaker.clone()),
                text: s.text.clone(),
            })
            .collect();
        let bs = blocks(&segs);
        let block_times: Vec<f64> = bs.iter().map(|b| b.t).collect();

        // Detector.
        let terms: Vec<String> = topics
            .iter()
            .flat_map(|t| topic_terms(&t.name, &t.description))
            .collect();
        let ev = for_mentions(&bs, &terms);
        let start = Instant::now();
        let got = detect(&client, &config, None, &topics, &case.summary, &ev)
            .await
            .unwrap_or_else(|e| panic!("{}: detector failed: {e:#}", case.name));
        let dt = start.elapsed().as_secs_f64();
        if !ev.text.trim().is_empty() {
            call_secs.push(dt);
        }
        let mut hit_exp = vec![false; case.expect.mentions.len()];
        let mut case_tp = 0;
        for (t, _summary, at) in &got {
            predicted += 1;
            times += 1;
            if block_times.iter().any(|b| (b - at).abs() < 1e-6) {
                times_valid += 1;
            }
            if let Some(i) =
                case.expect.mentions.iter().position(|e| {
                    e.topic == t.name && *at >= e.t_from - 1e-6 && *at <= e.t_to + 1e-6
                })
            {
                true_pos += 1;
                case_tp += 1;
                hit_exp[i] = true;
            }
        }
        expected += case.expect.mentions.len();
        found += hit_exp.iter().filter(|h| **h).count();
        let quiet = case.expect.mentions.is_empty();
        if quiet {
            quiet_cases += 1;
            if got.is_empty() {
                quiet_detector_ok += 1;
            }
        }
        println!(
            "[{}] detector {:.1}s evidence_chars={} mentions={} (tp {}/{} expected)",
            case.name,
            dt,
            ev.text.len(),
            got.len(),
            case_tp,
            case.expect.mentions.len()
        );

        // Writer: the linked topic, else the first expected-mention topic, else the first.
        let wi = case.linked.unwrap_or_else(|| {
            case.expect
                .mentions
                .first()
                .and_then(|m| topics.iter().position(|t| t.name == m.topic))
                .unwrap_or(0)
        });
        let wt = &topics[wi];
        let wev = for_entry(&bs, &topic_terms(&wt.name, &wt.description));
        let decisions: Vec<OpenDecision> = if case.linked.is_some() {
            case.expect
                .prior_decisions
                .iter()
                .enumerate()
                .map(|(i, text)| OpenDecision {
                    entry_id: format!("te-{i}"),
                    decision_id: format!("dec-{i}"),
                    text: text.clone(),
                    occurred_at: "2026-10-01T10:00:00Z".into(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let start = Instant::now();
        let entry = write_entry(
            &client,
            &config,
            None,
            wt,
            &case.summary,
            None,
            &wev,
            &decisions,
        )
        .await
        .unwrap_or_else(|e| panic!("{}: writer failed: {e:#}", case.name));
        let wdt = start.elapsed().as_secs_f64();
        call_secs.push(wdt);
        for t in entry
            .body
            .bullets
            .iter()
            .map(|b| b.t)
            .chain(entry.body.decisions.iter().map(|d| d.t))
            .flatten()
        {
            times += 1;
            if block_times.iter().any(|b| (b - t).abs() < 1e-6) {
                times_valid += 1;
            }
        }
        for sub in &case.expect.revises {
            rev_expected += 1;
            if entry
                .revisions
                .iter()
                .any(|(d, _, _)| d.text.to_lowercase().contains(&sub.to_lowercase()))
            {
                rev_found += 1;
            }
        }
        if case.expect.revises.is_empty() {
            rev_spurious += entry.revisions.len();
        }
        let writer_quiet = case.linked.is_some() && quiet;
        if writer_quiet && entry.body.bullets.is_empty() && entry.body.decisions.is_empty() {
            quiet_writer_ok += 1;
        }
        println!(
            "[{}] writer {:.1}s bullets={} decisions={} revisions={}",
            case.name,
            wdt,
            entry.body.bullets.len(),
            entry.body.decisions.len(),
            entry.revisions.len()
        );
    }

    let linked_quiet = cases
        .iter()
        .filter(|c| c.linked.is_some() && c.expect.mentions.is_empty())
        .count();
    let precision = ratio(true_pos, predicted);
    println!(
        "\n=== topics eval: {provider}/{model}, {} cases ===",
        cases.len()
    );
    println!("mention precision      {precision:.2} ({true_pos}/{predicted})");
    println!(
        "mention recall         {:.2} ({found}/{expected})",
        ratio(found, expected)
    );
    println!(
        "cited-time validity    {:.2} ({times_valid}/{times})",
        ratio(times_valid, times)
    );
    println!("revision recall        {:.2} ({rev_found}/{rev_expected}); spurious revisions {rev_spurious}", ratio(rev_found, rev_expected));
    println!("no-discussion correct  detector {quiet_detector_ok}/{quiet_cases}, writer (linked) {quiet_writer_ok}/{linked_quiet}");
    let mean = call_secs.iter().sum::<f64>() / call_secs.len().max(1) as f64;
    let max = call_secs.iter().cloned().fold(0.0, f64::max);
    println!(
        "seconds per call       mean {mean:.1}, max {max:.1} over {} calls",
        call_secs.len()
    );
    assert_eq!(times_valid, times, "a cited time was not a block label");
    assert!(precision >= 0.9, "mention precision {precision:.2} < 0.9");
}
