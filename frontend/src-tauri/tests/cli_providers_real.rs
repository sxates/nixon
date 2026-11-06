//! specs/0085 task 10: opt-in quality check through the REAL Claude Code binary.
//!
//! Every test is `#[ignore]` and does nothing unless `NIXON_REAL_CLI=claude`. It sends
//! fictional `--demo` meetings (fixtures/demo/meetings) through the production summary
//! path (`generate_meeting_summary` with the built-in default prompt) and action-item
//! extraction (`extract_candidates` with the schema), then checks that the run left no
//! session file behind in `~/.claude/projects` (only names are listed; nothing is read).
//! Never runs in CI. It spends real subscription calls: two per meeting.
//!
//! Optional: `NIXON_REAL_OUT=<dir>` writes each meeting's summary and items there.
//!
//! Run with:
//!   NIXON_REAL_CLI=claude cargo test --features metal --test cli_providers_real -- --ignored --nocapture

use app_lib::action_items::extractor::{extract_candidates, ActionItemCandidate, OwnerIdentity};
use app_lib::summary::llm_client::LLMProvider;
use app_lib::summary::processor::{build_speaker_attributed_transcript, generate_meeting_summary};
use app_lib::summary::prompt_sanitize::prompt_for_request;
use app_lib::summary::prompts_resolve::FALLBACK_PROMPT;
use app_lib::summary::provider_config::ProviderConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

fn enabled() -> bool {
    std::env::var("NIXON_REAL_CLI").as_deref() == Ok("claude")
}

/// Entry name -> mtime for the direct children of `~/.claude/projects`.
fn snapshot_projects() -> BTreeMap<String, SystemTime> {
    let Some(home) = std::env::var_os("HOME") else {
        return BTreeMap::new();
    };
    let dir = Path::new(&home).join(".claude/projects");
    let mut out = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                out.insert(e.file_name().to_string_lossy().into_owned(), m);
            }
        }
    }
    out
}

/// Fails only on a project entry made for our own throwaway cwd (`nixon-cli-*`); other
/// live Claude sessions on the machine may legitimately change theirs.
fn assert_no_nixon_session(before: &BTreeMap<String, SystemTime>) {
    let after = snapshot_projects();
    let ours: Vec<&String> = after
        .iter()
        .filter(|(name, mtime)| name.contains("nixon-cli") && before.get(*name) != Some(mtime))
        .map(|(name, _)| name)
        .collect();
    assert!(ours.is_empty(), "session file left behind: {ours:?}");
}

fn claude_code() -> ProviderConfig {
    ProviderConfig {
        provider: LLMProvider::ClaudeCode,
        model_provider: "claude-code".into(),
        model_name: "sonnet".into(),
        api_key: String::new(),
        ollama_endpoint: None,
        custom_openai_endpoint: None,
        custom_openai_max_tokens: None,
        custom_openai_temperature: None,
        custom_openai_top_p: None,
    }
}

/// The demo meeting's transcript as `Name: text` lines, like the app builds it.
fn demo_transcript(file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/demo/meetings")
        .join(file);
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let names: BTreeMap<&str, &str> = doc["speakers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["key"].as_str().unwrap(),
                s["display_name"].as_str().unwrap(),
            )
        })
        .collect();
    let segments: Vec<(Option<String>, String)> = doc["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let who = s["speaker"]
                .as_str()
                .and_then(|k| names.get(k))
                .map(|n| n.to_string());
            (who, s["text"].as_str().unwrap().to_string())
        })
        .collect();
    build_speaker_attributed_transcript(&segments).0
}

struct Outcome {
    summary: String,
    items: Vec<ActionItemCandidate>,
    summary_secs: f64,
    extract_secs: f64,
}

/// Two real calls: the summary, then the extraction over that summary.
async fn run_meeting(file: &str) -> Outcome {
    let client = reqwest::Client::new();
    let config = claude_code();
    let transcript = demo_transcript(file);
    let t = Instant::now();
    let (summary, _english, _acct) = generate_meeting_summary(
        &client,
        &config.provider,
        &config.model_name,
        "",
        &transcript,
        &prompt_for_request(FALLBACK_PROMPT),
        180_000,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        true,
        None,
    )
    .await
    .expect("summary through Claude Code");
    let summary_secs = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let items = extract_candidates(
        &client,
        &config,
        None,
        &summary,
        None,
        &[],
        &[],
        &OwnerIdentity::default(),
    )
    .await
    .expect("extraction through Claude Code");
    let extract_secs = t.elapsed().as_secs_f64();
    eprintln!(
        "[{file}] summary {} chars in {summary_secs:.1}s, {} items in {extract_secs:.1}s",
        summary.len(),
        items.len()
    );
    if let Some(out) = std::env::var_os("NIXON_REAL_OUT") {
        let dir = PathBuf::from(out);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{file}.summary.md")), &summary).unwrap();
        let json = serde_json::json!({
            "summary_secs": summary_secs, "extract_secs": extract_secs, "items": items,
        });
        std::fs::write(
            dir.join(format!("{file}.items.json")),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();
    }
    Outcome {
        summary,
        items,
        summary_secs,
        extract_secs,
    }
}

#[tokio::test]
#[ignore = "spends real Claude subscription calls; set NIXON_REAL_CLI=claude"]
async fn demo_meeting_1_summary_and_action_items() {
    if !enabled() {
        eprintln!("skipped: NIXON_REAL_CLI is not `claude`");
        return;
    }
    let before = snapshot_projects();
    let out = run_meeting("01-product-sync.json").await;
    assert!(!out.summary.trim().is_empty(), "empty summary");
    assert!(
        out.items.len() >= 3,
        "expected 3+ action items, got {}",
        out.items.len()
    );
    assert!(out.summary_secs > 0.0 && out.extract_secs > 0.0);
    assert_no_nixon_session(&before);
}

#[tokio::test]
#[ignore = "spends real Claude subscription calls; set NIXON_REAL_CLI=claude"]
async fn demo_meetings_2_and_3_summary_and_action_items() {
    if !enabled() {
        eprintln!("skipped: NIXON_REAL_CLI is not `claude`");
        return;
    }
    let before = snapshot_projects();
    for file in ["02-one-on-one.json", "03-steerco.json"] {
        let out = run_meeting(file).await;
        assert!(!out.summary.trim().is_empty(), "empty summary for {file}");
    }
    assert_no_nixon_session(&before);
}
