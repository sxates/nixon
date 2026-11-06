#!/bin/sh
# Fake AI CLI for the specs/0085 subscription-CLI provider tests.
#
# Stands in for `claude` / `codex` so automated tests never run a real CLI. Driven by
# FAKE_CLI_MODE, which the runner passes through its test-only `extra_env`. Every run
# writes what it saw (argv, pwd, the cwd's entry count, the temp dir's mode, env, and a
# SHA-256 of stdin) to $FAKE_CLI_LOG, so tests check isolation from the outside instead
# of trusting the runner.
#
# Modes (more arrive with later tasks):
#   success  print a fixed reply
#   echo     stream stdin straight back to stdout (large-input round trip)
#   hang     start a `sleep 3600` grandchild, then sleep (kill tests)
#   crash    exit 2
#   flood    write 20 MiB to stdout (output cap)

log="${FAKE_CLI_LOG:-/dev/null}"
mode="${FAKE_CLI_MODE:-success}"
stdin_copy="${TMPDIR:-/tmp}/fake-cli-stdin.$$"

{
  echo "MODE:$mode"
  for a in "$@"; do printf 'ARG:%s\n' "$a"; done
  printf 'PWD:%s\n' "$(pwd)"
  printf 'PWD_ENTRIES:%s\n' "$(ls -A | wc -l | tr -d ' ')"
  printf 'TMPDIR_MODE:%s\n' "$(stat -f '%Lp' "$TMPDIR" 2>/dev/null || stat -c '%a' "$TMPDIR")"
  env | sed 's/^/ENV:/'
} >>"$log"

log_stdin_sha() {
  printf 'STDIN_SHA256:%s\n' "$(shasum -a 256 "$stdin_copy" | cut -d' ' -f1)" >>"$log"
  rm -f "$stdin_copy"
}

consume_stdin() {
  cat >"$stdin_copy"
  log_stdin_sha
}

case "$mode" in
  success)
    consume_stdin
    echo "fake reply"
    ;;
  echo)
    tee "$stdin_copy"
    log_stdin_sha
    ;;
  hang)
    consume_stdin
    sleep 3600 &
    echo "GRANDCHILD:$!" >>"$log"
    sleep 3600
    ;;
  crash)
    consume_stdin
    echo "fake crash" >&2
    exit 2
    ;;
  flood)
    consume_stdin
    head -c 20971520 /dev/zero
    ;;
  *)
    consume_stdin
    echo "fake-ai-cli: unknown FAKE_CLI_MODE '$mode'" >&2
    exit 64
    ;;
esac
