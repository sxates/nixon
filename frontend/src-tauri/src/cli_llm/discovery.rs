//! Finds the user's installed AI CLI and checks it is new enough (specs/0085).
//!
//! Resolution order, first hit wins:
//! 1. the env override (`NIXON_CLI_CLAUDE_CODE` / `NIXON_CLI_CODEX`, dev and tests);
//! 2. the path the user chose with Locate… in Settings;
//! 3. a fixed list of install locations ([`candidates`]);
//! 4. one login-shell `command -v <name>`, only when the caller allows it. That runs the
//!    user's dotfiles, so the caller passes `allow_shell_probe` only after the user selected
//!    the provider in Settings, never at launch and never per call.
//!
//! The hit's path is kept exactly as found: `~/.local/bin/claude` is a symlink the CLI's
//! auto-updater re-points, so resolving it would pin an old version. The binary is then
//! validated with `<path> --version` (10 s) against [`min_version`].

use super::errors::CliError;
use super::runner::{self, Invocation, RunOutput};
use super::CliKind;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// How long `--version` and the auth probes get before they count as not responding.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// The login-shell `command -v` gets this long.
const SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// A CLI found on disk and validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Unresolved: a symlink stays a symlink.
    pub path: PathBuf,
    /// The semver the CLI reported ("2.1.319").
    pub version: String,
}

/// The oldest CLI version Nixon supports.
pub fn min_version(kind: CliKind) -> &'static str {
    match kind {
        // Newest flag we pass is 2.1.259; 2.1.290 fixed two `--restricted` holes.
        CliKind::ClaudeCode => "2.1.290",
        // No floor until the Codex adapter work sets one (W2).
        CliKind::CodexCli => "0.0.0",
    }
}

/// Everything a lookup depends on, so tests can fake the machine.
#[derive(Debug, Clone)]
pub struct Lookup {
    pub kind: CliKind,
    pub home: PathBuf,
    /// Source 1. [`Lookup::new`] reads it from the environment.
    pub env_override: Option<PathBuf>,
    /// Source 2, from `settings.cliProviderConfig`.
    pub user_override: Option<PathBuf>,
    /// Source 4 runs only when this is true.
    pub allow_shell_probe: bool,
    /// The user's login shell for source 4 (`$SHELL`).
    pub shell: Option<PathBuf>,
    /// Directories outside HOME searched after `~/.local/bin` (Homebrew, /usr/local).
    pub system_bin_dirs: Vec<PathBuf>,
    pub probe_timeout: Duration,
    /// Forwarded to the runner's test-only `extra_env` (the fake CLI's mode and log).
    /// Ignored in release builds.
    pub test_env: Vec<(String, String)>,
}

impl Lookup {
    pub fn new(kind: CliKind, home: PathBuf) -> Self {
        Self {
            kind,
            home,
            env_override: env_override(kind),
            user_override: None,
            allow_shell_probe: false,
            shell: std::env::var_os("SHELL")
                .map(PathBuf::from)
                .filter(|s| s.is_absolute()),
            system_bin_dirs: vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()],
            probe_timeout: PROBE_TIMEOUT,
            test_env: Vec::new(),
        }
    }
}

fn env_override(kind: CliKind) -> Option<PathBuf> {
    let var = match kind {
        CliKind::ClaudeCode => "NIXON_CLI_CLAUDE_CODE",
        CliKind::CodexCli => "NIXON_CLI_CODEX",
    };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_default()
}

// ---- process-wide cache ----

static CACHE: Lazy<Mutex<HashMap<CliKind, Discovered>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// The cached discovery for `kind`, if any.
pub fn cached(kind: CliKind) -> Option<Discovered> {
    CACHE.lock().ok()?.get(&kind).cloned()
}

/// Forgets `kind`'s cached discovery. Call on Locate…, on "Check again", and when a call
/// fails with `NotFound`.
pub fn invalidate(kind: CliKind) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.remove(&kind);
    }
}

/// Production entry: finds `kind` for this user, cached for the life of the process.
/// Pass `allow_shell_probe` only after the user selected this provider in Settings.
pub async fn discover(
    kind: CliKind,
    user_override: Option<PathBuf>,
    allow_shell_probe: bool,
) -> Result<Discovered, CliError> {
    let mut lookup = Lookup::new(kind, home_dir());
    lookup.user_override = user_override;
    lookup.allow_shell_probe = allow_shell_probe;
    discover_cached(&lookup).await
}

