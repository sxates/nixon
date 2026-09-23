//! A way for the frontend to write into the app's log file.
//!
//! Nixon's most stubborn bugs live in frontend orchestration — the deferred-backlog drain in
//! particular, which sequences retranscribe → diarize → summarize entirely in TypeScript.
//! None of its decisions reached `~/Library/Logs/<bundle-id>/Nixon.log`, so when a
//! recording's automatic processing stopped after the retranscription step (2026-09-21), the
//! log showed the Rust work succeeding and then simply nothing. `console.log` is no help: by
//! the time anyone asks, the devtools console is gone, and on a bundled build there is no
//! console at all.
//!
//! So the frontend gets one command that writes to the same file, at the same level filter,
//! with an explicit `[fe]` marker so it is greppable and obviously not Rust's own output.

use serde::Deserialize;

/// Levels the frontend may use. Anything unrecognised is treated as `info` rather than
/// rejected — a diagnostic that refuses to record itself is worse than a mislevelled one.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrontendLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Write a frontend message into the app log.
///
/// Deliberately infallible from the caller's point of view: it returns `()`, so an
/// instrumentation call can never introduce a failure path into the flow it is observing.
#[tauri::command]
pub fn api_log_frontend(level: Option<FrontendLevel>, scope: String, message: String) {
    let line = format!("[fe:{scope}] {message}");
    match level.unwrap_or(FrontendLevel::Info) {
        FrontendLevel::Debug => log::debug!("{line}"),
        FrontendLevel::Info => log::info!("{line}"),
        FrontendLevel::Warn => log::warn!("{line}"),
        FrontendLevel::Error => log::error!("{line}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend sends whatever it sends; an unknown or absent level must still record.
    #[test]
    fn an_absent_level_still_logs() {
        api_log_frontend(None, "test".into(), "no level given".into());
    }

    #[test]
    fn every_level_is_accepted() {
        for raw in ["\"debug\"", "\"info\"", "\"warn\"", "\"error\""] {
            let level: FrontendLevel =
                serde_json::from_str(raw).unwrap_or_else(|e| panic!("{raw} should parse: {e}"));
            api_log_frontend(Some(level), "test".into(), format!("level {raw}"));
        }
    }
}
