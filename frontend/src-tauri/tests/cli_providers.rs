//! Subscription CLI provider tests (specs/0085), against the fake CLI only.
//!
//! `tests/fixtures/cli/fake-ai-cli.sh` stands in for `claude` / `codex`; it logs its argv,
//! pwd, env and a SHA-256 of stdin to `$FAKE_CLI_LOG`, so isolation is checked from the
//! child's side. No test runs a real CLI.
//!
//! Covered here (runner parts): acceptance 1 (argv carries no prompt byte, cwd is an empty
//! dir under a 0700 temp dir that is gone afterwards, env holds only the allowlist),
//! 3 (5 MB through stdin in < 5 s), 4 (timeout / cancel / kill_all take the grandchild
//! down within 3 s), the output cap, and a crash exit code.
//!
//! Needs a debug build (the default for `cargo test`): the fake's mode and log reach it
//! through the runner's `extra_env`, which only exists under `cfg(any(test,
//! debug_assertions))`. `cargo test --release` does not compile this file.
//!
//! Run with:
//!   cargo test --features metal --test cli_providers

use app_lib::cli_llm::errors::CliError;
use app_lib::cli_llm::runner::{self, Invocation, RunOutput, IN_DIR_TOKEN, WORK_DIR_TOKEN};
use app_lib::cli_llm::CliKind;
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// `kill_all()` kills every live run in the process, so its test takes this exclusively
/// and every other spawning test takes it shared.
static KILL_ALL_LOCK: Lazy<RwLock<()>> = Lazy::new(|| RwLock::new(()));

const PROMPT_SENTINEL: &str = "SENTINEL-PROMPT-7f3a synthetic prompt body";

fn fake_cli() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli/fake-ai-cli.sh")
}

/// A per-test log file outside the runner's temp dir.
fn log_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("fake-cli.log")
}

fn invocation<'a>(
    binary: &'a Path,
    mode: &str,
    log: &Path,
    stdin: Vec<u8>,
    timeout: Duration,
) -> Invocation<'a> {
    let mut inv = Invocation::new(CliKind::ClaudeCode, binary, timeout);
    inv.stdin = stdin;
    inv.extra_env = vec![
        ("FAKE_CLI_MODE".into(), mode.into()),
        ("FAKE_CLI_LOG".into(), log.display().to_string()),
    ];
    inv
}

/// Lines of the log whose key is `key` ("ARG", "ENV", ...), values only.
fn log_values(log: &str, key: &str) -> Vec<String> {
    let prefix = format!("{key}:");
    log.lines()
        .filter_map(|l| l.strip_prefix(&prefix).map(str::to_string))
        .collect()
}

