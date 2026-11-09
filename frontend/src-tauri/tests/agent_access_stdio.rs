//! specs/0087 Task 6 — `nixon --mcp`, the read-only stdio MCP server.
//!
//! Spawns the real binary against a seeded, migrated temp data dir and speaks
//! newline-delimited JSON-RPC over its stdin/stdout. Every read is bounded by a timeout so a
//! hung server fails the test instead of hanging CI.
//!
//! Run with:
//!   cargo test --features metal --test agent_access_stdio

mod agent_access_fixture;
mod common;

use app_lib::agent_access::log::{AccessEntry, AccessLog};
use app_lib::agent_access::settings::{self, AgentAccessSettings};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const OFF: &str = "Nixon's agent access is turned off. Turn it on in Nixon › Settings.";
const BEHIND: &str = "Open Nixon once to finish updating, then try again.";
const NO_DB: &str = "Nixon has no meetings here yet. Open Nixon and record a meeting first.";
const CLIENT: &str = "stdio-test-client";
const READ_TIMEOUT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// Fixture data dir
// ---------------------------------------------------------------------------

/// A data dir holding the seeded meetings DB and an `agent_access.json` with the given
/// switch. The DB is seeded in a staging dir and copied out with `VACUUM INTO`: a pool's
/// `close()` returns before SQLite's own close (which checkpoints into the file) has run on
/// sqlx's worker thread, so a DB seeded in place would still be changing when a test hashes
/// it. The copy is switched back to WAL (as the app runs it) on one connection whose
/// `close()` waits for SQLite to let go.
async fn data_dir(enabled: bool) -> tempfile::TempDir {
    use sqlx::Connection;
    let (_stage, db) = agent_access_fixture::setup().await;
    let dir = tempfile::tempdir().unwrap();
    sqlx::query("VACUUM INTO ?")
        .bind(db_path(dir.path()).to_str().unwrap())
        .execute(db.pool())
        .await
        .unwrap();
    db.pool().close().await;
    let mut conn =
        sqlx::SqliteConnection::connect(&format!("sqlite:{}", db_path(dir.path()).display()))
            .await
            .unwrap();
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
    assert!(!dir.path().join("meeting_minutes.sqlite-wal").exists());
    set_enabled(dir.path(), enabled);
    dir
}

fn set_enabled(dir: &Path, enabled: bool) {
    settings::save(
        dir,
        &AgentAccessSettings {
            enabled,
            ..Default::default()
        },
    )
    .unwrap();
}

fn db_path(dir: &Path) -> PathBuf {
    dir.join("meeting_minutes.sqlite")
}

async fn log_entries(dir: &Path) -> Vec<AccessEntry> {
    AccessLog::open(dir).await.unwrap().list(100).await.unwrap()
}

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

// ---------------------------------------------------------------------------
// A minimal newline-delimited JSON-RPC client
// ---------------------------------------------------------------------------

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    /// Every stdout line seen so far, in order.
    seen: Vec<String>,
    next_id: u64,
}

