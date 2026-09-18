//! Debug-only loopback control listener for screenshot drivers (specs/0060).
//! Newline-delimited JSON over 127.0.0.1; port written to <app_data_dir>/dev-control.port.
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use super::guard;

pub static SHOT_READY: AtomicBool = AtomicBool::new(false);
pub const PORT_FILE: &str = "dev-control.port";

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Navigate {
        route: String,
    },
    Theme {
        value: String,
    },
    OnboardingStep {
        value: u8,
    },
    OnboardingComplete,
    Resize {
        w: f64,
        h: f64,
    },
    Ready {
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    HideDevBadge {
        value: bool,
    },
    Window,
    StartRecording {
        #[serde(default)]
        title: Option<String>,
    },
    StopRecording,
    Ping,
}

impl Request {
    pub fn parse(line: &str) -> Result<Self, String> {
        serde_json::from_str(line).map_err(|e| e.to_string())
    }
}

pub struct Reply(serde_json::Value);

impl Reply {
    pub fn ok(mut v: serde_json::Value) -> Self {
        v["ok"] = serde_json::Value::Bool(true);
        Reply(v)
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Reply(serde_json::json!({ "ok": false, "error": msg.into() }))
    }
    pub fn to_line(&self) -> String {
        format!("{}\n", self.0)
    }
}

pub fn navigate_js(route: &str) -> String {
    format!(
        "window.location.assign({})",
        serde_json::to_string(route).unwrap()
    )
}

/// The onboarding status the `onboarding_step` command saves: still in progress, at
/// `step`, with models marked "downloaded" once the step is past the model-download
/// steps (>3) so the driver can jump straight to any later screen.
pub fn onboarding_status_for_step(step: u8) -> crate::onboarding::OnboardingStatus {
    let done = step > 3;
    let m = |b: bool| if b { "downloaded" } else { "not_downloaded" }.to_string();
    crate::onboarding::OnboardingStatus {
        version: "1.0".into(),
        completed: false,
        current_step: step,
        model_status: crate::onboarding::ModelStatus {
            parakeet: m(done),
            summary: m(done),
            selected_summary_model: done.then(|| "qwen3.5:2b".to_string()),
        },
        last_updated: chrono::Utc::now().to_rfc3339(),
    }
}

