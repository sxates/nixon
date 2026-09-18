#!/bin/bash
# Cargo wrapper used as Tauri's `build.runner` in DEV builds only (set in
# tauri.dev.conf.json, which scripts/tauri-auto.js merges for `tauri dev`).
#
# Why: the bare dev binary is ad-hoc signed by the linker, and that signature
# changes on every rebuild. macOS keys Keychain item ACLs and TCC grants
# (Audio Capture) to the code signature, so every rebuild used to orphan
# them — a login-keychain password prompt on each Keychain read (ADR-0009
# API keys) and silently-dead system-audio capture (ADR-0004). Re-signing each
# debug build with a *stable* Apple Development identity gives every rebuild
# the same designated requirement, so one "Always Allow" / one TCC grant
# sticks forever.
#
# Tauri 2 `dev` invokes the runner as `<runner> run [cargo args] [-- app args]`
# — cargo run launches the app straight out of the link step, leaving no gap to
# sign in. So `run` is intercepted: `cargo build` with the same args, sign,
# then exec the binary ourselves (tauri's child PID stays the app).
#
# Identity resolution: $NIXON_DEV_SIGNING_IDENTITY if set, else the first
# "Apple Development" identity in the keychain, else no-op (ad-hoc signature
# kept — previous behavior, e.g. on a machine with no Apple cert).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Workspace target dir lives at the repo root (src-tauri is two levels down).
TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/../../../target}"
BIN="$TARGET_DIR/debug/nixon"

sign_debug_binary() {
  [[ "$(uname)" == "Darwin" && -f "$BIN" ]] || return 0
  local identity="${NIXON_DEV_SIGNING_IDENTITY:-}"
  if [[ -z "$identity" ]]; then
    identity=$(security find-identity -v -p codesigning 2>/dev/null \
      | awk -F'"' '/Apple Development/ {print $2; exit}') || true
  fi
  [[ -n "$identity" ]] || return 0
  codesign --force --sign "$identity" \
    --entitlements "$SCRIPT_DIR/dev-entitlements.plist" \
    "$BIN"
  echo "[nixon] dev binary re-signed with stable identity: $identity"
}

if [[ "${1:-}" == "run" ]]; then
  shift
  build_args=()
  app_args=()
  in_app_args=0
  passthrough=0
  for arg in "$@"; do
    if ((in_app_args)); then
      app_args+=("$arg")
    elif [[ "$arg" == "--" ]]; then
      in_app_args=1
    else
      # Non-debug profiles: unknown output dir — fall back to plain cargo run
      # (pre-wrapper behavior, unsigned). `tauri dev` never passes these.
      case "$arg" in
        --release|--profile*) passthrough=1 ;;
      esac
      build_args+=("$arg")
    fi
  done
  if ((passthrough)); then
    exec cargo run "$@"
  fi
  cargo build ${build_args[@]+"${build_args[@]}"}
  sign_debug_binary
  exec "$BIN" ${app_args[@]+"${app_args[@]}"}
fi

cargo "$@"

# Some flows build without running (e.g. a future tauri that spawns directly);
# keep those signed too. Release builds are signed by the Tauri bundler with
# the production identity instead.
if [[ "${1:-}" == "build" ]]; then
  skip=0
  for arg in "$@"; do
    case "$arg" in
      --release|--profile*) skip=1 ;;
    esac
  done
  ((skip)) || sign_debug_binary
fi
