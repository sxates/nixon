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

# specs/0059 dev flags. dev-gpu.sh drops positional args, so flags become env vars here.
#   --demo            seed the fictional dataset into the .debug profile at startup
#                     (also enables --control, for the specs/0060 screenshot driver)
#   --no-audio        with --demo: skip say/ffmpeg audio synthesis
#   --onboarding      clear onboarding status and simulate model downloads
#   --real-downloads  with --onboarding: keep the real downloads
#   --control         (specs/0060) start the debug-only loopback control listener,
#                     used by the screenshot driver to steer the running window
#   (end of dev flags)
REAL_DOWNLOADS=0; WANT_ONBOARDING=0
for arg in "$@"; do
  case "$arg" in
    --demo)           export NIXON_FIXTURES=demo; export NIXON_DEV_CONTROL=1 ;;
    --no-audio)       export NIXON_FIXTURES_NO_AUDIO=1 ;;
    --onboarding)     WANT_ONBOARDING=1 ;;
    --real-downloads) REAL_DOWNLOADS=1 ;;
    --control)        export NIXON_DEV_CONTROL=1 ;;
    -h|--help)        sed -n '/^# specs\/0059 dev flags/,/^#   (end of dev flags)/p' "$0" | sed 's/^#\{0,1\} \{0,1\}//' ; exit 0 ;;
    *) echo "dev-nixon.sh: unknown flag $arg" >&2; exit 2 ;;
  esac
done
if [ "$WANT_ONBOARDING" = 1 ]; then
  export NIXON_RESET_ONBOARDING=1
  [ "$REAL_DOWNLOADS" = 1 ] || export NIXON_FAKE_DOWNLOADS=1
fi
[ -n "${NIXON_FIXTURES:-}${NIXON_RESET_ONBOARDING:-}${NIXON_DEV_CONTROL:-}" ] && echo "[nixon] dev flags: ${NIXON_FIXTURES:+fixtures=demo }${NIXON_FIXTURES_NO_AUDIO:+no-audio }${NIXON_RESET_ONBOARDING:+reset-onboarding }${NIXON_FAKE_DOWNLOADS:+fake-downloads }${NIXON_DEV_CONTROL:+control}" || true

# specs/0058: dev builds never check for or install updates.
export NIXON_DISABLE_UPDATER=1

exec ./dev-gpu.sh
