//! Runs one subscription-CLI call as a sandboxed one-shot subprocess (specs/0085).
//!
//! Each call gets:
//! - a private temp dir `<std temp>/nixon-cli-XXXX/` (mode 0700) holding `work/` (empty,
//!   the child's cwd) and `in/` (the invocation's files: system prompt, schema). It is
//!   removed when `run` returns, on success, error, timeout and cancel alike;
//! - a cleared environment plus a short allowlist, so a stray `ANTHROPIC_API_KEY` or
//!   `CODEX_API_KEY` can never move billing off the user's subscription;
//! - its own process group under `nice -n 10`, registered in [`LIVE`] so a timeout, a
//!   cancel or app quit ([`kill_all`]) takes down the CLI *and* its children (npm-installed
//!   CLIs are a `node` process with children).
//!
//! The prompt only ever travels through stdin or `in/` files. Args name `in/` files with
//! [`IN_DIR_TOKEN`] (and the cwd with [`WORK_DIR_TOKEN`]); the runner substitutes the
//! absolute paths once the temp dir exists.
//!
//! Privacy: logs carry the kind, binary, args, exit code, duration and error label, never
//! stdin, stdout, stderr or `in/` file contents.

use super::errors::CliError;
use super::CliKind;
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;
use tokio_util::sync::CancellationToken;

/// In an arg, replaced by the absolute path of the run's `in/` dir (`{in}/system.txt`).
pub const IN_DIR_TOKEN: &str = "{in}";
/// In an arg, replaced by the absolute path of the run's (empty) cwd.
pub const WORK_DIR_TOKEN: &str = "{work}";

/// stdout past this is not a reply; the run is killed with `OutputTooLarge`.
pub const STDOUT_CAP: usize = 16 * 1024 * 1024;
/// stderr cap, same treatment.
pub const STDERR_CAP: usize = 1024 * 1024;
/// SIGTERM → SIGKILL grace on timeout / cancel / overflow.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// Once the CLI itself has exited, how long its pipes get to drain before a leftover
/// child still holding them is killed (rather than the run waiting out its timeout).
const DRAIN_GRACE: Duration = Duration::from_millis(500);
/// How long [`kill_group`] keeps re-signalling a group that hasn't gone yet.
const KILL_SETTLE: Duration = Duration::from_millis(500);
/// `nice`'s exit code when it couldn't exec the binary (gone, or a broken shebang).
const NICE_EXEC_FAILED: i32 = 127;

/// Appended to the binary's own directory (which resolves `node` for nvm, Volta and
/// Homebrew installs) to form the child's `PATH`.
pub(crate) const BASE_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// Process groups of runs still in flight, for [`kill_all`] at app quit.
static LIVE: Lazy<Mutex<HashSet<i32>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// Sees each stdout line (newline stripped) as it arrives, while the call runs. An `Err`
/// stops the call at once: the process group is SIGKILLed with no SIGTERM grace, and
/// `run` fails with that error. Claude's init tripwire is one.
pub type LineHook<'a> = Box<dyn FnMut(&[u8]) -> Result<(), CliError> + Send + 'a>;

pub struct Invocation<'a> {
    /// Which CLI this is (for logs).
    pub kind: CliKind,
    pub binary: &'a Path,
    /// May contain [`IN_DIR_TOKEN`] / [`WORK_DIR_TOKEN`]. Never prompt text.
    pub args: Vec<OsString>,
    pub stdin: Vec<u8>,
    /// Written into `<tmp>/in/<name>` before spawn. Names are plain file names.
    pub files: Vec<(&'static str, Vec<u8>)>,
    /// Adapter env extras (e.g. Claude's `DISABLE_AUTOUPDATER=1`), set after the allowlist.
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub cancel: Option<&'a CancellationToken>,
    pub on_stdout_line: Option<LineHook<'a>>,
    /// Test hook only (the fake CLI's `FAKE_CLI_MODE` / `FAKE_CLI_LOG`); production never
    /// sets it.
    #[cfg(any(test, debug_assertions))]
    pub extra_env: Vec<(String, String)>,
}

