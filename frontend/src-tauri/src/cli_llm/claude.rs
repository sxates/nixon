//! The Claude Code adapter (specs/0085): one `claude -p` call per LLM request.
//!
//! Isolation is by flags ([`args`]): no tools, no MCP servers, no settings, hooks, project
//! files or session history. The model is pinned to [`MODEL`] (owner decision). Claude Code
//! only reports what it loaded (`system/init`) *after* it has read the prompt, so the check
//! is a tripwire: [`Tripwire`] watches stdout as it arrives, and a bad init SIGKILLs the
//! call before the model's answer (seconds later) can be used. The answer is then read
//! from the last `result` event ([`interpret`]); failures are classified on `is_error`, the
//! assistant `error` field and the result text, never on `subtype` (a signed-out run says
//! `subtype: "success"`).
//!
//! The prompt only travels on stdin (one stream-json user message) and in `in/system.txt`,
//! never in argv. Nothing from stdout or stderr is logged.

use super::errors::CliError;
use super::runner::{self, Invocation, IN_DIR_TOKEN};
use super::CliKind;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The model alias every call uses (owner, 2026-11-05: pinned, no picker).
pub const MODEL: &str = "sonnet";
/// The one tool a `--json-schema` run is allowed: Claude's internal structured-output call.
const STRUCTURED_OUTPUT_TOOL: &str = "StructuredOutput";
/// A whole summary takes ~6-9 s; this is the spec's ceiling for one call.
pub const TIMEOUT: Duration = Duration::from_secs(300);
const SYSTEM_FILE: &str = "system.txt";

/// One Claude Code call.
pub struct Call<'a> {
    pub binary: &'a Path,
    pub system: &'a str,
    pub user: &'a str,
    /// When set, the reply is the validated `structured_output`, re-serialized as JSON.
    pub json_schema: Option<&'a Value>,
    pub cancel: Option<&'a CancellationToken>,
    pub timeout: Duration,
    /// Forwarded to the runner's test-only `extra_env` (the fake CLI's mode and log).
    /// Ignored in release builds.
    pub test_env: Vec<(String, String)>,
}

/// Production entry: runs one call with the default timeout and returns the reply text
/// (or, with `json_schema`, the structured output as a JSON string).
pub async fn generate(
    binary: &Path,
    system: &str,
    user: &str,
    json_schema: Option<&Value>,
    cancel: Option<&CancellationToken>,
) -> Result<String, CliError> {
    generate_with(Call {
        binary,
        system,
        user,
        json_schema,
        cancel,
        timeout: TIMEOUT,
        test_env: Vec::new(),
    })
    .await
}

/// [`generate`] with every knob exposed (tests set the timeout and the fake's env).
pub async fn generate_with(call: Call<'_>) -> Result<String, CliError> {
    let schema = call.json_schema.is_some();
    let mut inv = Invocation::new(CliKind::ClaudeCode, call.binary, call.timeout);
    inv.args = args(call.json_schema);
    inv.stdin = stdin_message(call.user);
    inv.files = vec![(SYSTEM_FILE, call.system.as_bytes().to_vec())];
    inv.env = env();
    inv.cancel = call.cancel;
    let mut tripwire = Tripwire::new(schema);
    inv.on_stdout_line = Some(Box::new(move |line| tripwire.check_line(line)));
    #[cfg(any(test, debug_assertions))]
    {
        inv.extra_env = call.test_env;
    }
    #[cfg(not(any(test, debug_assertions)))]
    let _ = call.test_env;

    let out = runner::run(inv).await?;
    let mut result = interpret(&out.stdout, out.exit_code(), schema);
    // 127 with no stream at all: the CLI's own wrapper couldn't exec (`env: node: No such
    // file`). The install is broken, so report NotFound and let discovery look again.
    if result == Err(CliError::Crashed { code: 127 }) {
        result = Err(CliError::NotFound);
    }
    if let Err(e) = &result {
        log::warn!(
            "cli_llm: Claude Code call failed: {} (exit {})",
            e.log_label(),
            out.exit_code()
        );
        #[cfg(debug_assertions)]
        if *e == CliError::Unknown && !out.stderr.is_empty() {
            let head: String = String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(200)
                .collect();
            log::debug!("cli_llm: Claude Code stderr (first 200 chars): {head}");
        }
    }
    result
}