/// Spawns the control listener when `NIXON_DEV_CONTROL=1` and the bundle is a `.debug`
/// build. Called once from `lib.rs` `setup`, right after the specs/0059 dev hooks.
pub fn spawn_if_requested(app: AppHandle) {
    if !guard::env_flag(guard::ENV_DEV_CONTROL) {
        return;
    }
    if !guard::allowed("NIXON_DEV_CONTROL") {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(e) => {
                log::error!("[dev] control bind failed: {e}");
                return;
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let port_file = crate::app_paths::app_data_dir().join(PORT_FILE);
        if let Err(e) = std::fs::write(&port_file, port.to_string()) {
            log::error!("[dev] control port file: {e}");
            return;
        }
        log::info!(
            "[dev] control listener on 127.0.0.1:{port} ({})",
            port_file.display()
        );
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                continue;
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let (r, mut w) = sock.into_split();
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let reply = match Request::parse(&line) {
                        Ok(req) => handle(&app, req).await,
                        Err(e) => Reply::err(e),
                    };
                    if w.write_all(reply.to_line().as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
}

async fn handle(app: &AppHandle, req: Request) -> Reply {
    let Some(win) = app.get_webview_window("main") else {
        return Reply::err("no main window");
    };
    match req {
        Request::Ping => Reply::ok(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") })),
        Request::Navigate { route } => win
            .eval(navigate_js(&route))
            .map(|_| Reply::ok(serde_json::json!({})))
            .unwrap_or_else(|e| Reply::err(e.to_string())),
        Request::Theme { value } => {
            if value != "light" && value != "dark" {
                return Reply::err("theme must be light|dark");
            }
            win.eval(format!(
                "localStorage.setItem('nixon.theme', '{value}'); location.reload()"
            ))
            .map(|_| Reply::ok(serde_json::json!({})))
            .unwrap_or_else(|e| Reply::err(e.to_string()))
        }
        Request::OnboardingStep { value } => {
            if !(1..=5).contains(&value) {
                return Reply::err("step 1..5");
            }
            match crate::onboarding::save_onboarding_status(app, &onboarding_status_for_step(value))
                .await
            {
                Ok(()) => {
                    let _ = win.eval("location.reload()");
                    Reply::ok(serde_json::json!({}))
                }
                Err(e) => Reply::err(e.to_string()),
            }
        }
        Request::OnboardingComplete => {
            match crate::onboarding::save_onboarding_status(
                app,
                &super::seeded_onboarding_status(chrono::Utc::now()),
            )
            .await
            {
                Ok(()) => {
                    let _ = win.eval("location.reload()");
                    Reply::ok(serde_json::json!({}))
                }
                Err(e) => Reply::err(e.to_string()),
            }
        }
        Request::Resize { w, h } => win
            .set_size(tauri::LogicalSize::new(w, h))
            .map(|_| Reply::ok(serde_json::json!({})))
            .unwrap_or_else(|e| Reply::err(e.to_string())),
        Request::HideDevBadge { value } => win
            .eval(format!(
                "document.documentElement.dataset.shot = '{}'",
                if value { "1" } else { "" }
            ))
            .map(|_| Reply::ok(serde_json::json!({})))
            .unwrap_or_else(|e| Reply::err(e.to_string())),
        Request::Ready { timeout_ms } => {
            let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(10_000));
            let t0 = std::time::Instant::now();
            SHOT_READY.store(false, Ordering::SeqCst);
            while t0.elapsed() < timeout {
                let _ = win.eval(
                    "window.__TAURI_INTERNALS__.invoke('dev_shot_ping', { ready: document.documentElement.dataset.shotReady === '1' })",
                );
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                if SHOT_READY.load(Ordering::SeqCst) {
                    return Reply::ok(serde_json::json!({ "ms": t0.elapsed().as_millis() as u64 }));
                }
            }
            Reply::err("not ready before timeout")
        }
        Request::Window => window_info(&win),
        Request::StartRecording { title } => {
            start_recording(app, title.unwrap_or_else(|| "Screenshot take".into())).await
        }
        Request::StopRecording => stop_recording_and_discard(app).await,
    }
}

#[cfg(target_os = "macos")]
fn window_info(win: &tauri::WebviewWindow) -> Reply {
    use objc::{msg_send, sel, sel_impl};
    let Ok(ns) = win.ns_window() else {
        return Reply::err("ns_window unavailable");
    };
    // `objc`'s `sel_impl!` macro (expanded from `msg_send!`) checks
    // `cfg(feature = "cargo-clippy")`, which this workspace's `--check-cfg` doesn't
    // declare — a false positive on every `msg_send!` call site in the `objc` 0.2.x
    // crate, unrelated to this function's own code. An `#[allow]` on the enclosing
    // `let`/fn doesn't reach it (the diagnostic is anchored to the macro expansion
    // itself), so this is silenced crate-wide in `Cargo.toml` (`[lints.rust]`) instead.
    let number: i64 = unsafe { msg_send![ns as *mut objc::runtime::Object, windowNumber] };
    let pos = win.outer_position().map(|p| (p.x, p.y)).unwrap_or((0, 0));
    let size = win
        .outer_size()
        .map(|s| (s.width, s.height))
        .unwrap_or((0, 0));
    let scale = win.scale_factor().unwrap_or(1.0);
    Reply::ok(serde_json::json!({
        "window_number": number,
        "x": pos.0,
        "y": pos.1,
        "w": size.0,
        "h": size.1,
        "scale": scale,
    }))
}
#[cfg(not(target_os = "macos"))]
fn window_info(_: &tauri::WebviewWindow) -> Reply {
    Reply::err("macOS only")
}

/// `start_recording` command: same internal entry point the real `start_recording`
/// Tauri command delegates to (`audio/capture_commands.rs`), with no device override
/// and no meeting to resume — every screenshot take is a fresh, throwaway recording.
async fn start_recording(app: &AppHandle, title: String) -> Reply {
    match crate::audio::recording_commands::start_recording_with_devices_and_meeting(
        app.clone(),
        None,
        None,
        Some(title),
        None,
        None,
    )
    .await
    {
        Ok(()) => Reply::ok(serde_json::json!({})),
        Err(e) => Reply::err(e),
    }
}

/// `stop_recording` command: stops via the same internal entry point
/// `capture_commands::stop_recording` delegates to, then discards the throwaway
/// recording — its meeting row (if the frontend's own stop handling already saved
/// one) and its on-disk folder, when that folder lives under `recordings_root()`.
///
/// NOTE (specs/0060 review): this runs the exact `stop_recording` path the real command
/// uses, which only unloads the STT engine and emits `recording-stopped` — it never
/// touches the database itself. Persisting the meeting (and any downstream
/// summary/diarization kick-off) is driven by the frontend's `useRecordingStop` hook
/// reacting to that same event *in the live window this listener is driving* — so a
/// `start_recording` + `stop_recording` round trip over this socket exercises the whole
/// normal save pipeline, not a stub. The cleanup below is a best-effort compensating
/// delete for whatever the frontend had persisted by the time this returns; a driver
/// that stops immediately after should still poll/re-check rather than assume the
/// frontend's save already landed.
async fn stop_recording_and_discard(app: &AppHandle) -> Reply {
    let args = crate::audio::recording_commands::RecordingArgs {
        save_path: String::new(),
    };
    let stop_info = match crate::audio::recording_commands::stop_recording(app.clone(), args).await
    {
        Ok(v) => v,
        Err(e) => return Reply::err(e),
    };
    let folder_path = stop_info
        .get("folder_path")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    if let Some(folder_path) = &folder_path {
        if let Some(state) = app.try_state::<crate::state::AppState>() {
            let pool = state.db_manager.pool();
            match sqlx::query_scalar::<_, String>("SELECT id FROM meetings WHERE folder_path = ?")
                .bind(folder_path)
                .fetch_optional(pool)
                .await
            {
                Ok(Some(id)) => {
                    if let Err(e) = super::seed::remove_meeting_rows(pool, &id).await {
                        log::error!("[dev] control stop_recording: row cleanup failed: {e:#}");
                    }
                }
                Ok(None) => {}
                Err(e) => log::error!("[dev] control stop_recording: meeting lookup failed: {e}"),
            }
        }

        let root = crate::audio::recordings_root();
        let path = std::path::Path::new(folder_path);
        if path.starts_with(&root) {
            if let Err(e) = std::fs::remove_dir_all(path) {
                log::error!("[dev] control stop_recording: folder cleanup failed: {e}");
            }
        }
    }

    Reply::ok(serde_json::json!({ "folder_path": folder_path }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_and_rejects_unknown() {
        assert!(matches!(
            Request::parse(r#"{"cmd":"navigate","route":"/meetings"}"#).unwrap(),
            Request::Navigate { .. }
        ));
        // `w`/`h` are `f64`, and float literals in patterns are a hard error (E0308 on
        // an integer literal, and the `illegal_floating_point_literal_pattern` lint on a
        // float one) — the brief's verbatim pattern doesn't compile, so this destructures
        // and compares by value instead.
        match Request::parse(r#"{"cmd":"resize","w":1280,"h":860}"#).unwrap() {
            Request::Resize { w, h } => {
                assert_eq!(w, 1280.0);
                assert_eq!(h, 860.0);
            }
            other => panic!("expected Resize, got {other:?}"),
        }
        assert!(Request::parse(r#"{"cmd":"format_disk"}"#).is_err());
        assert!(Request::parse("not json").is_err());
    }

    #[test]
    fn onboarding_status_for_step_marks_models_by_step() {
        let s3 = onboarding_status_for_step(3);
        assert!(!s3.completed);
        assert_eq!(s3.current_step, 3);
        assert_eq!(s3.model_status.parakeet, "not_downloaded");
        let s4 = onboarding_status_for_step(4);
        assert_eq!(s4.model_status.parakeet, "downloaded");
    }

    #[test]
    fn navigate_js_escapes_the_route() {
        assert_eq!(
            navigate_js("/a?b=1&c=\"x\""),
            "window.location.assign(\"/a?b=1&c=\\\"x\\\"\")"
        );
    }

    #[test]
    fn reply_serialises_ok_and_error() {
        assert_eq!(
            Reply::ok(serde_json::json!({"ms": 5})).to_line(),
            "{\"ms\":5,\"ok\":true}\n"
        );
        assert_eq!(
            Reply::err("nope").to_line(),
            "{\"error\":\"nope\",\"ok\":false}\n"
        );
    }
}