impl<'a> Invocation<'a> {
    /// An invocation with no args, stdin, files or env extras.
    pub fn new(kind: CliKind, binary: &'a Path, timeout: Duration) -> Self {
        Self {
            kind,
            binary,
            args: Vec::new(),
            stdin: Vec::new(),
            files: Vec::new(),
            env: Vec::new(),
            timeout,
            cancel: None,
            on_stdout_line: None,
            #[cfg(any(test, debug_assertions))]
            extra_env: Vec::new(),
        }
    }
}

/// A finished run. A non-zero exit is still `Ok`: the adapter classifies stdout/stderr
/// first (an auth failure exits 1 with the reason on stdout) and falls back to
/// [`RunOutput::crash_error`].
#[derive(Debug)]
pub struct RunOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
}

impl RunOutput {
    /// The exit code; a signal death reads shell-style as 128 + signal.
    pub fn exit_code(&self) -> i32 {
        exit_code(&self.status)
    }

    /// `Crashed { code }` for a non-zero exit, `None` for success.
    pub fn crash_error(&self) -> Option<CliError> {
        (!self.status.success()).then(|| CliError::Crashed {
            code: self.exit_code(),
        })
    }
}

/// Runs `inv` to completion, timeout or cancel. See the module docs for the sandbox.
pub async fn run(inv: Invocation<'_>) -> Result<RunOutput, CliError> {
    let kind = inv.kind;
    let result = run_inner(inv).await;
    if let Err(e) = &result {
        log::warn!("cli_llm: {kind:?} call failed: {}", e.log_label());
    }
    result
}

async fn run_inner(inv: Invocation<'_>) -> Result<RunOutput, CliError> {
    let kind = inv.kind;
    if !inv.binary.is_file() {
        log::warn!("cli_llm: binary not found at {}", inv.binary.display());
        return Err(CliError::NotFound);
    }

    // Dropped (and deleted) when this function returns, after the child is reaped.
    let tmp = tempfile::Builder::new()
        .prefix("nixon-cli-")
        .tempdir()
        .map_err(io_failure("create the temp dir"))?;
    // Canonical (macOS temp lives behind the /var → /private/var symlink), so args, cwd
    // and TMPDIR all name the same real path.
    let root = tmp
        .path()
        .canonicalize()
        .map_err(io_failure("resolve the temp dir"))?;
    let in_dir = root.join("in");
    let work_dir = root.join("work");
    prepare_dirs(&root, &in_dir, &work_dir, &inv.files)?;

    let args: Vec<OsString> = inv
        .args
        .into_iter()
        .map(|a| substitute(a, &in_dir, &work_dir))
        .collect();
    log::info!(
        "cli_llm: running {:?} {} {:?}",
        inv.kind,
        inv.binary.display(),
        args
    );

    let mut command = base_command(inv.binary);
    command
        .args(&args)
        .current_dir(&work_dir)
        .env_clear()
        .envs(base_env(inv.binary, &root))
        .envs(inv.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(any(test, debug_assertions))]
    command.envs(inv.extra_env);
    #[cfg(unix)]
    command.process_group(0);

    let started = Instant::now();
    let mut child = command.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CliError::NotFound
        } else {
            io_failure("spawn the CLI")(e)
        }
    })?;
    let pgid = child.id().map(|id| id as i32).ok_or(CliError::Unknown)?;
    let mut group = LiveGroup::register(pgid);

    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(CliError::Unknown);
    };

    let hook_stopped = AtomicBool::new(false);
    let mut hook = inv.on_stdout_line;
    let outcome = {
        let io = async {
            // Concurrently: a prompt bigger than the pipe buffer would otherwise deadlock
            // against a child that writes before it has read all of stdin.
            let pipes = async {
                tokio::try_join!(
                    read_stdout(stdout, hook.as_mut(), &hook_stopped),
                    read_capped(stderr, STDERR_CAP),
                    write_stdin(stdin, &inv.stdin),
                )
            };
            tokio::pin!(pipes);
            let status = tokio::select! {
                r = &mut pipes => {
                    let (out, err, ()) = r?;
                    let status = child.wait().await.map_err(io_failure("wait for the CLI"))?;
                    return Ok::<_, CliError>((status, out, err));
                }
                s = child.wait() => s.map_err(io_failure("wait for the CLI"))?,
            };
            // The CLI exited but its pipes are still open. Give them a moment to drain; a
            // leftover child still holding them after that is killed, so a finished run
            // doesn't wait out its timeout. The group outlives the reaped leader only
            // while a member exists, so its pgid can't have been reused.
            let (out, err, ()) = match tokio::time::timeout(DRAIN_GRACE, &mut pipes).await {
                Ok(r) => r?,
                Err(_) => {
                    log::warn!(
                        "cli_llm: {:?} exited but a child kept its output open",
                        kind
                    );
                    kill_group(pgid, None).await;
                    pipes.await?
                }
            };
            Ok((status, out, err))
        };
        let cancelled = async {
            match inv.cancel {
                Some(token) => token.cancelled().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            r = io => r,
            _ = tokio::time::sleep(inv.timeout) => Err(CliError::Timeout),
            _ = cancelled => Err(CliError::Cancelled),
        }
    };

    match outcome {
        Ok((status, stdout, stderr)) => {
            // The leader is reaped. Anything left in its group (a CLI's background helper)
            // dies with the call; a pgid is never reused while its group still exists.
            if group_exists(pgid) {
                kill_group(pgid, None).await;
            }
            group.reaped();
            if exit_code(&status) == NICE_EXEC_FAILED && stderr.starts_with(b"nice:") {
                log::warn!(
                    "cli_llm: {} could not be started (gone or not runnable)",
                    inv.binary.display()
                );
                return Err(CliError::NotFound);
            }
            let duration = started.elapsed();
            log::info!(
                "cli_llm: {:?} exited with code {} after {:?}",
                inv.kind,
                exit_code(&status),
                duration
            );
            Ok(RunOutput {
                status,
                stdout,
                stderr,
                duration,
            })
        }
        Err(e) => {
            if hook_stopped.load(Ordering::SeqCst) {
                kill_group(pgid, Some(&mut child)).await;
            } else {
                terminate_group(pgid, &mut child).await;
            }
            let _ = child.wait().await;
            group.reaped();
            Err(e)
        }
    }
}

