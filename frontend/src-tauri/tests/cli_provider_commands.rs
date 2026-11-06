//! specs/0085 task 8: the Settings IPC for the subscription CLI providers and the stored
//! Locate... override. Real migrated DB, `tauri::test::mock_app`, the fake CLI through the
//! wrapper in `common_cli` (never a real `claude`).
//!
//! Run with:
//!   cargo test --features metal --test cli_provider_commands

mod common_cli;

use app_lib::cli_llm::commands::{
    api_cli_provider_set_path, api_cli_provider_test, api_cli_providers_status, build_status,
    CliProviderStatus,
};
use app_lib::cli_llm::discovery::{self, Discovered};
use app_lib::cli_llm::errors::CliError;
use app_lib::cli_llm::{config, CliKind};
use app_lib::database::manager::DatabaseManager;
use app_lib::database::repositories::setting::SettingsRepository;
use app_lib::state::AppState;
use common_cli::FakeClaude;
use tauri::Manager;

async fn app_with_db(
    provider: Option<&str>,
) -> (tauri::App<tauri::test::MockRuntime>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new(
        dir.path().join("t.sqlite").to_str().unwrap(),
        dir.path().join("none.db").to_str().unwrap(),
    )
    .await
    .unwrap();
    if let Some(p) = provider {
        SettingsRepository::save_model_config(db.pool(), p, "sonnet", "large-v3", None)
            .await
            .unwrap();
    }
    let app = tauri::test::mock_app();
    app.manage(AppState { db_manager: db });
    (app, dir)
}

async fn status_of(app: &tauri::App<tauri::test::MockRuntime>) -> CliProviderStatus {
    let all = api_cli_providers_status(app.state(), None).await.unwrap();
    assert_eq!(all.len(), 1, "only Claude Code until W2");
    all.into_iter().next().unwrap()
}

#[tokio::test]
async fn migration_adds_the_column_on_a_fresh_db_and_config_round_trips() {
    // The override mirror is process-wide: hold the binary-wide lock while changing it.
    let _fake = FakeClaude::install().await;
    let (app, _dir) = app_with_db(None).await;
    let pool = app.state::<AppState>().db_manager.pool().clone();
    // The column exists even with no settings row yet.
    assert_eq!(
        config::binary_path(&pool, CliKind::ClaudeCode)
            .await
            .unwrap(),
        None
    );
    // With no settings row, Locate refuses rather than inventing a provider row.
    let err = config::set_binary_path(&pool, CliKind::ClaudeCode, Some("/opt/x/claude"))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), config::NO_SETTINGS_ROW);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "no settings row may be created by Locate");
    assert_eq!(config::user_override(CliKind::ClaudeCode), None);
    SettingsRepository::save_model_config(&pool, "builtin-ai", "qwen3.5:2b", "large-v3", None)
        .await
        .unwrap();
    config::set_binary_path(&pool, CliKind::ClaudeCode, Some("/opt/x/claude"))
        .await
        .unwrap();
    config::set_binary_path(&pool, CliKind::CodexCli, Some("/opt/x/codex"))
        .await
        .unwrap();
    assert_eq!(
        config::binary_path(&pool, CliKind::ClaudeCode)
            .await
            .unwrap()
            .as_deref(),
        Some("/opt/x/claude")
    );
    // Clearing one leaves the other, and the stored JSON has the spec's shape.
    config::set_binary_path(&pool, CliKind::ClaudeCode, None)
        .await
        .unwrap();
    assert_eq!(
        config::binary_path(&pool, CliKind::ClaudeCode)
            .await
            .unwrap(),
        None
    );
    let json: String = sqlx::query_scalar("SELECT cliProviderConfig FROM settings WHERE id='1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["codex-cli"]["binaryPath"], "/opt/x/codex");
    assert!(v["claude-code"]["binaryPath"].is_null());
}

