//! Discovery and auth-probe tests for the subscription CLI providers (specs/0085), against
//! the fake CLI only. A temp HOME tree stands in for the user's machine, and the system bin
//! dirs are injected, so a real `claude` on the developer's Mac can never be found or run.
//!
//! Run with:
//!   cargo test --features metal --test cli_discovery_auth

use app_lib::cli_llm::auth::{self, AuthState, API_KEY_NOTE};
use app_lib::cli_llm::discovery::{self, Discovered, Lookup};
use app_lib::cli_llm::errors::CliError;
use app_lib::cli_llm::CliKind;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn fake_cli() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli/fake-ai-cli.sh")
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli/claude")
}

/// Puts a copy of the fake CLI at `path` (creating parent dirs), executable.
fn plant(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::copy(fake_cli(), path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn home(&self) -> PathBuf {
        let h = self.dir.path().join("home");
        std::fs::create_dir_all(&h).unwrap();
        h
    }
    fn log(&self) -> PathBuf {
        self.dir.path().join("fake-cli.log")
    }
    fn lookup(&self, kind: CliKind, mode: &str) -> Lookup {
        let mut l = Lookup::new(kind, self.home());
        l.system_bin_dirs = Vec::new();
        l.env_override = None;
        l.shell = None;
        l.test_env = vec![
            ("FAKE_CLI_MODE".into(), mode.into()),
            ("FAKE_CLI_LOG".into(), self.log().display().to_string()),
            (
                "FAKE_CLI_FIXTURES".into(),
                fixtures_dir().display().to_string(),
            ),
        ];
        l
    }
}

async fn found(l: &Lookup) -> Discovered {
    discovery::discover_with(l)
        .await
        .unwrap_or_else(|e| panic!("discovery failed: {e:?}"))
}

#[tokio::test]
async fn first_candidate_wins_in_spec_order() {
    let sb = Sandbox::new();
    let home = sb.home();
    plant(&home.join(".volta/bin/claude"));
    plant(&home.join(".npm-global/bin/claude"));
    let l = sb.lookup(CliKind::ClaudeCode, "success");
    assert_eq!(found(&l).await.path, home.join(".npm-global/bin/claude"));

    plant(&home.join(".local/bin/claude"));
    assert_eq!(found(&l).await.path, home.join(".local/bin/claude"));
}

#[tokio::test]
async fn system_dirs_come_after_local_bin_and_before_npm_global() {
    let sb = Sandbox::new();
    let home = sb.home();
    let sys = sb.dir.path().join("sys-bin");
    plant(&sys.join("claude"));
    plant(&home.join(".npm-global/bin/claude"));
    let mut l = sb.lookup(CliKind::ClaudeCode, "success");
    l.system_bin_dirs = vec![sys.clone()];
    assert_eq!(found(&l).await.path, sys.join("claude"));
    plant(&home.join(".local/bin/claude"));
    assert_eq!(found(&l).await.path, home.join(".local/bin/claude"));
}

#[tokio::test]
async fn old_claude_local_install_is_a_claude_only_candidate() {
    let sb = Sandbox::new();
    let home = sb.home();
    plant(&home.join(".claude/local/claude"));
    let l = sb.lookup(CliKind::ClaudeCode, "success");
    assert_eq!(found(&l).await.path, home.join(".claude/local/claude"));
    // Codex looks for `codex`, not under ~/.claude.
    let l = sb.lookup(CliKind::CodexCli, "success");
    assert_eq!(
        discovery::discover_with(&l).await.unwrap_err(),
        CliError::NotFound
    );
}

#[tokio::test]
async fn nvm_picks_the_newest_version_numerically() {
    let sb = Sandbox::new();
    let home = sb.home();
    for v in ["v18.20.0", "v9.0.0", "v20.11.0", "v20.9.1"] {
        plant(&home.join(format!(".nvm/versions/node/{v}/bin/claude")));
    }
    let l = sb.lookup(CliKind::ClaudeCode, "success");
    assert_eq!(
        found(&l).await.path,
        home.join(".nvm/versions/node/v20.11.0/bin/claude")
    );
}

#[tokio::test]
async fn npmrc_prefix_is_a_candidate_and_tilde_expands() {
    let sb = Sandbox::new();
    let home = sb.home();
    std::fs::write(
        home.join(".npmrc"),
        "; comment\nregistry=https://example.invalid\nprefix=~/custom-npm\n",
    )
    .unwrap();
    plant(&home.join("custom-npm/bin/claude"));
    let l = sb.lookup(CliKind::ClaudeCode, "success");
    assert_eq!(found(&l).await.path, home.join("custom-npm/bin/claude"));
}

#[tokio::test]
async fn symlink_path_is_kept_unresolved() {
    let sb = Sandbox::new();
    let home = sb.home();
    let real = sb.dir.path().join("versions/2.1.319/claude");
    plant(&real);
    let link = home.join(".local/bin/claude");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    symlink(&real, &link).unwrap();
    let l = sb.lookup(CliKind::ClaudeCode, "success");
    let d = found(&l).await;
    assert_eq!(d.path, link);
    assert_ne!(d.path, real.canonicalize().unwrap());
}

#[tokio::test]
async fn env_override_beats_user_override_and_candidates() {
    let sb = Sandbox::new();
    let home = sb.home();
    plant(&home.join(".local/bin/claude"));
    let user = sb.dir.path().join("user/claude");
    plant(&user);
    let env_one = sb.dir.path().join("env/claude");
    plant(&env_one);
    let mut l = sb.lookup(CliKind::ClaudeCode, "success");
    l.user_override = Some(user.clone());
    assert_eq!(found(&l).await.path, user, "user override beats candidates");
    l.env_override = Some(env_one.clone());
    assert_eq!(found(&l).await.path, env_one);
}

#[tokio::test]
async fn a_missing_override_falls_through_to_the_next_source() {
    let sb = Sandbox::new();
    let home = sb.home();
    plant(&home.join(".local/bin/claude"));
    let mut l = sb.lookup(CliKind::ClaudeCode, "success");
    l.env_override = Some(sb.dir.path().join("nope/claude"));
    l.user_override = Some(sb.dir.path().join("also-nope/claude"));
    assert_eq!(found(&l).await.path, home.join(".local/bin/claude"));
}

#[tokio::test]
async fn nothing_found_is_not_found() {
    let sb = Sandbox::new();
    let l = sb.lookup(CliKind::ClaudeCode, "success");
    assert_eq!(
        discovery::discover_with(&l).await.unwrap_err(),
        CliError::NotFound
    );
}

#[tokio::test]
async fn a_non_executable_file_is_not_a_hit() {
    let sb = Sandbox::new();
    let home = sb.home();
    let p = home.join(".local/bin/claude");
    plant(&p);
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    let l = sb.lookup(CliKind::ClaudeCode, "success");
    assert_eq!(
        discovery::discover_with(&l).await.unwrap_err(),
        CliError::NotFound
    );
}

#[tokio::test]
async fn version_is_parsed_from_the_cli_output() {
    let sb = Sandbox::new();
    plant(&sb.home().join(".local/bin/claude"));
    let d = found(&sb.lookup(CliKind::ClaudeCode, "success")).await;
    assert_eq!(d.version, "2.1.319");
}

#[tokio::test]
async fn too_old_is_reported_with_both_versions() {
    let sb = Sandbox::new();
    plant(&sb.home().join(".local/bin/claude"));
    let err = discovery::discover_with(&sb.lookup(CliKind::ClaudeCode, "version-old"))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        CliError::TooOld {
            found: "2.1.200".into(),
            min: "2.1.290".into()
        }
    );
}

