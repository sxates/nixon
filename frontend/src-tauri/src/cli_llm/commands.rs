//! Tauri IPC for the subscription CLI providers (specs/0085): what Settings → Summary
//! shows for Claude Code / Codex, the Locate... override, and the Test button.
//!
//! Status only ever runs `--version` and `auth status` (never a model call). The one
//! command that spends the user's plan is [`api_cli_provider_test`], and only on a click.

use super::auth::{self, AuthState};
use super::discovery::{self, Discovered};
use super::errors::{CliError, CliFailure};
use super::{config, CliKind};
use crate::state::AppState;
use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;
use tauri::State;

/// The providers Settings lists. Codex joins in W2, when its adapter lands.
const LISTED: [CliKind; 1] = [CliKind::ClaudeCode];

/// The fixed, fictional prompt the Test button sends.
const TEST_PROMPT: &str = "Reply with the single word: ready";
const TEST_SYSTEM: &str = "You are a connection check. Follow the instruction exactly.";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CliState {
    NotFound,
    TooOld,
    NeedsSignIn,
    Ready,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliProviderStatus {
    pub id: &'static str,
    pub display_name: &'static str,
    pub state: CliState,
    /// Where the CLI was found, as found (a symlink stays a symlink).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub min_version: &'static str,
    /// The API-key billing heads-up, and nothing else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_note: Option<String>,
    /// Set only when a probe failed for a reason other than "not installed", "too old" or
    /// "signed out" (a hang, a crash, unreadable output): the state is then not trustworthy,
    /// so the UI shows this with Check again instead of the install / sign-in copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
    pub models: Vec<ModelChoice>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliTestResult {
    pub ok: bool,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn models(kind: CliKind) -> Vec<ModelChoice> {
    match kind {
        CliKind::ClaudeCode => vec![ModelChoice {
            id: "sonnet",
            label: "Sonnet",
        }],
        CliKind::CodexCli => vec![ModelChoice {
            id: "",
            label: "Codex default",
        }],
    }
}

/// The login-shell `command -v` runs user dotfiles, so it is allowed only while the user
/// has this very provider selected in Settings (owner decision 7), never per LLM call.
pub fn allow_shell_probe(saved_provider: Option<&str>, kind: CliKind) -> bool {
    saved_provider == Some(kind.id())
}

/// Maps a discovery result (and, when the CLI was found, its auth probe) to the status the
/// UI shows. A probe that failed for some other reason (timeout, crash, unreadable output)
/// keeps the nearest state but sets `probeError` to the reason.
pub fn build_status(
    kind: CliKind,
    found: Result<Discovered, CliError>,
    auth: Option<Result<AuthState, CliError>>,
) -> CliProviderStatus {
    let mut status = CliProviderStatus {
        id: kind.id(),
        display_name: kind.display_name(),
        state: CliState::NotFound,
        path: None,
        version: None,
        min_version: discovery::min_version(kind),
        auth_note: None,
        probe_error: None,
        models: models(kind),
    };
    let reason = |e: CliError| CliFailure::new(kind, e).to_string();
    match found {
        Err(CliError::NotFound) => {}
        Err(CliError::TooOld { found, .. }) => {
            status.state = CliState::TooOld;
            status.version = Some(found);
        }
        Err(other) => status.probe_error = Some(reason(other)),
        Ok(d) => {
            status.path = Some(d.path.display().to_string());
            status.version = Some(d.version);
            match auth {
                Some(Ok(AuthState::Ready { note })) => {
                    status.state = CliState::Ready;
                    status.auth_note = note;
                }
                Some(Ok(AuthState::NeedsSignIn)) | None => status.state = CliState::NeedsSignIn,
                Some(Err(e)) => {
                    status.state = CliState::NeedsSignIn;
                    status.probe_error = Some(reason(e));
                }
            }
        }
    }
    status
}

/// Forgets the cached binary, then looks again and asks the CLI who is signed in. Always
/// re-probing makes every call double as "Check again".
async fn probe_status(pool: &sqlx::SqlitePool, kind: CliKind) -> CliProviderStatus {
    discovery::invalidate(kind);
    let provider = config::saved_provider(pool).await.ok().flatten();
    let override_path = config::user_override(kind);
    let found = discovery::discover(
        kind,
        override_path,
        allow_shell_probe(provider.as_deref(), kind),
    )
    .await;
    let auth = match &found {
        Ok(d) => Some(auth::check(kind, &d.path).await),
        Err(_) => None,
    };
    build_status(kind, found, auth)
}

fn kind_of(id: &str) -> Result<CliKind, String> {
    CliKind::from_id(id).ok_or_else(|| format!("Unknown AI provider \"{id}\"."))
}

/// Detection state for each subscription CLI. Safe to call whenever Settings opens or the
/// user presses "Check again"; it re-probes every time.
#[tauri::command]
pub async fn api_cli_providers_status(
    state: State<'_, AppState>,
) -> Result<Vec<CliProviderStatus>, String> {
    let pool = state.db_manager.pool();
    let mut all = Vec::new();
    for kind in LISTED {
        all.push(probe_status(pool, kind).await);
    }
    Ok(all)
}

/// Locate...: validates `path` (absolute, executable, answers `--version` with a supported
/// version) before saving it. `None` clears the override. Returns the fresh status.
#[tauri::command]
pub async fn api_cli_provider_set_path(
    state: State<'_, AppState>,
    id: String,
    path: Option<String>,
) -> Result<CliProviderStatus, String> {
    let kind = kind_of(&id)?;
    let pool = state.db_manager.pool();
    let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(p) = &path {
        match discovery::validate_path(kind, &PathBuf::from(p)).await {
            Ok(_) => {}
            Err(CliError::NotFound | CliError::BadOutput) => {
                return Err(format!(
                    "That file can't be run as {}. Choose the `{}` program itself.",
                    kind.display_name(),
                    kind.command_name()
                ))
            }
            Err(e) => return Err(CliFailure::new(kind, e).to_string()),
        }
    }
    config::set_binary_path(pool, kind, path.as_deref())
        .await
        .map_err(|e| format!("Couldn't save that location: {e}"))?;
    Ok(probe_status(pool, kind).await)
}

/// The Test button: one real, user-initiated call through the full runner + isolation
/// path. `model` is accepted for the day the adapters take one; Claude uses its own pin.
#[tauri::command]
pub async fn api_cli_provider_test(
    _state: State<'_, AppState>,
    id: String,
    _model: String,
) -> Result<CliTestResult, String> {
    let kind = kind_of(&id)?;
    let started = Instant::now();
    let result = super::generate(
        kind,
        config::user_override(kind),
        TEST_SYSTEM,
        TEST_PROMPT,
        &crate::summary::summary_engine::options::GenerationOptions::default(),
        None,
    )
    .await;
    Ok(CliTestResult {
        ok: result.is_ok(),
        latency_ms: started.elapsed().as_millis() as u64,
        error: result.err(),
    })
}