#[tokio::test]
async fn saving_the_override_keeps_the_chosen_provider_and_model() {
    let _fake = FakeClaude::install().await;
    let (app, _dir) = app_with_db(Some("claude-code")).await;
    let pool = app.state::<AppState>().db_manager.pool().clone();
    config::set_binary_path(&pool, CliKind::ClaudeCode, Some("/opt/x/claude"))
        .await
        .unwrap();
    let (p, m): (String, String) =
        sqlx::query_as("SELECT provider, model FROM settings WHERE id='1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((p.as_str(), m.as_str()), ("claude-code", "sonnet"));
    config::set_binary_path(&pool, CliKind::ClaudeCode, None)
        .await
        .unwrap();
}

#[test]
fn build_status_maps_every_state_and_serializes_camel_case() {
    let k = CliKind::ClaudeCode;
    let found = Discovered {
        path: "/bin/claude".into(),
        version: "2.1.319".into(),
    };
    let ready = build_status(
        k,
        Ok(found.clone()),
        Some(Ok(app_lib::cli_llm::auth::AuthState::Ready { note: None })),
    );
    assert_eq!(
        serde_json::to_value(&ready).unwrap(),
        serde_json::json!({
            "id": "claude-code", "displayName": "Claude Code", "state": "ready",
            "path": "/bin/claude", "version": "2.1.319", "minVersion": "2.1.290",
            "models": [{"id": "sonnet", "label": "Sonnet"}]
        })
    );
    let signed_out = build_status(
        k,
        Ok(found.clone()),
        Some(Ok(app_lib::cli_llm::auth::AuthState::NeedsSignIn)),
    );
    assert_eq!(
        serde_json::to_value(&signed_out).unwrap()["state"],
        "needsSignIn"
    );
    let api_key = build_status(
        k,
        Ok(found),
        Some(Ok(app_lib::cli_llm::auth::AuthState::Ready {
            note: Some("note".into()),
        })),
    );
    assert_eq!(serde_json::to_value(&api_key).unwrap()["authNote"], "note");
    let missing = serde_json::to_value(build_status(k, Err(CliError::NotFound), None)).unwrap();
    assert_eq!(missing["state"], "notFound");
    assert!(missing.get("path").is_none());
    let old = serde_json::to_value(build_status(
        k,
        Err(CliError::TooOld {
            found: "2.1.200".into(),
            min: "2.1.290".into(),
        }),
        None,
    ))
    .unwrap();
    assert_eq!(old["state"], "tooOld");
    assert_eq!(old["version"], "2.1.200");
    // A probe that failed some other way is "can't use it", with the reason.
    let odd = serde_json::to_value(build_status(k, Err(CliError::ProbeTimeout), None)).unwrap();
    assert_eq!(odd["state"], "notFound");
    assert!(odd["probeError"]
        .as_str()
        .unwrap()
        .contains("didn't respond"));
    assert!(odd.get("authNote").is_none());
    let found = Discovered {
        path: "/bin/claude".into(),
        version: "2.1.319".into(),
    };
    for e in [CliError::ProbeTimeout, CliError::Crashed { code: 2 }] {
        let v = serde_json::to_value(build_status(k, Err(e.clone()), None)).unwrap();
        assert!(v["probeError"].is_string(), "{e:?}");
        let v =
            serde_json::to_value(build_status(k, Ok(found.clone()), Some(Err(e.clone())))).unwrap();
        assert!(v["probeError"].is_string(), "{e:?}");
        assert!(v.get("authNote").is_none());
    }
    // Genuine states never carry one.
    for v in [&ready, &signed_out, &api_key] {
        assert!(serde_json::to_value(v).unwrap().get("probeError").is_none());
    }
    assert!(missing.get("probeError").is_none() && old.get("probeError").is_none());
    let codex = serde_json::to_value(build_status(
        CliKind::CodexCli,
        Err(CliError::NotFound),
        None,
    ))
    .unwrap();
    assert_eq!(
        codex["models"],
        serde_json::json!([{"id": "", "label": "Codex default"}])
    );
}

#[tokio::test]
async fn status_reports_ready_signed_out_and_too_old_and_only_ever_probes() {
    let fake = FakeClaude::install().await;
    let (app, _dir) = app_with_db(None).await;

    fake.set_mode("status-ready");
    let s = status_of(&app).await;
    assert_eq!(s.id, "claude-code");
    assert_eq!(serde_json::to_value(&s).unwrap()["state"], "ready");
    assert_eq!(s.version.as_deref(), Some("2.1.319"));
    assert_eq!(s.path.as_deref(), Some(fake.wrapper().to_str().unwrap()));
    // Acceptance 10: nothing but `--version` and `auth status` ran, and no prompt.
    let seen = fake.invocations();
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|a| a == "--version" || a == "auth status"),
        "{seen:?}"
    );
    assert!(seen.iter().any(|a| a == "auth status"));
    assert_eq!(
        fake.calls(),
        seen.iter().filter(|a| *a == "auth status").count()
    );

    fake.set_mode("status-signed-out");
    assert_eq!(
        serde_json::to_value(status_of(&app).await).unwrap()["state"],
        "needsSignIn"
    );

    assert!(serde_json::to_value(status_of(&app).await)
        .unwrap()
        .get("probeError")
        .is_none());

    // An auth probe that crashes is a probe error, not "signed out".
    fake.set_mode("crash");
    let crashed = serde_json::to_value(status_of(&app).await).unwrap();
    assert!(crashed["probeError"].is_string(), "{crashed}");

    fake.set_mode("version-old");
    let old = serde_json::to_value(status_of(&app).await).unwrap();
    assert_eq!(old["state"], "tooOld");
    assert_eq!(old["version"], "2.1.200");
    assert_eq!(old["minVersion"], "2.1.290");
}