fn read_log(path: &Path) -> String {
    std::fs::read_to_string(path).expect("fake CLI wrote no log")
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn wait_gone(pid: i32, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    !alive(pid)
}

async fn wait_for_grandchild(log: &Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(log) {
            if let Some(pid) = log_values(&text, "GRANDCHILD").first() {
                return pid.parse().expect("grandchild pid");
            }
        }
        assert!(
            Instant::now() < deadline,
            "the hang fake never started its grandchild"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn ok(result: Result<RunOutput, CliError>) -> RunOutput {
    result.unwrap_or_else(|e| panic!("run failed: {e:?}"))
}

#[tokio::test]
async fn isolation_argv_cwd_env_and_temp_dir() {
    let _guard = KILL_ALL_LOCK.read().await;
    // Keys a user might have exported; the child must see none of them.
    for key in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "GEMINI_API_KEY",
        "NIXON_TEST_STRAY_VAR",
    ] {
        std::env::set_var(key, "must-not-leak");
    }

    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let mut inv = invocation(
        &binary,
        "success",
        &log,
        PROMPT_SENTINEL.as_bytes().to_vec(),
        Duration::from_secs(30),
    );
    inv.args = vec![
        OsString::from("-p"),
        OsString::from("--system-prompt-file"),
        OsString::from(format!("{IN_DIR_TOKEN}/system.txt")),
        OsString::from("-C"),
        OsString::from(WORK_DIR_TOKEN),
    ];
    inv.files = vec![("system.txt", b"SYSTEM-SENTINEL-be41".to_vec())];
    inv.env = vec![("DISABLE_AUTOUPDATER".into(), "1".into())];

    let out = ok(runner::run(inv).await);
    assert!(out.status.success());
    assert_eq!(out.stdout, b"fake reply\n");
    let text = read_log(&log);

    // argv: no byte of the prompt, and the `{in}` / `{work}` tokens resolved to absolute
    // paths inside the run's temp dir.
    let args = log_values(&text, "ARG");
    for a in &args {
        assert!(
            !a.contains("SENTINEL"),
            "prompt or file content leaked into argv: {a}"
        );
    }
    let pwd = PathBuf::from(&log_values(&text, "PWD")[0]);
    let tmp_root = pwd.parent().unwrap().to_path_buf();
    assert_eq!(pwd.file_name().unwrap(), "work");
    assert!(
        tmp_root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("nixon-cli-"),
        "{tmp_root:?}"
    );
    let system_file = tmp_root.join("in/system.txt");
    assert_eq!(args[2], system_file.display().to_string());
    assert_eq!(PathBuf::from(&args[4]), pwd);

    // stdin arrived intact.
    let sha = hex(&Sha256::digest(PROMPT_SENTINEL.as_bytes()));
    assert_eq!(log_values(&text, "STDIN_SHA256"), vec![sha]);

    // cwd was empty, the temp dir 0700, and all of it is gone now.
    assert_eq!(log_values(&text, "PWD_ENTRIES"), vec!["0"]);
    assert_eq!(log_values(&text, "TMPDIR_MODE"), vec!["700"]);
    assert!(
        !tmp_root.exists(),
        "temp dir survived the call: {tmp_root:?}"
    );

    // env: only the allowlist (plus what `sh` itself sets, and the test hook).
    let allowed: BTreeSet<&str> = [
        "HOME",
        "USER",
        "LOGNAME",
        "PATH",
        "LANG",
        "TMPDIR",
        "TERM",
        "NO_COLOR",
        "DISABLE_AUTOUPDATER",
        "FAKE_CLI_MODE",
        "FAKE_CLI_LOG",
        // set by the shell running the fake, not by the runner
        "PWD",
        "OLDPWD",
        "SHLVL",
        "_",
    ]
    .into_iter()
    .collect();
    let env: Vec<(String, String)> = log_values(&text, "ENV")
        .into_iter()
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
        .collect();
    for (k, v) in &env {
        assert!(
            allowed.contains(k.as_str()),
            "env var off the allowlist: {k}"
        );
        assert_ne!(v, "must-not-leak", "{k} leaked into the child");
    }
    let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    assert_eq!(get("TMPDIR").map(PathBuf::from), Some(tmp_root.clone()));
    assert_eq!(get("TERM").as_deref(), Some("dumb"));
    assert_eq!(get("NO_COLOR").as_deref(), Some("1"));
    assert_eq!(get("LANG").as_deref(), Some("en_US.UTF-8"));
    assert_eq!(get("DISABLE_AUTOUPDATER").as_deref(), Some("1"));
    let path = get("PATH").unwrap();
    let bin_dir = binary.parent().unwrap().display().to_string();
    assert!(path.starts_with(&format!("{bin_dir}:")), "{path}");
    assert!(path.ends_with("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"));
    assert_eq!(get("HOME"), std::env::var("HOME").ok());
}

#[tokio::test]
async fn temp_dir_is_removed_on_failure_too() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    // 2 s, not a few hundred ms: the fake must have started (and logged its cwd) before
    // the timeout, even on a loaded CI machine.
    let inv = invocation(&binary, "hang", &log, vec![], Duration::from_secs(2));
    assert_eq!(runner::run(inv).await.unwrap_err(), CliError::Timeout);
    wait_for_grandchild(&log).await;
    let pwd = PathBuf::from(&log_values(&read_log(&log), "PWD")[0]);
    assert!(!pwd.parent().unwrap().exists());
}

#[tokio::test]
async fn five_megabytes_round_trip_through_stdin() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let prompt: Vec<u8> = (0..5 * 1024 * 1024)
        .map(|i| b'a' + (i % 26) as u8)
        .collect();
    let inv = invocation(
        &binary,
        "echo",
        &log,
        prompt.clone(),
        Duration::from_secs(30),
    );

    let started = Instant::now();
    let out = ok(runner::run(inv).await);
    let took = started.elapsed();
    assert!(out.status.success());
    assert!(
        out.stdout == prompt,
        "echoed {} bytes, sent {}",
        out.stdout.len(),
        prompt.len()
    );
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

#[tokio::test]
async fn timeout_kills_the_whole_group() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    // Long enough that the fake has started its grandchild before the timeout fires.
    let timeout = Duration::from_secs(2);
    let inv = invocation(&binary, "hang", &log, vec![], timeout);

    let started = Instant::now();
    let result = runner::run(inv).await;
    let took = started.elapsed();
    assert_eq!(result.unwrap_err(), CliError::Timeout);
    assert!(took < timeout + Duration::from_secs(3), "took {took:?}");
    let grandchild = wait_for_grandchild(&log).await;
    assert!(
        wait_gone(grandchild, Duration::from_secs(1)).await,
        "grandchild {grandchild} survived the timeout"
    );
}

#[tokio::test]
async fn cancel_kills_the_whole_group() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let cancel = CancellationToken::new();
    let mut inv = invocation(&binary, "hang", &log, vec![], Duration::from_secs(60));
    inv.cancel = Some(&cancel);

    let (result, cancelled_at) = tokio::join!(runner::run(inv), async {
        wait_for_grandchild(&log).await;
        cancel.cancel();
        Instant::now()
    });
    assert_eq!(result.unwrap_err(), CliError::Cancelled);
    assert!(cancelled_at.elapsed() < Duration::from_secs(3));
    let grandchild = wait_for_grandchild(&log).await;
    assert!(
        wait_gone(grandchild, Duration::from_secs(1)).await,
        "grandchild {grandchild} survived cancel"
    );
}