#[tokio::test]
async fn a_version_probe_that_hangs_is_a_probe_timeout_not_the_five_minute_copy() {
    let sb = Sandbox::new();
    plant(&sb.home().join(".local/bin/claude"));
    let mut l = sb.lookup(CliKind::ClaudeCode, "hang");
    l.probe_timeout = Duration::from_millis(400);
    let err = discovery::discover_with(&l).await.unwrap_err();
    assert_eq!(err, CliError::ProbeTimeout);
    assert_eq!(
        err.message(CliKind::ClaudeCode),
        "Claude Code didn't respond. Check again in a moment."
    );
}

fn fake_shell(sb: &Sandbox, prints: &Path) -> PathBuf {
    let sh = sb.dir.path().join("fake-shell.sh");
    std::fs::write(
        &sh,
        format!(
            "#!/bin/sh\n# expects: -l -c 'command -v <name>'\n[ \"$1\" = -l ] && [ \"$2\" = -c ] || exit 9\n\
             echo 'noisy dotfile output'\necho '{}'\n",
            prints.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
    sh
}

#[tokio::test]
async fn login_shell_probe_runs_only_when_allowed() {
    let sb = Sandbox::new();
    let hidden = sb.dir.path().join("somewhere/else/claude");
    plant(&hidden);
    let mut l = sb.lookup(CliKind::ClaudeCode, "success");
    l.shell = Some(fake_shell(&sb, &hidden));

    l.allow_shell_probe = false;
    assert_eq!(
        discovery::discover_with(&l).await.unwrap_err(),
        CliError::NotFound
    );

    l.allow_shell_probe = true;
    assert_eq!(found(&l).await.path, hidden);
}

#[tokio::test]
async fn login_shell_probe_is_last_not_first() {
    let sb = Sandbox::new();
    let home = sb.home();
    plant(&home.join(".local/bin/claude"));
    let hidden = sb.dir.path().join("somewhere/else/claude");
    plant(&hidden);
    let mut l = sb.lookup(CliKind::ClaudeCode, "success");
    l.shell = Some(fake_shell(&sb, &hidden));
    l.allow_shell_probe = true;
    assert_eq!(found(&l).await.path, home.join(".local/bin/claude"));
}

#[tokio::test]
async fn auth_signed_in_is_ready() {
    let sb = Sandbox::new();
    let bin = sb.home().join(".local/bin/claude");
    plant(&bin);
    let l = sb.lookup(CliKind::ClaudeCode, "status-ready");
    let state = auth::check_with(CliKind::ClaudeCode, &bin, l.probe_timeout, &l.test_env)
        .await
        .unwrap();
    assert_eq!(state, AuthState::Ready { note: None });
}

#[tokio::test]
async fn auth_signed_out_needs_sign_in() {
    let sb = Sandbox::new();
    let bin = sb.home().join(".local/bin/claude");
    plant(&bin);
    let l = sb.lookup(CliKind::ClaudeCode, "status-signed-out");
    let state = auth::check_with(CliKind::ClaudeCode, &bin, l.probe_timeout, &l.test_env)
        .await
        .unwrap();
    assert_eq!(state, AuthState::NeedsSignIn);
}

#[tokio::test]
async fn codex_login_status_maps_exit_codes() {
    let sb = Sandbox::new();
    let bin = sb.home().join(".local/bin/codex");
    plant(&bin);
    for (mode, want) in [
        ("status-ready", AuthState::Ready { note: None }),
        ("status-signed-out", AuthState::NeedsSignIn),
    ] {
        let l = sb.lookup(CliKind::CodexCli, mode);
        let got = auth::check_with(CliKind::CodexCli, &bin, l.probe_timeout, &l.test_env)
            .await
            .unwrap();
        assert_eq!(got, want, "{mode}");
    }
    assert!(API_KEY_NOTE.contains("billed to that account"));
}

#[tokio::test]
async fn a_status_check_runs_only_version_and_auth_status_never_a_prompt() {
    let sb = Sandbox::new();
    let bin = sb.home().join(".local/bin/claude");
    plant(&bin);
    let mut l = sb.lookup(CliKind::ClaudeCode, "status-ready");
    l.user_override = Some(bin.clone());
    let d = found(&l).await;
    let state = auth::check_with(CliKind::ClaudeCode, &d.path, l.probe_timeout, &l.test_env)
        .await
        .unwrap();
    assert_eq!(state, AuthState::Ready { note: None });

    let log = format!("\n{}", std::fs::read_to_string(sb.log()).unwrap());
    let runs: Vec<&str> = log.split("\nMODE:").skip(1).collect();
    assert_eq!(runs.len(), 2, "exactly two CLI runs: {log}");
    let argv: Vec<Vec<&str>> = runs
        .iter()
        .map(|r| r.lines().filter_map(|l| l.strip_prefix("ARG:")).collect())
        .collect();
    assert_eq!(argv[0], vec!["--version"]);
    assert_eq!(argv[1], vec!["auth", "status"]);
    assert!(
        !log.contains("ARG:-p"),
        "a status check must not run a prompt"
    );
}

#[tokio::test]
async fn auth_probe_that_hangs_is_a_probe_timeout() {
    let sb = Sandbox::new();
    let bin = sb.home().join(".local/bin/claude");
    plant(&bin);
    let l = sb.lookup(CliKind::ClaudeCode, "hang");
    let err = auth::check_with(
        CliKind::ClaudeCode,
        &bin,
        Duration::from_millis(400),
        &l.test_env,
    )
    .await
    .unwrap_err();
    assert_eq!(err, CliError::ProbeTimeout);
}

#[tokio::test]
async fn cache_serves_a_hit_and_invalidate_forgets_it() {
    let sb = Sandbox::new();
    let bin = sb.home().join(".local/bin/claude");
    plant(&bin);
    let l = sb.lookup(CliKind::CodexCli, "success");
    // Codex has its own cache slot, so this can't race the Claude tests.
    let codex = sb.home().join(".local/bin/codex");
    plant(&codex);
    let first = discovery::discover_cached(&l).await.unwrap();
    std::fs::remove_file(&codex).unwrap();
    // The cached path is re-checked, so a vanished binary is a miss, not a stale hit.
    assert_eq!(
        discovery::discover_cached(&l).await.unwrap_err(),
        CliError::NotFound
    );
    plant(&codex);
    let again = discovery::discover_cached(&l).await.unwrap();
    assert_eq!(first, again);
    discovery::invalidate(CliKind::CodexCli);
    assert!(discovery::cached(CliKind::CodexCli).is_none());
}