impl Server {
    fn spawn(dir: &Path) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nixon"))
            .args(["--mcp", "--data-dir"])
            .arg(dir)
            .args(["--link-scheme", "nixon-dev"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn nixon --mcp");
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Server {
            stdin: child.stdin.take(),
            child,
            lines,
            seen: Vec::new(),
            next_id: 1,
        }
    }

    fn send(&mut self, msg: Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    fn next_line(&mut self) -> String {
        let line = self
            .lines
            .recv_timeout(READ_TIMEOUT)
            .expect("server answered within the timeout");
        self.seen.push(line.clone());
        line
    }

    /// Sends a request and returns its response, skipping any notifications.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self.next_line();
            let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| {
                panic!("stdout line is not JSON ({e}): {line}");
            });
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    fn handshake(&mut self) -> Value {
        let r = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": CLIENT, "version": "1.0"}
            }),
        );
        assert!(r.get("result").is_some(), "initialize failed: {r}");
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        r
    }

    /// Calls a tool; returns (text, isError).
    fn call(&mut self, tool: &str, args: Value) -> (String, bool) {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let result = r
            .get("result")
            .unwrap_or_else(|| panic!("tools/call returned a JSON-RPC error: {r}"));
        let text = result["content"]
            .as_array()
            .expect("content array")
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        (text, result["isError"].as_bool().unwrap_or(false))
    }

    /// Closes stdin and waits for the process to exit; drains the remaining stdout lines.
    fn finish(mut self) -> (i32, Vec<String>) {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + READ_TIMEOUT;
        let status = loop {
            if let Some(s) = self.child.try_wait().unwrap() {
                break s;
            }
            if std::time::Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("server did not exit after stdin closed");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        while let Ok(line) = self.lines.recv_timeout(Duration::from_secs(2)) {
            self.seen.push(line);
        }
        (status.code().unwrap_or(-1), std::mem::take(&mut self.seen))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn initialize_then_list_tools() {
    let dir = data_dir(true).await;
    let mut s = Server::spawn(dir.path());
    let init = s.handshake();
    assert_eq!(init["result"]["serverInfo"]["name"], "nixon-dev", "{init}");
    let r = s.request("tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().expect("tools array");
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["get_meeting", "list_meetings", "search_meetings"]);
    let desc = |n: &str| {
        tools.iter().find(|t| t["name"] == n).unwrap()["description"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(desc("search_meetings").starts_with("Search all of the user's recorded meetings"));
    assert!(desc("search_meetings").contains("cite those links"));
    assert!(desc("get_meeting").contains("follow the cursor to read on."));
    let search = tools
        .iter()
        .find(|t| t["name"] == "search_meetings")
        .unwrap();
    assert_eq!(
        search["inputSchema"]["required"],
        json!(["query"]),
        "{search}"
    );
    let (code, _) = s.finish();
    assert_eq!(code, 0);
}

#[tokio::test]
async fn off_refuses_and_logs() {
    let dir = data_dir(false).await;
    let mut s = Server::spawn(dir.path());
    s.handshake();
    let (text, is_error) = s.call("search_meetings", json!({"query": "pricing"}));
    assert_eq!(text, OFF);
    assert!(!is_error);
    s.finish();
    let log = log_entries(dir.path()).await;
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0].outcome, "off");
    assert_eq!(log[0].tool, "search_meetings");
    assert_eq!(log[0].client.as_deref(), Some(CLIENT));
    assert_eq!(log[0].args["query"], "pricing");
    assert!(log[0].meeting_ids.is_empty());
}

#[tokio::test]
async fn on_search_returns_links() {
    let dir = data_dir(true).await;
    let mut s = Server::spawn(dir.path());
    s.handshake();
    let (text, is_error) = s.call("search_meetings", json!({"query": "pricing"}));
    assert!(!is_error, "{text}");
    assert!(
        text.contains("nixon-dev://meeting/m-pricing?t=754.2"),
        "{text}"
    );
    s.finish();
    let log = log_entries(dir.path()).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, "ok");
    assert!(log[0].meeting_ids.contains(&"m-pricing".to_string()));
}

#[tokio::test]
async fn list_and_get_work_and_log_meetings_read() {
    let dir = data_dir(true).await;
    let mut s = Server::spawn(dir.path());
    s.handshake();
    let (list, _) = s.call("list_meetings", json!({"person": "dana"}));
    assert!(list.contains("nixon-dev://meeting/m-pricing"), "{list}");
    let (get, is_error) = s.call(
        "get_meeting",
        json!({"meeting_id": "m-pricing", "from_seconds": 750, "to_seconds": 805}),
    );
    assert!(!is_error, "{get}");
    assert!(get.contains("The pricing page needs work"), "{get}");
    assert!(!get.contains("Welcome everyone"), "{get}");
    s.finish();
    let log = log_entries(dir.path()).await;
    assert_eq!(log.len(), 2);
    assert!(log.iter().all(|e| e.outcome == "ok"));
    let get_row = log.iter().find(|e| e.tool == "get_meeting").unwrap();
    assert_eq!(get_row.meeting_ids, vec!["m-pricing".to_string()]);
    assert_eq!(get_row.args["from_seconds"], json!(750.0));
}

#[tokio::test]
async fn bad_arguments_are_error_results_and_unknown_id_is_not() {
    let dir = data_dir(true).await;
    let mut s = Server::spawn(dir.path());
    s.handshake();
    let (text, is_error) = s.call(
        "get_meeting",
        json!({"meeting_id": "m-pricing", "include": ["audio"]}),
    );
    assert!(is_error, "{text}");
    assert!(text.contains("Unknown include"), "{text}");
    let (text, is_error) = s.call(
        "search_meetings",
        json!({"query": "pricing", "from_date": "last tuesday"}),
    );
    assert!(is_error, "{text}");
    assert!(text.contains("YYYY-MM-DD"), "{text}");
    let (text, is_error) = s.call("get_meeting", json!({"meeting_id": "nope"}));
    assert!(!is_error, "{text}");
    assert!(text.contains("No meeting with id nope"), "{text}");
    s.finish();
    let mut outcomes: Vec<String> = log_entries(dir.path())
        .await
        .into_iter()
        .map(|e| e.outcome)
        .collect();
    outcomes.sort();
    assert_eq!(outcomes, ["error", "error", "ok"]);
}

#[tokio::test]
async fn toggle_takes_effect_without_restart() {
    let dir = data_dir(true).await;
    let mut s = Server::spawn(dir.path());
    s.handshake();
    let (text, _) = s.call("search_meetings", json!({"query": "pricing"}));
    assert!(text.contains("nixon-dev://meeting/"), "{text}");
    set_enabled(dir.path(), false);
    let (text, _) = s.call("search_meetings", json!({"query": "pricing"}));
    assert_eq!(text, OFF);
    set_enabled(dir.path(), true);
    let (text, _) = s.call("list_meetings", json!({}));
    assert!(text.contains("nixon-dev://meeting/"), "{text}");
    s.finish();
}

#[tokio::test]
async fn schema_behind_refuses() {
    let dir = data_dir(true).await;
    {
        let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path(dir.path()).display()))
            .await
            .unwrap();
        sqlx::query(
            "DELETE FROM _sqlx_migrations WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }
    let mut s = Server::spawn(dir.path());
    s.handshake();
    let (text, is_error) = s.call("list_meetings", json!({}));
    assert_eq!(text, BEHIND);
    assert!(!is_error);
    s.finish();
    let log = log_entries(dir.path()).await;
    assert_eq!(log[0].outcome, "needs_update");
}

#[tokio::test]
async fn missing_db_refuses() {
    let dir = tempfile::tempdir().unwrap();
    set_enabled(dir.path(), true);
    let mut s = Server::spawn(dir.path());
    s.handshake();
    let (text, is_error) = s.call("get_meeting", json!({"meeting_id": "m-pricing"}));
    assert_eq!(text, NO_DB);
    assert!(!is_error);
    s.finish();
    assert!(
        !db_path(dir.path()).exists(),
        "the server must not create the meetings DB"
    );
    let log = log_entries(dir.path()).await;
    assert_eq!(log[0].outcome, "no_db");
}

#[tokio::test]
async fn never_writes_meetings_db() {
    let dir = data_dir(true).await;
    // W0: a read-only reader may recreate -wal/-shm beside the DB (harmless), so only the
    // main file is compared.
    let before = sha256(&db_path(dir.path()));
    let header = std::fs::read(db_path(dir.path())).unwrap();
    assert_eq!(
        &header[18..20],
        &[2, 2],
        "fixture DB is in WAL mode, like the app's"
    );
    let mut s = Server::spawn(dir.path());
    s.handshake();
    s.request("tools/list", json!({}));
    s.call("search_meetings", json!({"query": "pricing"}));
    s.call("list_meetings", json!({}));
    s.call("get_meeting", json!({"meeting_id": "m-pricing"}));
    s.finish();
    assert_eq!(sha256(&db_path(dir.path())), before);
}

#[tokio::test]
async fn read_only_pool_rejects_writes() {
    let dir = data_dir(true).await;
    let pool = app_lib::agent_access::entry::open_meetings_pool(dir.path())
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 3);
    let insert = sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('x', 'x', 'x', 'x')",
    )
    .execute(&pool)
    .await;
    assert!(insert.is_err(), "an INSERT through the MCP pool must fail");
    pool.close().await;
}