#[tokio::test]
async fn kill_all_leaves_no_registered_group_alive() {
    let _guard = KILL_ALL_LOCK.write().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let inv = invocation(&binary, "hang", &log, vec![], Duration::from_secs(60));

    let (_result, (pgids, grandchild)) = tokio::join!(runner::run(inv), async {
        let grandchild = wait_for_grandchild(&log).await;
        let pgids = runner::live_pgids();
        assert!(!pgids.is_empty(), "the hung run was never registered");
        runner::kill_all();
        (pgids, grandchild)
    });
    assert!(
        wait_gone(grandchild, Duration::from_secs(3)).await,
        "grandchild survived"
    );
    for pgid in pgids {
        // SAFETY: signal 0 to a process group only probes for existence.
        let group_alive = unsafe { libc::killpg(pgid, 0) == 0 };
        assert!(!group_alive, "group {pgid} survived kill_all");
    }
    assert!(runner::live_pgids().is_empty());
}

#[tokio::test]
async fn sigkill_fallback_removes_a_grandchild_that_ignores_sigterm() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let cancel = CancellationToken::new();
    let mut inv = invocation(
        &binary,
        "hang-ignore-term",
        &log,
        vec![],
        Duration::from_secs(60),
    );
    inv.cancel = Some(&cancel);

    let (result, (grandchild, cancelled_at)) = tokio::join!(runner::run(inv), async {
        let grandchild = wait_for_grandchild(&log).await;
        cancel.cancel();
        (grandchild, Instant::now())
    });
    assert_eq!(result.unwrap_err(), CliError::Cancelled);
    let took = cancelled_at.elapsed();
    // SIGTERM did nothing to it, so the run waited out the grace and sent SIGKILL.
    assert!(took >= Duration::from_millis(1500), "took {took:?}");
    assert!(
        wait_gone(grandchild, Duration::from_millis(2500).saturating_sub(took)).await,
        "TERM-ignoring grandchild {grandchild} survived the SIGKILL fallback"
    );
}

#[tokio::test]
async fn a_grandchild_holding_stdout_does_not_hold_up_a_finished_run() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let inv = invocation(&binary, "orphan", &log, vec![], Duration::from_secs(30));

    let started = Instant::now();
    let out = ok(runner::run(inv).await);
    let took = started.elapsed();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"fake reply\n");
    assert!(took < Duration::from_secs(3), "took {took:?}");
    let grandchild = wait_for_grandchild(&log).await;
    assert!(
        wait_gone(grandchild, Duration::from_secs(1)).await,
        "grandchild {grandchild} outlived the finished run"
    );
}

#[tokio::test]
async fn leftover_group_members_are_killed_after_a_normal_exit() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let inv = invocation(&binary, "linger", &log, vec![], Duration::from_secs(30));

    let out = ok(runner::run(inv).await);
    assert!(out.status.success());
    let grandchild = wait_for_grandchild(&log).await;
    assert!(
        wait_gone(grandchild, Duration::from_secs(1)).await,
        "grandchild {grandchild} outlived the finished run"
    );
}

#[tokio::test]
async fn an_unexecutable_binary_is_not_found_not_a_crash() {
    // Passes the runner's is_file check, but `nice` can't exec it and exits 127: the same
    // thing a CLI that vanished between discovery and spawn does.
    let scratch = tempfile::tempdir().unwrap();
    let binary = scratch.path().join("claude");
    std::fs::write(&binary, "#!/nonexistent/nixon-test/interpreter\n").unwrap();
    std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let inv = Invocation::new(CliKind::ClaudeCode, &binary, Duration::from_secs(5));
    assert_eq!(runner::run(inv).await.unwrap_err(), CliError::NotFound);
}

#[tokio::test]
async fn flood_hits_the_output_cap() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let inv = invocation(&binary, "flood", &log, vec![], Duration::from_secs(30));
    assert_eq!(
        runner::run(inv).await.unwrap_err(),
        CliError::OutputTooLarge
    );
}

#[tokio::test]
async fn crash_reports_the_exit_code() {
    let _guard = KILL_ALL_LOCK.read().await;
    let scratch = tempfile::tempdir().unwrap();
    let log = log_path(&scratch);
    let binary = fake_cli();
    let inv = invocation(&binary, "crash", &log, vec![], Duration::from_secs(30));
    let out = ok(runner::run(inv).await);
    assert_eq!(out.exit_code(), 2);
    assert_eq!(out.stderr, b"fake crash\n");
    assert_eq!(out.crash_error(), Some(CliError::Crashed { code: 2 }));
}

#[tokio::test]
async fn missing_binary_is_not_found() {
    let missing = Path::new("/nonexistent/nixon-test/claude");
    let inv = Invocation::new(CliKind::ClaudeCode, missing, Duration::from_secs(5));
    assert_eq!(runner::run(inv).await.unwrap_err(), CliError::NotFound);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