/// SIGTERM to the group, up to [`KILL_GRACE`] for it to go, then SIGKILL.
async fn terminate_group(pgid: i32, child: &mut Child) {
    signal_group(pgid, Signal::Term);
    let deadline = Instant::now() + KILL_GRACE;
    loop {
        // Reap the leader as soon as it dies, so a zombie doesn't keep the group "alive".
        let _ = child.try_wait();
        if !group_exists(pgid) {
            return;
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    kill_group(pgid, Some(child)).await;
}

/// SIGKILLs the group, and again until it is gone (up to [`KILL_SETTLE`]): a member that
/// was mid-`fork` when the first signal went out can come out of it unsignalled.
async fn kill_group(pgid: i32, mut child: Option<&mut Child>) {
    let deadline = Instant::now() + KILL_SETTLE;
    loop {
        signal_group(pgid, Signal::Kill);
        if let Some(child) = child.as_deref_mut() {
            // Reap the leader, so its zombie doesn't read as a live group.
            let _ = child.try_wait();
        }
        if !group_exists(pgid) || Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// SIGKILLs every in-flight run's process group. Called at app quit.
pub fn kill_all() {
    let pgids: Vec<i32> = live_pgids();
    if !pgids.is_empty() {
        log::info!(
            "cli_llm: killing {} running CLI call(s) on quit",
            pgids.len()
        );
    }
    for pgid in pgids {
        signal_group(pgid, Signal::Kill);
    }
}

/// Process groups currently registered (in-flight runs).
pub fn live_pgids() -> Vec<i32> {
    LIVE.lock()
        .map(|live| live.iter().copied().collect())
        .unwrap_or_default()
}

/// Keeps a run's pgid in [`LIVE`] until the leader is reaped. If the `run` future is
/// dropped mid-call, the group is SIGKILLed: `kill_on_drop` alone only reaches the leader.
struct LiveGroup {
    pgid: i32,
    reaped: bool,
}

impl LiveGroup {
    fn register(pgid: i32) -> Self {
        if let Ok(mut live) = LIVE.lock() {
            live.insert(pgid);
        }
        Self {
            pgid,
            reaped: false,
        }
    }

    fn reaped(&mut self) {
        self.reaped = true;
        if let Ok(mut live) = LIVE.lock() {
            live.remove(&self.pgid);
        }
    }
}

impl Drop for LiveGroup {
    fn drop(&mut self) {
        if !self.reaped {
            // The leader is unreaped (at worst a zombie), so the pgid can't have been reused.
            signal_group(self.pgid, Signal::Kill);
            if let Ok(mut live) = LIVE.lock() {
                live.remove(&self.pgid);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Signal {
    Term,
    Kill,
}

#[cfg(unix)]
fn signal_group(pgid: i32, signal: Signal) {
    let sig = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // SAFETY: killpg has no memory-safety preconditions; a stale pgid just yields ESRCH.
    unsafe {
        libc::killpg(pgid, sig);
    }
}

#[cfg(not(unix))]
fn signal_group(_pgid: i32, _signal: Signal) {}

#[cfg(unix)]
fn group_exists(pgid: i32) -> bool {
    // SAFETY: signal 0 only checks for existence.
    let rc = unsafe { libc::killpg(pgid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn group_exists(_pgid: i32) -> bool {
    false
}

/// `nice -n 10 <binary>` like the llama sidecar; the process group still covers the child.
fn base_command(binary: &Path) -> tokio::process::Command {
    #[cfg(unix)]
    {
        let mut command = tokio::process::Command::new("/usr/bin/nice");
        command.arg("-n").arg("10").arg(binary);
        command
    }
    #[cfg(not(unix))]
    {
        tokio::process::Command::new(binary)
    }
}

/// The allowlist: identity, a fixed PATH, locale, the run's TMPDIR, and no colour.
fn base_env(binary: &Path, tmp_root: &Path) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    for key in ["HOME", "USER", "LOGNAME"] {
        if let Some(value) = std::env::var_os(key) {
            env.push((key.into(), value));
        }
    }
    let mut path = OsString::new();
    if let Some(dir) = binary.parent().filter(|d| !d.as_os_str().is_empty()) {
        path.push(dir.as_os_str());
        path.push(":");
    }
    path.push(BASE_PATH);
    env.push(("PATH".into(), path));
    env.push(("LANG".into(), "en_US.UTF-8".into()));
    env.push(("TMPDIR".into(), tmp_root.as_os_str().to_owned()));
    env.push(("TERM".into(), "dumb".into()));
    env.push(("NO_COLOR".into(), "1".into()));
    env
}

fn prepare_dirs(
    root: &Path,
    in_dir: &Path,
    work_dir: &Path,
    files: &[(&'static str, Vec<u8>)],
) -> Result<(), CliError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(io_failure("lock down the temp dir"))?;
    }
    std::fs::create_dir(in_dir).map_err(io_failure("create in/"))?;
    std::fs::create_dir(work_dir).map_err(io_failure("create work/"))?;
    for (name, contents) in files {
        if !is_plain_file_name(name) {
            log::error!("cli_llm: refusing input file name {name:?}");
            return Err(CliError::Unknown);
        }
        std::fs::write(in_dir.join(name), contents).map_err(io_failure("write an input file"))?;
    }
    Ok(())
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/')
}

/// Replaces [`IN_DIR_TOKEN`] / [`WORK_DIR_TOKEN`] in one arg.
fn substitute(arg: OsString, in_dir: &Path, work_dir: &Path) -> OsString {
    match arg.to_str() {
        Some(s) if s.contains(IN_DIR_TOKEN) || s.contains(WORK_DIR_TOKEN) => s
            .replace(IN_DIR_TOKEN, &in_dir.to_string_lossy())
            .replace(WORK_DIR_TOKEN, &work_dir.to_string_lossy())
            .into(),
        _ => arg,
    }
}

/// [`read_capped`] for stdout, feeding each complete line to `hook` as it arrives (and a
/// final unterminated one at EOF). A hook error sets `stopped` and ends the read.
async fn read_stdout(
    mut reader: impl AsyncRead + Unpin,
    mut hook: Option<&mut LineHook<'_>>,
    stopped: &AtomicBool,
) -> Result<Vec<u8>, CliError> {
    let Some(hook) = hook.as_mut() else {
        return read_capped(reader, STDOUT_CAP).await;
    };
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut line_start = 0;
    // Where the newline search resumes, so a long line arriving in many chunks is scanned
    // once, not once per chunk.
    let mut scanned = 0;
    loop {
        let n = reader
            .read(&mut chunk)
            .await
            .map_err(io_failure("read the CLI's output"))?;
        if buf.len() + n > STDOUT_CAP {
            return Err(CliError::OutputTooLarge);
        }
        buf.extend_from_slice(&chunk[..n]);
        let mut feed =
            |line: &[u8]| hook(line).inspect_err(|_| stopped.store(true, Ordering::SeqCst));
        if n == 0 {
            if line_start < buf.len() {
                feed(&buf[line_start..])?;
            }
            return Ok(buf);
        }
        while let Some(nl) = buf[scanned..].iter().position(|b| *b == b'\n') {
            let end = scanned + nl;
            feed(&buf[line_start..end])?;
            line_start = end + 1;
            scanned = line_start;
        }
        scanned = buf.len();
    }
}

async fn read_capped(mut reader: impl AsyncRead + Unpin, cap: usize) -> Result<Vec<u8>, CliError> {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .await
            .map_err(io_failure("read the CLI's output"))?;
        if n == 0 {
            return Ok(buf);
        }
        if buf.len() + n > cap {
            return Err(CliError::OutputTooLarge);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Writes all of stdin at once (a CLI given held-back stdin can give up), then closes it.
/// A child that exits without reading everything is not an error here; its exit says why.
async fn write_stdin(mut stdin: tokio::process::ChildStdin, bytes: &[u8]) -> Result<(), CliError> {
    if let Err(e) = stdin.write_all(bytes).await {
        log::debug!("cli_llm: stdin closed early ({:?})", e.kind());
    }
    drop(stdin);
    Ok(())
}

fn exit_code(status: &ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

fn io_failure(what: &'static str) -> impl Fn(std::io::Error) -> CliError {
    move |e| {
        log::error!("cli_llm: failed to {what}: {e}");
        CliError::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_resolves_both_tokens() {
        let in_dir = Path::new("/t/nixon-cli-x/in");
        let work = Path::new("/t/nixon-cli-x/work");
        let got = |s: &str| substitute(s.into(), in_dir, work);
        assert_eq!(got("{in}/system.txt"), "/t/nixon-cli-x/in/system.txt");
        assert_eq!(got("{work}"), "/t/nixon-cli-x/work");
        assert_eq!(
            got("--x={in}/a:{work}"),
            "--x=/t/nixon-cli-x/in/a:/t/nixon-cli-x/work"
        );
        assert_eq!(got("--model"), "--model");
    }

    #[test]
    fn input_file_names_must_be_plain() {
        assert!(is_plain_file_name("system.txt"));
        for bad in ["", ".", "..", "../x", "a/b"] {
            assert!(!is_plain_file_name(bad), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn line_hook_reassembles_lines_split_across_reads() {
        // A 16-byte pipe forces every line to arrive in several reads.
        let (mut tx, rx) = tokio::io::duplex(16);
        let long = "x".repeat(100);
        let body = format!("{long}\nshort\n\nlast-no-newline");
        let writer = tokio::spawn({
            let body = body.clone();
            async move {
                tx.write_all(body.as_bytes()).await.unwrap();
            }
        });
        let mut seen: Vec<String> = Vec::new();
        let mut hook: LineHook = Box::new(|l: &[u8]| {
            seen.push(String::from_utf8(l.to_vec()).unwrap());
            Ok(())
        });
        let stopped = AtomicBool::new(false);
        let out = read_stdout(rx, Some(&mut hook), &stopped).await.unwrap();
        drop(hook);
        writer.await.unwrap();
        assert_eq!(out, body.as_bytes());
        assert_eq!(seen, [long.as_str(), "short", "", "last-no-newline"]);
        assert!(!stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn path_is_binary_dir_then_fixed_dirs() {
        let env = base_env(Path::new("/opt/x/bin/claude"), Path::new("/t/r"));
        let path = env.iter().find(|(k, _)| k == "PATH").unwrap().1.clone();
        assert_eq!(path, OsString::from(format!("/opt/x/bin:{BASE_PATH}")));
        assert!(env.iter().any(|(k, v)| k == "TMPDIR" && v == "/t/r"));
    }
}
