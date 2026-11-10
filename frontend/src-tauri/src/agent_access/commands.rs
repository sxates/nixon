//! Tauri IPC for the Settings agent-access switch (specs/0087).

use super::desktop_config;
use super::log::{AccessEntry, AccessLog};
use super::log_export;
use super::register::{self, AgentAccessStatus, Profile};
use super::settings;
use crate::app_paths;
use crate::cli_llm::{self, CliKind};
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CLAUDE_TIMEOUT: Duration = Duration::from_secs(20);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
static DESKTOP_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn profile() -> Profile {
    register::profile_for(app_paths::bundle_identifier().unwrap_or("ai.vinyl.app"))
}

fn set_err(slot: &Mutex<Option<String>>, e: &Option<String>) {
    if let Ok(mut g) = slot.lock() {
        *g = e.clone();
    }
}

fn get_err(slot: &Mutex<Option<String>>) -> Option<String> {
    slot.lock().ok().and_then(|g| g.clone())
}

fn set_last_error(e: &Option<String>) {
    set_err(&LAST_ERROR, e);
}

fn last_error() -> Option<String> {
    get_err(&LAST_ERROR)
}

/// The Claude desktop config, or `None` when there is no home folder.
fn desktop_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| desktop_config::config_path(&h))
}

/// Adds or removes the Claude desktop chat entry (Amendment A). Independent of Claude Code:
/// its failure is reported separately and never changes the switch.
fn apply_desktop(enabled: bool, p: &Profile, exe: &std::path::Path, dir: &std::path::Path) {
    let Some(cfg) = desktop_path() else { return };
    let res = if enabled {
        desktop_config::add(&cfg, p, exe, dir).map(|_| ())
    } else {
        desktop_config::remove(&cfg, p)
    };
    if let Err(e) = &res {
        log::warn!("agent access: Claude desktop config: {e}");
    }
    set_err(&DESKTOP_ERROR, &res.err());
}

/// Fills the desktop fields from the file as it is now.
fn with_desktop(mut st: AgentAccessStatus, p: &Profile) -> AgentAccessStatus {
    if let Some(cfg) = desktop_path() {
        st.desktop_installed = desktop_config::is_installed(&cfg);
        st.desktop_registered = st.desktop_installed && desktop_config::is_registered(&cfg, p);
    }
    st.desktop_error = get_err(&DESKTOP_ERROR);
    st
}

/// Runs `claude` with the normal user environment (it must write `~/.claude.json`).
fn run_claude(bin: &std::path::Path, args: &[String]) -> Result<(), String> {
    let mut child = Command::new(bin)
        .args(args)
        .env(
            "PATH",
            register::child_path(bin, std::env::var_os("PATH").as_deref()),
        )
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
    let p2 = p.clone();
    let st = tauri::async_runtime::spawn_blocking(move || {
        let run = |a: &[String]| match &claude {
            Ok(bin) => run_claude(bin, a),
            Err(e) => Err(e.clone()),
        };
        let st = register::set_enabled(enabled, &p2, &exe, &dir, &run);
        apply_desktop(enabled, &p2, &exe, &dir);
        st
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    set_last_error(&st.last_error);
    Ok(with_desktop(st, &p))
}

#[tauri::command]
pub async fn api_agent_access_status() -> Result<AgentAccessStatus, String> {
    let p = profile();
    let dir = app_paths::app_data_dir();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let s = settings::load(&dir);
    let st = AgentAccessStatus {
        enabled: s.enabled,
        registered: s.enabled && s.registered_exe.is_some() && last_error().is_none(),
        server_name: p.server_name.to_string(),
        manual_command: register::manual_command(&p, &exe, &dir),
        last_error: last_error(),
        desktop_installed: false,
        desktop_registered: false,
        desktop_error: None,
    };
    Ok(with_desktop(st, &p))
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
    let titles = meeting_titles(state.db_manager.pool(), &entries).await;
    Ok(entries
        .into_iter()
        .map(|entry| {
            let meetings = entry
                .meeting_ids
                .iter()
                .map(|id| MeetingRef {
                    id: id.clone(),
                    title: titles.get(id).cloned().flatten(),
                })
                .collect();
            AccessEntryView { entry, meetings }
        })
        .collect())
}

/// id → title for every meeting the entries name; a deleted meeting is absent.
async fn meeting_titles(
    pool: &sqlx::SqlitePool,
    entries: &[AccessEntry],
) -> HashMap<String, Option<String>> {
    let mut titles = HashMap::new();
    for id in entries.iter().flat_map(|e| &e.meeting_ids) {
        if titles.contains_key(id) {
            continue;
        }
        let title: Option<Option<String>> =
            sqlx::query_scalar("SELECT title FROM meetings WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await
                .unwrap_or(None);
        if let Some(t) = title {
            titles.insert(id.clone(), t);
        }
    }
    titles
}

/// Writes everything the log holds (90 days) to a text file and opens it.
#[tauri::command]
pub async fn api_agent_access_open_full_log(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let dir = app_paths::app_data_dir();
    let log = AccessLog::open(&dir).await.map_err(|e| e.to_string())?;
    let entries = log.list(u32::MAX).await.map_err(|e| e.to_string())?;
    let titles = meeting_titles(state.db_manager.pool(), &entries).await;
    let path = log_export::file_path(&dir);
    std::fs::write(&path, log_export::format(&entries, &titles))
        .map_err(|e| format!("Could not write the log file: {e}"))?;
    Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Could not open the log file: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn api_agent_access_clear_log() -> Result<(), String> {
    let log = AccessLog::open(&app_paths::app_data_dir())
        .await
        .map_err(|e| e.to_string())?;
    log.clear().await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(log_export::file_path(&app_paths::app_data_dir()));
    Ok(())
}

/// Spawned once from setup. Never prompts or toasts; failures are only logged. The desktop
/// entry is re-ensured every launch (a no-op write when already right), which also covers
/// installs switched on before Amendment A.
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
            apply_desktop(true, &p, &exe, &dir);
            register::refresh_if_needed(&p, &exe, &dir, &run)
        })
        .await;
        if let Ok(Some(e)) = res {
            log::warn!("agent access: could not refresh registration: {e}");
        }
    });
}