#[tokio::test]
async fn stdout_is_only_jsonrpc() {
    let dir = data_dir(true).await;
    let mut s = Server::spawn(dir.path());
    s.handshake();
    s.request("tools/list", json!({}));
    s.call("search_meetings", json!({"query": "pricing"}));
    s.call("list_meetings", json!({}));
    s.call("get_meeting", json!({"meeting_id": "m-pricing"}));
    s.call(
        "get_meeting",
        json!({"meeting_id": "m-pricing", "include": ["x"]}),
    );
    set_enabled(dir.path(), false);
    s.call("list_meetings", json!({}));
    let (code, lines) = s.finish();
    assert_eq!(code, 0);
    assert!(lines.len() >= 7, "{lines:?}");
    for line in &lines {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("non-JSON on stdout ({e}): {line}"));
        assert_eq!(v["jsonrpc"], "2.0", "{line}");
    }
}

#[test]
fn bad_args_exit_2() {
    for args in [
        vec!["--mcp"],
        vec!["--mcp", "--data-dir", "/tmp/x"],
        vec!["--mcp", "--data-dir", "/tmp/x", "--link-scheme", "http"],
        vec!["--mcp", "--link-scheme", "nixon"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_nixon"))
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert!(!out.stderr.is_empty(), "{args:?}");
    }
}