/// The argv, in the spec's order. `{in}` becomes the run's input dir. The schema is small
/// and holds no meeting text, so it may go in argv.
pub fn args(json_schema: Option<&Value>) -> Vec<OsString> {
    let system_file = format!("{IN_DIR_TOKEN}/{SYSTEM_FILE}");
    let mut args: Vec<OsString> = [
        "-p",
        "--safe-mode",
        "--restricted",
        "--tools",
        "",
        "--disallowedTools",
        "mcp__*",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--no-session-persistence",
        "--max-turns",
        "1",
        "--system-prompt-file",
        &system_file,
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        MODEL,
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    if let Some(schema) = json_schema {
        args.push("--json-schema".into());
        args.push(schema.to_string().into());
    }
    args
}

/// Env extras on top of the runner's allowlist: no self-update, no telemetry or other
/// non-essential traffic, no prompt history.
pub fn env() -> Vec<(String, String)> {
    [
        ("DISABLE_AUTOUPDATER", "1"),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
        ("CLAUDE_CODE_SKIP_PROMPT_HISTORY", "1"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// stdin: one stream-json user message, newline-terminated. serde does the escaping.
pub fn stdin_message(user: &str) -> Vec<u8> {
    let message = json!({
        "type": "user",
        "message": { "role": "user", "content": user },
    });
    let mut line = message.to_string().into_bytes();
    line.push(b'\n');
    line
}

/// Watches stdout while the call runs. Fails the call (and so kills it) on a `system/init`
/// that shows tools, MCP servers or non-built-in plugins, and on an `api_retry` for an auth
/// failure (Claude would otherwise retry for minutes).
pub struct Tripwire {
    schema: bool,
    init_seen: bool,
}

impl Tripwire {
    pub fn new(schema: bool) -> Self {
        Self {
            schema,
            init_seen: false,
        }
    }

    pub fn check_line(&mut self, line: &[u8]) -> Result<(), CliError> {
        let Ok(event) = serde_json::from_slice::<Value>(line) else {
            return Ok(());
        };
        if event_type(&event) != Some("system") {
            return Ok(());
        }
        match event.get("subtype").and_then(Value::as_str) {
            Some("init") if !self.init_seen => {
                self.init_seen = true;
                check_init(&event, self.schema)
            }
            Some("api_retry") if error_code(&event) == Some("authentication_failed") => {
                Err(CliError::NotSignedIn)
            }
            _ => Ok(()),
        }
    }
}

/// The isolation allowlist (W0): tools exactly `[]` (or `["StructuredOutput"]` on a schema
/// run), no MCP servers, and only built-in plugins. A missing field fails: what can't be
/// checked isn't trusted.
pub fn check_init(init: &Value, schema: bool) -> Result<(), CliError> {
    let tools_ok = match init.get("tools").and_then(Value::as_array) {
        Some(tools) if tools.is_empty() => true,
        Some(tools) => {
            schema && tools.len() == 1 && tools[0].as_str() == Some(STRUCTURED_OUTPUT_TOOL)
        }
        None => false,
    };
    let mcp_ok = init
        .get("mcp_servers")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty);
    let plugins_ok = init
        .get("plugins")
        .and_then(Value::as_array)
        .is_some_and(|plugins| {
            plugins
                .iter()
                .all(|p| p.get("path").and_then(Value::as_str) == Some("builtin"))
        });
    if tools_ok && mcp_ok && plugins_ok {
        return Ok(());
    }
    // Which check failed, never the names (a plugin name could be anything the user chose).
    log::error!(
        "cli_llm: Claude Code started without isolation (tools ok: {tools_ok}, MCP ok: \
         {mcp_ok}, plugins ok: {plugins_ok}); stopping it"
    );
    Err(CliError::IsolationFailed)
}

/// What the finished stream says. Pure, so the W0 captures can be replayed against it.
pub fn interpret(stdout: &[u8], exit_code: i32, schema: bool) -> Result<String, CliError> {
    let events: Vec<Value> = stdout
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(Value::is_object)
        .collect();
    let init = events
        .iter()
        .find(|e| event_type(e) == Some("system") && subtype(e) == Some("init"));
    let result = events
        .iter()
        .rev()
        .find(|e| event_type(e) == Some("result"));

    let Some(init) = init else {
        // No init means isolation was never confirmed, so even an answer is discarded.
        return Err(match result {
            None if exit_code != 0 => CliError::Crashed { code: exit_code },
            _ => CliError::BadOutput,
        });
    };
    check_init(init, schema)?;

    if let Some(result) = result {
        if result.get("is_error").and_then(Value::as_bool) == Some(false) {
            return reply(result, schema);
        }
    }
    if let Some(error) = classify_failure(&events) {
        return Err(error);
    }
    Err(match result {
        Some(_) => CliError::Unknown,
        None if exit_code != 0 => CliError::Crashed { code: exit_code },
        None => CliError::BadOutput,
    })
}

/// A successful `result`'s answer.
fn reply(result: &Value, schema: bool) -> Result<String, CliError> {
    if schema {
        return match result.get("structured_output") {
            Some(output) if !output.is_null() => Ok(output.to_string()),
            _ => Err(CliError::BadOutput),
        };
    }
    match result.get("result").and_then(Value::as_str).map(str::trim) {
        Some(text) if !text.is_empty() => Ok(text.to_string()),
        _ => Err(CliError::BadOutput),
    }
}

/// "You've hit your session/weekly/Sonnet limit(s)": a plan limit. "hit your rate limit" is
/// the transient kind and is excluded ([`is_plan_limit_text`]).
static HIT_YOUR_LIMIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bhit your\b.*\blimits?\b").unwrap());
static HIT_YOUR_RATE_LIMIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bhit your rate limits?\b").unwrap());
/// The reset hint, up to the first sentence end, "·" or parenthesis, so trailing upgrade
/// text never reaches the user copy. A "." followed by a digit ("3.45pm") is not an end.
static RESETS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bresets?\b(?:[^\n().!?\u{00B7}]|\.\d)*").unwrap());
const AUTH_TEXT: [&str; 4] = [
    "not logged in",
    "login expired",
    "authentication required",
    "oauth token",
];

/// Classifies a failed stream: not signed in, then plan limit, then transient rate limit.
fn classify_failure(events: &[Value]) -> Option<CliError> {
    let mut codes: Vec<&str> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut statuses: Vec<i64> = Vec::new();
    let mut rejected: Option<Option<i64>> = None;
    for event in events {
        match (event_type(event), subtype(event)) {
            (Some("assistant"), _) => {
                if let Some(code) = error_code(event) {
                    codes.push(code);
                    texts.extend(message_text(event));
                }
            }
            (Some("system"), Some("api_retry")) => {
                codes.extend(error_code(event));
                statuses.extend(event.get("error_status").and_then(Value::as_i64));
            }
            (Some("rate_limit_event"), _) => {
                let info = event.get("rate_limit_info");
                let status = info.and_then(|i| i.get("status")).and_then(Value::as_str);
                if status == Some("rejected") {
                    rejected = Some(info.and_then(|i| i.get("resetsAt")).and_then(Value::as_i64));
                }
            }
            (Some("result"), _) => {
                texts.extend(
                    event
                        .get("result")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                );
                statuses.extend(event.get("api_error_status").and_then(Value::as_i64));
            }
            _ => {}
        }
    }
    let lower: Vec<String> = texts.iter().map(|t| t.to_lowercase()).collect();
    let any_text = |needle: &str| lower.iter().any(|t| t.contains(needle));

    if codes.contains(&"authentication_failed") || AUTH_TEXT.iter().any(|n| any_text(n)) {
        return Some(CliError::NotSignedIn);
    }
    let plan_limit = texts.iter().any(|t| is_plan_limit_text(t))
        || any_text("credit balance is too low")
        || codes.contains(&"billing_error");
    if plan_limit || rejected.is_some() {
        let detail = texts
            .iter()
            .find_map(|t| RESETS.find(t))
            .map(|m| m.as_str().trim().trim_end_matches('.').to_string())
            .or_else(|| rejected.flatten().and_then(format_reset))
            .unwrap_or_default();
        return Some(CliError::UsageLimit { detail });
    }
    let transient = codes
        .iter()
        .any(|c| c.contains("rate_limit") || c.contains("overloaded"))
        || statuses.iter().any(|s| *s == 429 || *s == 529)
        || ["rate limit", "overloaded", "429"]
            .iter()
            .any(|n| any_text(n));
    transient.then_some(CliError::RateLimited)
}

fn is_plan_limit_text(text: &str) -> bool {
    HIT_YOUR_LIMIT.is_match(text) && !HIT_YOUR_RATE_LIMIT.is_match(text)
}

/// `resetsAt` (Unix seconds) as "resets 3:45pm", or with the date when it isn't today.
fn format_reset(unix: i64) -> Option<String> {
    use chrono::{Local, TimeZone};
    let at = Local.timestamp_opt(unix, 0).single()?;
    let time = at.format("%-I:%M%P");
    if at.date_naive() == Local::now().date_naive() {
        Some(format!("resets {time}"))
    } else {
        Some(format!("resets {} {time}", at.format("%b %-d")))
    }
}

fn event_type(event: &Value) -> Option<&str> {
    event.get("type").and_then(Value::as_str)
}

fn subtype(event: &Value) -> Option<&str> {
    event.get("subtype").and_then(Value::as_str)
}

fn error_code(event: &Value) -> Option<&str> {
    event.get("error").and_then(Value::as_str)
}

/// The text blocks of an assistant event's message.
fn message_text(event: &Value) -> Vec<String> {
    event
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
