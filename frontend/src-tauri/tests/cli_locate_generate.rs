//! specs/0085: a program chosen with Locate… is the one a real LLM call runs. Drives
//! `api_cli_provider_set_path` (the Locate path) and then `generate_summary_with_options`
//! with `LLMProvider::ClaudeCode`, against the fake CLI (never a real `claude`).
//!
//! Discovery order is env override > Locate override > fixed candidates, so the env
//! override that `common_cli` sets is removed first: otherwise it, not Locate, would win.
//!
//! Run with:
//!   cargo test --features metal --test cli_locate_generate

mod common_cli;

use app_lib::cli_llm::commands::api_cli_provider_set_path;
use app_lib::cli_llm::{config, discovery, CliKind};
use app_lib::database::manager::DatabaseManager;
use app_lib::database::repositories::setting::SettingsRepository;
use app_lib::state::AppState;
use app_lib::summary::llm_client::{generate_summary_with_options, LLMProvider};
use app_lib::summary::summary_engine::options::GenerationOptions;
use common_cli::FakeClaude;
use std::os::unix::fs::PermissionsExt;
use tauri::Manager;

#[tokio::test]
async fn a_located_program_is_the_one_a_summary_call_runs() {
    let fake = FakeClaude::install().await;
    fake.set_mode("success");
    std::env::remove_var("NIXON_CLI_CLAUDE_CODE");
    discovery::invalidate(CliKind::ClaudeCode);

    // The located program lives elsewhere and marks each run, then hands off to the
    // wrapper (which answers `--version` and the prompt run like a real `claude`).
    let located_dir = tempfile::tempdir().unwrap();
    let marker = located_dir.path().join("located-runs");
    let located = located_dir.path().join("claude");
    std::fs::write(
        &located,
        format!(
            "#!/bin/sh\necho \"$1\" >>'{}'\nexec '{}' \"$@\"\n",
            marker.display(),
            fake.wrapper().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&located, std::fs::Permissions::from_mode(0o755)).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new(
        dir.path().join("t.sqlite").to_str().unwrap(),
        dir.path().join("none.db").to_str().unwrap(),
    )
    .await
    .unwrap();
    SettingsRepository::save_model_config(db.pool(), "claude-code", "sonnet", "large-v3", None)
        .await
        .unwrap();
    let app = tauri::test::mock_app();
    app.manage(AppState { db_manager: db });

    api_cli_provider_set_path(
        app.state(),
        "claude-code".into(),
        Some(located.display().to_string()),
    )
    .await
    .unwrap();
    // Forget what set_path's re-probe cached, so the call itself has to resolve the binary.
    discovery::invalidate(CliKind::ClaudeCode);
    let _ = std::fs::remove_file(&marker);
    fake.set_mode("success");

    let reply = generate_summary_with_options(
        &reqwest::Client::new(),
        &LLMProvider::ClaudeCode,
        "sonnet",
        "",
        "Synthetic system prompt.",
        "Synthetic transcript for a fictional demo meeting.",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &GenerationOptions::default(),
    )
    .await;

    // Clean up the process-wide override before asserting, so a failure can't leak it.
    let pool = app.state::<AppState>().db_manager.pool().clone();
    config::set_binary_path(&pool, CliKind::ClaudeCode, None)
        .await
        .unwrap();
    discovery::invalidate(CliKind::ClaudeCode);

    assert!(
        reply.as_deref().unwrap().starts_with("**Summary"),
        "{reply:?}"
    );
    assert_eq!(fake.calls(), 1, "one prompt run");
    let runs = std::fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        runs.lines().any(|l| l == "-p"),
        "the prompt run went through the located program: {runs:?}"
    );
}