#[test]
fn the_login_shell_probe_is_allowed_only_for_the_selected_provider() {
    use app_lib::cli_llm::commands::allow_shell_probe;
    let (k, c) = (CliKind::ClaudeCode, CliKind::CodexCli);
    // Saved in Settings.
    assert!(allow_shell_probe(Some("claude-code"), None, k));
    assert!(!allow_shell_probe(Some("claude-code"), None, c));
    assert!(!allow_shell_probe(Some("builtin-ai"), None, k));
    // Picked in the chooser (the panel is showing it), not yet saved.
    assert!(allow_shell_probe(None, Some("claude-code"), k));
    assert!(allow_shell_probe(Some("builtin-ai"), Some("claude-code"), k));
    assert!(!allow_shell_probe(None, Some("codex-cli"), k));
    // The dropdown's status call names nothing.
    assert!(!allow_shell_probe(None, None, k));
}

#[tokio::test]
async fn the_chooser_selection_allows_the_shell_probe_and_the_dropdown_never_does() {
    use app_lib::cli_llm::commands::shell_probe_allowed;
    let k = CliKind::ClaudeCode;
    // First-time user: nothing saved yet, Claude Code picked in the chooser.
    let (app, _dir) = app_with_db(Some("builtin-ai")).await;
    let pool = app.state::<AppState>().db_manager.pool().clone();
    assert!(shell_probe_allowed(&pool, Some("claude-code"), k).await);
    assert!(!shell_probe_allowed(&pool, None, k).await, "dropdown open");
    // No settings row at all: same rule.
    let (fresh, _d) = app_with_db(None).await;
    let fresh_pool = fresh.state::<AppState>().db_manager.pool().clone();
    assert!(shell_probe_allowed(&fresh_pool, Some("claude-code"), k).await);
    assert!(!shell_probe_allowed(&fresh_pool, None, k).await);
    // Saved provider keeps working with no selection passed.
    let (saved, _s) = app_with_db(Some("claude-code")).await;
    let saved_pool = saved.state::<AppState>().db_manager.pool().clone();
    assert!(shell_probe_allowed(&saved_pool, None, k).await);
}

#[tokio::test]
async fn status_checks_again_every_call() {
    let fake = FakeClaude::install().await;
    let (app, _dir) = app_with_db(None).await;
    fake.set_mode("status-ready");
    assert_eq!(
        serde_json::to_value(status_of(&app).await).unwrap()["state"],
        "ready"
    );
    // The cache would still hold the first answer's version; a re-check must re-probe.
    fake.set_mode("version-old");
    assert_eq!(
        serde_json::to_value(status_of(&app).await).unwrap()["state"],
        "tooOld"
    );
}

