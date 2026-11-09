//! Tauri IPC for the Settings agent-access switch (specs/0087).

use super::log::{AccessEntry, AccessLog};
use super::register::{self, AgentAccessStatus, Profile};
use super::settings;
use crate::app_paths;
use crate::cli_llm::{self, CliKind};
use crate::state::AppState;
use serde::Serialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CLAUDE_TIMEOUT: Duration = Duration::from_secs(20);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn profile() -> Profile {
    register::profile_for(app_paths::bundle_identifier().unwrap_or("ai.vinyl.app"))
}

fn set_last_error(e: &Option<String>) {
    if let Ok(mut g) = LAST_ERROR.lock() {
        *g = e.clone();
    }
}

fn last_error() -> Option<String> {
    LAST_ERROR.lock().ok().and_then(|g| g.clone())
}

/// Runs `claude` with the normal user environment (it must write `~/.claude.json`).
fn run_claude(bin: &std::path::Path, args: &[String]) -> Result<(), String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run Claude Code: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().map_err(|e| e.to_string())?;
                if status.success() {
                    return Ok(());
                }
                let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let msg = if msg.is_empty() {
                    String::from_utf8_lossy(&out.stdout).trim().to_string()
                } else {
                    msg
                };
                return Err(if msg.is_empty() {
                    format!("Claude Code exited with {status}.")
                } else {
                    msg
                });
            }
            Ok(None) if start.elapsed() > CLAUDE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Claude Code did not answer within 20 seconds.".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

async fn find_claude() -> Result<PathBuf, String> {
    cli_llm::discovery::discover(
        CliKind::ClaudeCode,
        cli_llm::config::user_override(CliKind::ClaudeCode),
        false,
    )
    .await
    .map(|d| d.path)
    .map_err(|_| {
        "Nixon could not find Claude Code. Install it, or paste the command below into a terminal."
            .to_string()
    })
}

async fn apply(enabled: bool) -> Result<AgentAccessStatus, String> {
    let p = profile();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = app_paths::app_data_dir();
    let claude = find_claude().await;
    let st = tauri::async_runtime::spawn_blocking(move || {
        let run = |a: &[String]| match &claude {
            Ok(bin) => run_claude(bin, a),
            Err(e) => Err(e.clone()),
        };
        register::set_enabled(enabled, &p, &exe, &dir, &run)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    set_last_error(&st.last_error);
    Ok(st)
}

#[tauri::command]
pub async fn api_agent_access_status() -> Result<AgentAccessStatus, String> {
    let p = profile();
    let dir = app_paths::app_data_dir();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let s = settings::load(&dir);
    Ok(AgentAccessStatus {
        enabled: s.enabled,
        registered: s.enabled && s.registered_exe.is_some() && last_error().is_none(),
        server_name: p.server_name.to_string(),
        manual_command: register::manual_command(&p, &exe, &dir),
        last_error: last_error(),
    })
}

#[tauri::command]
pub async fn api_agent_access_set_enabled(enabled: bool) -> Result<AgentAccessStatus, String> {
    apply(enabled).await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingRef {
    pub id: String,
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessEntryView {
    #[serde(flatten)]
    pub entry: AccessEntry,
    pub meetings: Vec<MeetingRef>,
}

#[tauri::command]
pub async fn api_agent_access_log(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<AccessEntryView>, String> {
    let log = AccessLog::open(&app_paths::app_data_dir())
        .await
        .map_err(|e| e.to_string())?;
    let entries = log
        .list(limit.unwrap_or(100))
        .await
        .map_err(|e| e.to_string())?;
    let pool = state.db_manager.pool();
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut meetings = Vec::new();
        for id in &entry.meeting_ids {
            let title: Option<Option<String>> =
                sqlx::query_scalar("SELECT title FROM meetings WHERE id = ?")
                    .bind(id)
                    .fetch_optional(pool)
                    .await
                    .unwrap_or(None);
            meetings.push(MeetingRef {
                id: id.clone(),
                title: title.flatten(),
            });
        }
        out.push(AccessEntryView { entry, meetings });
    }
    Ok(out)
}

#[tauri::command]
pub async fn api_agent_access_clear_log() -> Result<(), String> {
    let log = AccessLog::open(&app_paths::app_data_dir())
        .await
        .map_err(|e| e.to_string())?;
    log.clear().await.map_err(|e| e.to_string())
}

/// Spawned once from setup. Never prompts or toasts; failures are only logged.
pub fn refresh_registration_on_startup() {
    tauri::async_runtime::spawn(async {
        let p = profile();
        let dir = app_paths::app_data_dir();
        if !settings::load(&dir).enabled {
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let claude = find_claude().await;
        let res = tauri::async_runtime::spawn_blocking(move || {
            let run = |a: &[String]| match &claude {
                Ok(bin) => run_claude(bin, a),
                Err(e) => Err(e.clone()),
            };
            register::refresh_if_needed(&p, &exe, &dir, &run)
        })
        .await;
        if let Ok(Some(e)) = res {
            log::warn!("agent access: could not refresh registration: {e}");
        }
    });
}