/// [`discover_with`] behind the cache. A cached path whose file has vanished or lost its
/// execute bit is dropped rather than served, and a `NotFound` clears the entry.
pub async fn discover_cached(lookup: &Lookup) -> Result<Discovered, CliError> {
    if let Some(hit) = cached(lookup.kind) {
        if is_executable(&hit.path) {
            return Ok(hit);
        }
        invalidate(lookup.kind);
    }
    match discover_with(lookup).await {
        Ok(found) => {
            if let Ok(mut cache) = CACHE.lock() {
                cache.insert(lookup.kind, found.clone());
            }
            Ok(found)
        }
        Err(e) => {
            if e == CliError::NotFound {
                invalidate(lookup.kind);
            }
            Err(e)
        }
    }
}

/// Resolves and validates the binary with no caching.
pub async fn discover_with(lookup: &Lookup) -> Result<Discovered, CliError> {
    let path = resolve(lookup).await.ok_or(CliError::NotFound)?;
    let version = validate(lookup, &path).await?;
    Ok(Discovered { path, version })
}

async fn resolve(lookup: &Lookup) -> Option<PathBuf> {
    let overrides = [&lookup.env_override, &lookup.user_override];
    if let Some(p) = overrides.into_iter().flatten().find(|p| is_executable(p)) {
        return Some(p.clone());
    }
    if let Some(p) = candidates(lookup).into_iter().find(|p| is_executable(p)) {
        return Some(p);
    }
    if lookup.allow_shell_probe {
        return shell_probe(lookup).await;
    }
    None
}

/// The fixed install locations, in the order they are tried. With no usable (absolute)
/// home, only the system dirs: a home-relative path would resolve against the cwd.
pub fn candidates(lookup: &Lookup) -> Vec<PathBuf> {
    let name = lookup.kind.command_name();
    let home = &lookup.home;
    let system = lookup.system_bin_dirs.iter().map(|d| d.join(name));
    if !home.is_absolute() {
        return system.collect();
    }
    let mut list = vec![home.join(".local/bin").join(name)];
    if lookup.kind == CliKind::ClaudeCode {
        list.push(home.join(".claude/local/claude"));
    }
    list.extend(system);
    list.push(home.join(".npm-global/bin").join(name));
    if let Some(prefix) = npmrc_prefix(home) {
        list.push(prefix.join("bin").join(name));
    }
    list.extend(
        nvm_node_versions(home)
            .into_iter()
            .map(|v| v.join("bin").join(name)),
    );
    for rel in [
        ".volta/bin",
        ".bun/bin",
        "Library/pnpm",
        ".local/share/pnpm",
        ".asdf/shims",
        ".local/share/mise/shims",
    ] {
        list.push(home.join(rel).join(name));
    }
    list
}

/// `prefix=` from `~/.npmrc`, with a leading `~` expanded. Relative prefixes are ignored.
fn npmrc_prefix(home: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(home.join(".npmrc")).ok()?;
    parse_npmrc_prefix(&text, home)
}

fn parse_npmrc_prefix(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().find_map(|line| {
        let (key, value) = line.trim().split_once('=')?;
        (key.trim() == "prefix").then(|| value.trim().trim_matches(|c| c == '"' || c == '\''))
    })?;
    let path = if value == "~" {
        home.to_path_buf()
    } else if let Some(rest) = value.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(value)
    };
    path.is_absolute().then_some(path)
}

/// `~/.nvm/versions/node/<version>` dirs, newest version first (numeric, so v20 beats v9).
fn nvm_node_versions(home: &Path) -> Vec<PathBuf> {
    let root = home.join(".nvm/versions/node");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut versions: Vec<(Option<semver::Version>, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let parsed = semver::Version::parse(name.trim_start_matches('v')).ok();
            (parsed, e.path())
        })
        .collect();
    // `Option` orders None below Some, so reversing puts unparsable names last.
    versions.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    versions.into_iter().map(|(_, p)| p).collect()
}

/// `$SHELL -l -c 'command -v <name>'` under the runner's clean env, 3 s. Dotfiles may print
/// noise, so the answer is the last line that is an absolute path to an executable.
async fn shell_probe(lookup: &Lookup) -> Option<PathBuf> {
    let shell = lookup.shell.as_deref()?;
    let mut inv = Invocation::new(lookup.kind, shell, SHELL_PROBE_TIMEOUT);
    inv.args = vec![
        OsString::from("-l"),
        OsString::from("-c"),
        OsString::from(format!("command -v {}", lookup.kind.command_name())),
    ];
    let out = runner::run(inv).await.ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('/'))
        .map(PathBuf::from)
        .filter(|p| is_executable(p))
}