#[tokio::test]
async fn set_path_rejects_what_cannot_run_and_saves_what_can() {
    let fake = FakeClaude::install().await;
    fake.set_mode("status-ready");
    let (app, scratch) = app_with_db(Some("builtin-ai")).await;
    let pool = app.state::<AppState>().db_manager.pool().clone();

    let plain = scratch.path().join("not-executable");
    std::fs::write(&plain, "#!/bin/sh\necho 2.1.319\n").unwrap();
    let err = api_cli_provider_set_path(
        app.state(),
        "claude-code".into(),
        Some(plain.display().to_string()),
    )
    .await
    .unwrap_err();
    assert!(err.contains("claude"), "{err}");
    let missing = scratch.path().join("nope");
    assert!(api_cli_provider_set_path(
        app.state(),
        "claude-code".into(),
        Some(missing.display().to_string())
    )
    .await
    .is_err());
    assert!(api_cli_provider_set_path(
        app.state(),
        "claude-code".into(),
        Some("relative/claude".into())
    )
    .await
    .is_err());
    assert!(
        api_cli_provider_set_path(app.state(), "gemini-cli".into(), None)
            .await
            .is_err()
    );
    assert_eq!(
        config::binary_path(&pool, CliKind::ClaudeCode)
            .await
            .unwrap(),
        None
    );

    // A too-old binary is rejected with the version copy, and not saved.
    fake.set_mode("version-old");
    let wrapper = fake.wrapper();
    let err = api_cli_provider_set_path(
        app.state(),
        "claude-code".into(),
        Some(wrapper.display().to_string()),
    )
    .await
    .unwrap_err();
    assert!(err.contains("2.1.200"), "{err}");
    assert_eq!(
        config::binary_path(&pool, CliKind::ClaudeCode)
            .await
            .unwrap(),
        None
    );

    // A valid binary is saved, mirrored for generate(), and the fresh status comes back.
    fake.set_mode("status-ready");
    let s = api_cli_provider_set_path(
        app.state(),
        "claude-code".into(),
        Some(wrapper.display().to_string()),
    )
    .await
    .unwrap();
    assert_eq!(serde_json::to_value(&s).unwrap()["state"], "ready");
    // The status says an override is saved, so the panel can offer "Use default".
    assert_eq!(
        serde_json::to_value(&s).unwrap()["overridePath"],
        wrapper.to_str().unwrap()
    );
    assert_eq!(
        config::binary_path(&pool, CliKind::ClaudeCode)
            .await
            .unwrap()
            .as_deref(),
        wrapper.to_str()
    );
    assert_eq!(
        config::user_override(CliKind::ClaudeCode),
        Some(wrapper.clone())
    );
    assert!(discovery::cached(CliKind::ClaudeCode).is_some());

    // None clears it (and the cache, so the next lookup re-resolves).
    let cleared = api_cli_provider_set_path(app.state(), "claude-code".into(), None)
        .await
        .unwrap();
    assert!(serde_json::to_value(&cleared)
        .unwrap()
        .get("overridePath")
        .is_none());
    assert_eq!(
        config::binary_path(&pool, CliKind::ClaudeCode)
            .await
            .unwrap(),
        None
    );
    assert_eq!(config::user_override(CliKind::ClaudeCode), None);
}

#[tokio::test]
async fn test_command_sends_the_fixed_prompt_and_reports_latency_or_the_failure_copy() {
    let fake = FakeClaude::install().await;
    let (app, _dir) = app_with_db(None).await;
    fake.set_mode("success");
    let ok = api_cli_provider_test(app.state(), "claude-code".into(), "sonnet".into())
        .await
        .unwrap();
    assert!(ok.ok);
    assert!(ok.error.is_none());
    assert_eq!(fake.calls(), 1);
    fake.set_mode("not-logged-in");
    let bad = api_cli_provider_test(app.state(), "claude-code".into(), "sonnet".into())
        .await
        .unwrap();
    assert!(!bad.ok);
    assert!(bad.error.unwrap().contains("signed in"));
    let v = serde_json::to_value(&ok).unwrap();
    assert!(v.get("latencyMs").is_some() && v.get("error").is_none());
}

#[tokio::test]
async fn locate_on_a_db_with_no_settings_row_refuses_with_plain_copy() {
    let fake = FakeClaude::install().await;
    fake.set_mode("status-ready");
    let (app, _dir) = app_with_db(None).await;
    let pool = app.state::<AppState>().db_manager.pool().clone();
    let err = api_cli_provider_set_path(
        app.state(),
        "claude-code".into(),
        Some(fake.wrapper().display().to_string()),
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        "Finish setting up Nixon first, then choose Claude Code."
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "Locate must not insert provider='builtin-ai', model=''");
    assert_eq!(config::user_override(CliKind::ClaudeCode), None);
}

#[tokio::test]
async fn status_reports_no_override_when_none_is_saved() {
    let fake = FakeClaude::install().await;
    fake.set_mode("status-ready");
    let (app, _dir) = app_with_db(Some("builtin-ai")).await;
    let pool = app.state::<AppState>().db_manager.pool().clone();
    config::set_binary_path(&pool, CliKind::ClaudeCode, None)
        .await
        .unwrap();
    let v = serde_json::to_value(status_of(&app).await).unwrap();
    assert_eq!(v["state"], "ready");
    assert!(v.get("overridePath").is_none(), "{v}");
}
