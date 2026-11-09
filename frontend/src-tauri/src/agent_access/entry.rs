//! specs/0087 — the `nixon --mcp` entry point: argument parsing, the tokio runtime, and the
//! read-only meetings pool. `main.rs` calls in here before `app_lib::run()`, so this mode
//! never starts Tauri, its logger, or the single-instance lock.
//!
//! stdout carries only MCP JSON-RPC; every diagnostic here goes to stderr.

use anyhow::{Context, Result};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The migrations this binary was built against. Only read for their versions: `--mcp`
/// mode never runs them.
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub const DB_FILE: &str = "meeting_minutes.sqlite";
const SCHEMES: [&str; 2] = ["nixon", "nixon-dev"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpArgs {
    pub data_dir: PathBuf,
    /// `nixon` (production) or `nixon-dev`; also the MCP server name.
    pub link_scheme: String,
}

/// `None` when `--mcp` is absent (normal app launch). Otherwise the parsed arguments, or
/// a message for stderr naming what is missing.
pub fn parse_mcp_args(args: &[String]) -> Option<Result<McpArgs, String>> {
    if !args.iter().any(|a| a == "--mcp") {
        return None;
    }
    Some(parse_flags(args))
}

fn parse_flags(args: &[String]) -> Result<McpArgs, String> {
    let value = |flag: &str| -> Result<Option<String>, String> {
        match args.iter().position(|a| a == flag) {
            None => Ok(None),
            Some(i) => match args.get(i + 1).filter(|v| !v.starts_with("--")) {
                Some(v) => Ok(Some(v.clone())),
                None => Err(format!("{flag} needs a value.")),
            },
        }
    };
    const USAGE: &str = "usage: nixon --mcp --data-dir <path> --link-scheme <nixon|nixon-dev>";
    let data_dir = value("--data-dir")?
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| format!("--mcp needs --data-dir. {USAGE}"))?;
    let link_scheme =
        value("--link-scheme")?.ok_or_else(|| format!("--mcp needs --link-scheme. {USAGE}"))?;
    if !SCHEMES.contains(&link_scheme.as_str()) {
        return Err(format!(
            "Unknown --link-scheme \"{link_scheme}\": use nixon or nixon-dev."
        ));
    }
    Ok(McpArgs {
        data_dir: PathBuf::from(data_dir),
        link_scheme,
    })
}

/// Serves MCP over stdio until stdin closes; returns the process exit code.
pub fn run_stdio(args: McpArgs) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("nixon --mcp: could not start: {e}");
            return 1;
        }
    };
    match runtime.block_on(super::server::serve(args)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("nixon --mcp: {e:#}");
            1
        }
    }
}

/// Opens the meetings DB read-only. Never creates it and never runs migrations; a
/// read-only reader may still create the `-wal`/`-shm` side files, which is harmless.
pub async fn open_meetings_pool(data_dir: &Path) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(data_dir.join(DB_FILE))
        .read_only(true)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
        .context("Couldn't open Nixon's meetings database")
}

/// True when the DB has every migration this binary knows about. False when the app was
/// updated but not reopened since (or the migrations table is missing). A DB ahead of the
/// binary is fine: migrations are additive.
pub async fn schema_is_current(pool: &SqlitePool) -> bool {
    let expected = MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0);
    let applied: Result<Option<i64>, _> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(pool)
            .await;
    matches!(applied, Ok(Some(v)) if v >= expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        std::iter::once("nixon")
            .chain(a.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn absent_flag_is_normal_launch() {
        assert_eq!(parse_mcp_args(&args(&[])), None);
        assert_eq!(parse_mcp_args(&args(&["--data-dir", "/x"])), None);
    }

    #[test]
    fn parses_both_flags_in_any_order() {
        let want = McpArgs {
            data_dir: PathBuf::from("/data dir"),
            link_scheme: "nixon-dev".into(),
        };
        let a = args(&[
            "--mcp",
            "--data-dir",
            "/data dir",
            "--link-scheme",
            "nixon-dev",
        ]);
        assert_eq!(parse_mcp_args(&a), Some(Ok(want.clone())));
        let b = args(&[
            "--link-scheme",
            "nixon-dev",
            "--data-dir",
            "/data dir",
            "--mcp",
        ]);
        assert_eq!(parse_mcp_args(&b), Some(Ok(want)));
    }

    #[test]
    fn rejects_missing_or_bad_values() {
        for a in [
            args(&["--mcp"]),
            args(&["--mcp", "--data-dir", "/x"]),
            args(&["--mcp", "--link-scheme", "nixon"]),
            args(&["--mcp", "--data-dir", "--link-scheme", "nixon"]),
            args(&["--mcp", "--data-dir", "/x", "--link-scheme", "https"]),
            args(&["--mcp", "--data-dir", "/x", "--link-scheme"]),
        ] {
            assert!(matches!(parse_mcp_args(&a), Some(Err(_))), "{a:?}");
        }
    }

    #[test]
    fn expected_schema_version_is_known() {
        assert!(MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0) > 0);
    }
}
