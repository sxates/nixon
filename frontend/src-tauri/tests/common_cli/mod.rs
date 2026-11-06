//! Drives production LLM entry points against the fake CLI (specs/0085).
//!
//! Production calls pass no test env to the runner, so the fake's mode can't travel in
//! `FAKE_CLI_MODE`. A wrapper script stands in for `claude` instead: it answers
//! `--version` (discovery), logs every other run's argv, and execs the fake in the mode
//! named in a file the test rewrites between cases. Discovery finds the wrapper through
//! `NIXON_CLI_CLAUDE_CODE`, and HOME points at an empty temp dir so no real install can be
//! picked up. That state is process-wide, so [`FakeClaude::install`] takes a binary-wide
//! lock that the returned value holds: tests in one binary run one at a time.

// Each test binary that includes this uses only part of it.
#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct FakeClaude {
    dir: tempfile::TempDir,
    _home: tempfile::TempDir,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl FakeClaude {
    /// Writes the wrapper and points discovery (and HOME) at it for this process, holding
    /// the binary-wide lock until dropped.
    pub async fn install() -> Self {
        let lock = LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fake = manifest.join("tests/fixtures/cli/fake-ai-cli.sh");
        let fixtures = manifest.join("tests/fixtures/cli/claude");
        let wrapper = dir.path().join("claude");
        let script = format!(
            r#"#!/bin/sh
echo "$*" >>'{dir}/invocations'
if [ "$1" = --version ]; then
  m="$(cat '{dir}/mode' 2>/dev/null)"; [ "$m" = version-old ] || m=success
  FAKE_CLI_MODE="$m" exec '{fake}' --version
fi
echo run >>'{dir}/calls'
for a in "$@"; do printf '%s\n' "$a"; done >'{dir}/argv'
FAKE_CLI_MODE="$(cat '{dir}/mode')" FAKE_CLI_FIXTURES='{fixtures}' exec '{fake}' "$@"
"#,
            fake = fake.display(),
            dir = dir.path().display(),
            fixtures = fixtures.display(),
        );
        std::fs::write(&wrapper, script).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Edition 2021: set_var is safe. Only this test binary's process sees them.
        std::env::set_var("NIXON_CLI_CLAUDE_CODE", &wrapper);
        std::env::set_var("HOME", home.path());
        Self {
            dir,
            _home: home,
            _lock: lock,
        }
    }

    /// Selects the fake's mode for the next runs and resets the run count.
    pub fn set_mode(&self, mode: &str) {
        std::fs::write(self.path("mode"), mode).unwrap();
        let _ = std::fs::remove_file(self.path("calls"));
        let _ = std::fs::remove_file(self.path("invocations"));
    }

    /// Prompt runs since the last [`set_mode`] (`--version` probes are not counted).
    pub fn calls(&self) -> usize {
        std::fs::read_to_string(self.path("calls"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    /// The last prompt run's argv, one arg per line as written.
    pub fn last_argv(&self) -> Vec<String> {
        std::fs::read_to_string(self.path("argv"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Every argv the wrapper saw since install, one line per invocation, `--version`
    /// probes and auth probes included.
    pub fn invocations(&self) -> Vec<String> {
        std::fs::read_to_string(self.path("invocations"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The wrapper's path (what `NIXON_CLI_CLAUDE_CODE` points at).
    pub fn wrapper(&self) -> PathBuf {
        self.path("claude")
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}