/// Runs a quick probe (`--version`, `auth status`). A timeout is [`CliError::ProbeTimeout`],
/// never the five-minute copy. Claude probes get the adapter's env switches too (no
/// self-update, no non-essential traffic, no prompt history), like its model calls.
pub(super) async fn probe(
    kind: CliKind,
    binary: &Path,
    args: &[&str],
    timeout: Duration,
    test_env: &[(String, String)],
) -> Result<RunOutput, CliError> {
    let mut inv = Invocation::new(kind, binary, timeout);
    inv.args = args.iter().map(OsString::from).collect();
    if kind == CliKind::ClaudeCode {
        inv.env = super::claude::env();
    }
    #[cfg(any(test, debug_assertions))]
    {
        inv.extra_env = test_env.to_vec();
    }
    #[cfg(not(any(test, debug_assertions)))]
    let _ = test_env;
    match runner::run(inv).await {
        Err(CliError::Timeout) => Err(CliError::ProbeTimeout),
        other => other,
    }
}

/// Checks a user-chosen binary on its own (no search, no cache): it must be an executable
/// file that answers `--version` with a supported version. Returns the version.
pub async fn validate_path(kind: CliKind, path: &Path) -> Result<String, CliError> {
    if !path.is_absolute() || !is_executable(path) {
        return Err(CliError::NotFound);
    }
    validate(&Lookup::new(kind, PathBuf::new()), path).await
}

/// `<path> --version`, parsed and compared with [`min_version`]. Returns the version.
async fn validate(lookup: &Lookup, path: &Path) -> Result<String, CliError> {
    let out = probe(
        lookup.kind,
        path,
        &["--version"],
        lookup.probe_timeout,
        &lookup.test_env,
    )
    .await?;
    if let Some(crash) = out.crash_error() {
        return Err(crash);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let version = parse_version(&text).ok_or(CliError::BadOutput)?;
    let min = min_version(lookup.kind);
    if version_lt(&version, min) {
        return Err(CliError::TooOld {
            found: version,
            min: min.to_string(),
        });
    }
    Ok(version)
}

/// The first semver in `--version` output: "2.1.319 (Claude Code)", "codex-cli 0.46.0".
fn parse_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|t| t.trim_start_matches('v'))
        .find(|t| semver::Version::parse(t).is_ok())
        .map(str::to_string)
}

fn version_lt(found: &str, min: &str) -> bool {
    match (semver::Version::parse(found), semver::Version::parse(min)) {
        (Ok(f), Ok(m)) => f < m,
        _ => false,
    }
}

/// A regular file (symlinks followed) with an execute bit set.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_version_out_of_each_clis_banner() {
        assert_eq!(
            parse_version("2.1.319 (Claude Code)\n").as_deref(),
            Some("2.1.319")
        );
        assert_eq!(parse_version("codex-cli 0.46.0").as_deref(), Some("0.46.0"));
        assert_eq!(parse_version("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn version_comparison_is_numeric_not_lexical() {
        assert!(version_lt("2.1.99", "2.1.290"));
        assert!(!version_lt("2.1.290", "2.1.290"));
        assert!(!version_lt("2.10.0", "2.1.290"));
        assert!(!version_lt("0.0.1", "0.0.0"));
    }

    #[test]
    fn npmrc_prefix_handles_comments_quotes_and_tilde() {
        let home = Path::new("/h");
        assert_eq!(
            parse_npmrc_prefix("prefix=/opt/npm", home),
            Some("/opt/npm".into())
        );
        assert_eq!(
            parse_npmrc_prefix("; x\n prefix = \"~/n\" ", home),
            Some("/h/n".into())
        );
        assert_eq!(parse_npmrc_prefix("prefix=relative", home), None);
        assert_eq!(parse_npmrc_prefix("registry=x", home), None);
    }

    #[test]
    fn an_empty_or_relative_home_searches_no_home_paths() {
        // An unset-but-empty HOME must never turn `.local/bin/claude` into a cwd lookup.
        for home in ["", "relative/home"] {
            let mut l = Lookup::new(CliKind::ClaudeCode, home.into());
            l.system_bin_dirs = vec!["/opt/homebrew/bin".into()];
            let got: Vec<String> = candidates(&l)
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            assert_eq!(got, ["/opt/homebrew/bin/claude"], "home {home:?}");
        }
    }

    #[test]
    fn candidate_list_follows_the_spec_order() {
        let mut l = Lookup::new(CliKind::ClaudeCode, "/h".into());
        l.system_bin_dirs = vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()];
        let got: Vec<String> = candidates(&l)
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        assert_eq!(
            got,
            [
                "/h/.local/bin/claude",
                "/h/.claude/local/claude",
                "/opt/homebrew/bin/claude",
                "/usr/local/bin/claude",
                "/h/.npm-global/bin/claude",
                "/h/.volta/bin/claude",
                "/h/.bun/bin/claude",
                "/h/Library/pnpm/claude",
                "/h/.local/share/pnpm/claude",
                "/h/.asdf/shims/claude",
                "/h/.local/share/mise/shims/claude",
            ]
        );
    }
}
