#!/bin/bash
# Nixon dev launcher.
#
# Ensures the Rust toolchain is on PATH, then delegates to upstream's GPU dev
# script (dev-gpu.sh), which builds the llama-helper sidecar and runs `tauri dev`.
#
# Why this wrapper exists: rustup installs cargo to ~/.cargo/bin but only wires it
# into login / interactive zsh shells. dev-gpu.sh runs under `#!/bin/bash`
# non-interactively, so it never sources that env and fails with
# `cargo: command not found`. Sourcing it here makes launches work from any shell
# (including Claude Code's `!` prefix). We keep dev-gpu.sh unmodified to stay
# mergeable with upstream meetily.
set -e

if ! command -v cargo >/dev/null 2>&1; then
  [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "❌ cargo not found. Install Rust: https://rustup.rs  (then re-run)" >&2
  exit 1
fi

cd "$(dirname "$0")"

# Google Calendar OAuth client (specs/0032). Optional: without it the feature is
# inert and Settings shows "not configured". Values bake in via option_env! at
# compile time, so they must be in the environment before cargo builds.
if [ -f "src-tauri/.env.google" ]; then
  set -a
  . "src-tauri/.env.google"
  set +a
fi

# specs/0057 rename guard: pre-0057 .env.google files export VINYL_GOOGLE_CLIENT_ID/_SECRET,
# which the build no longer reads — Calendar would go silently inert. Warn, don't abort.
if [ -f "src-tauri/.env.google" ] && { [ -z "${NIXON_GOOGLE_CLIENT_ID:-}" ] || [ -z "${NIXON_GOOGLE_CLIENT_SECRET:-}" ]; }; then
  {
    echo ""
    echo "⚠️  Google Calendar will be INERT in this build."
    echo "   src-tauri/.env.google exists but NIXON_GOOGLE_CLIENT_ID/NIXON_GOOGLE_CLIENT_SECRET are not both set."
    echo "   A pre-0057 .env.google still uses the old VINYL_ prefix; rename both keys:"
    echo "     VINYL_GOOGLE_CLIENT_ID     -> NIXON_GOOGLE_CLIENT_ID"
    echo "     VINYL_GOOGLE_CLIENT_SECRET -> NIXON_GOOGLE_CLIENT_SECRET"
    echo "   See SETUP.md \"Google Calendar OAuth client\"."
    echo ""
  } >&2
fi

exec ./dev-gpu.sh "$@"
