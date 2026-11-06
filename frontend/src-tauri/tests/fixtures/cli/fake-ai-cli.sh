#!/bin/sh
# Fake AI CLI for the specs/0085 subscription-CLI provider tests.
#
# Stands in for `claude` / `codex` so automated tests never run a real CLI. Driven by
# FAKE_CLI_MODE, which the runner passes through its test-only `extra_env`. Every run
# writes what it saw (argv, pwd, the cwd's entry count, the temp dir's mode, env, and a
# SHA-256 of stdin) to $FAKE_CLI_LOG, so tests check isolation from the outside instead
# of trusting the runner. The file after `--system-prompt-file` is logged as SYSTEM_SHA256.
#
# Claude stream-json modes. Like the real CLI, each reads all of stdin before printing
# `system/init`, and replays a committed W0 capture (tests/fixtures/cli/claude):
#   success        success-stream.ndjson (a plain summary)
#   json           success-json-schema.ndjson (structured_output with 4 action items)
#   json-3         the same capture, its result swapped for a synthetic one with 3 items
#                  (the specs/0054 shape a json_object reply once collapsed to one)
#   not-logged-in  not-signed-in-stream.ndjson, exit 1
#   limit          usage-limit-stream.SYNTHETIC.ndjson, exit 1
#   rate-limit     init, then a 429 rate_limit error result, exit 1
#   bad-init       init lists a tool (Bash), then sleeps before the rest of the success
#                  stream, so only an early kill keeps its answer out
#   bad-init-mcp   the same, but init lists an MCP server
#   api-retry-auth init, then an api_retry authentication_failed event, then sleeps (as
#                  Claude does between its own retries)
#   garbage        non-JSON stdout, exit 0
#
# Runner modes:
#   echo     stream stdin straight back to stdout (large-input round trip)
#   line-then-hang  print one line, then hang like `hang-ignore-term` (stdout line hook)
#   hang     start a `sleep 3600` grandchild, then sleep (kill tests)
#   hang-ignore-term  like hang, but the grandchild ignores SIGTERM (SIGKILL fallback)
#   orphan   start a grandchild that keeps stdout open, print a reply, exit 0
#   linger   start a grandchild detached from the pipes, print a reply, exit 0
#   crash    exit 2
#   exec-fail  what a CLI's `#!/usr/bin/env node` wrapper does when node is gone: an
#              `env:` error on stderr, exit 127, stdin never read
#   flood    write 20 MiB to stdout (output cap)
#
# Probe modes (discovery and auth checks, which never send a prompt). `--version` answers
# "2.1.319 (Claude Code)" in every mode but `version-old`, which answers an old version;
# `hang`, `crash` and `flood` act on any args, so a probe can be made to time out.
#   status-ready       `auth status` / `login status` signed in (exit 0)
#   status-signed-out  the same, signed out (exit 1)
#   version-old        `--version` prints 2.1.200
# The auth-status replies are the committed fixtures, found via $FAKE_CLI_FIXTURES (the
# tests/fixtures/cli/claude dir) or next to this script.

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
  prev=""
  for a in "$@"; do
    if [ "$prev" = --system-prompt-file ]; then
      printf 'SYSTEM_SHA256:%s\n' "$(shasum -a 256 "$a" | cut -d' ' -f1)"
    fi
    prev="$a"
  done
} >>"$log"

log_stdin_sha() {
  printf 'STDIN_SHA256:%s\n' "$(shasum -a 256 "$stdin_copy" | cut -d' ' -f1)" >>"$log"
  rm -f "$stdin_copy"
}

consume_stdin() {
  cat >"$stdin_copy"
  log_stdin_sha
}

fixtures="${FAKE_CLI_FIXTURES:-$(dirname "$0")/claude}"
case "$mode:$1" in
  version-old:--version)
    echo "2.1.200 (Claude Code)"
    exit 0
    ;;
  success:--version | status-*:--version)
    echo "2.1.319 (Claude Code)"
    exit 0
    ;;
  status-ready:auth)
    cat "$fixtures/auth-status-signed-in.json"
    exit 0
    ;;
  status-signed-out:auth)
    cat "$fixtures/auth-status-signed-out.json"
    exit 1
    ;;
  status-ready:login)
    echo "Logged in using ChatGPT"
    exit 0
    ;;
  status-signed-out:login)
    echo "Not logged in" >&2
    exit 1
    ;;
esac

# Line 2 of every stream capture is `system/init`.
init_line() { sed -n 2p "$fixtures/success-stream.ndjson"; }

case "$mode" in
  success)
    consume_stdin
    cat "$fixtures/success-stream.ndjson"
    ;;
  json)
    consume_stdin
    cat "$fixtures/success-json-schema.ndjson"
    ;;
  json-3)
    consume_stdin
    sed '$d' "$fixtures/success-json-schema.ndjson"
    echo '{"type":"result","subtype":"success","is_error":false,"result":"","structured_output":{"action_items":[{"description":"Send the revised launch checklist to the team","assignee":"me","due":"Friday"},{"description":"Book the demo room for the review","assignee":null,"due":null},{"description":"Draft the release notes","assignee":"me","due":"next week"}]}}'
    ;;
  not-logged-in)
    consume_stdin
    cat "$fixtures/not-signed-in-stream.ndjson"
    exit 1
    ;;
  limit)
    consume_stdin
    cat "$fixtures/usage-limit-stream.SYNTHETIC.ndjson"
    exit 1
    ;;
  rate-limit)
    consume_stdin
    sed -n 1p "$fixtures/success-stream.ndjson"
    init_line
    echo '{"type":"assistant","message":{"role":"assistant","model":"<synthetic>","content":[{"type":"text","text":"API Error: Request rejected (429)"}]},"error":"rate_limit","is_api_error_message":true}'
    echo '{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"API Error: Request rejected (429)","terminal_reason":"api_error"}'
    exit 1
    ;;
  bad-init | bad-init-mcp)
    consume_stdin
    echo "PID:$$" >>"$log"
    sed -n 1p "$fixtures/success-stream.ndjson"
    if [ "$mode" = bad-init ]; then
      init_line | sed 's/"tools": \[\]/"tools": ["Bash"]/'
    else
      init_line | sed 's/"mcp_servers": \[\]/"mcp_servers": [{"name": "canary", "status": "connected"}]/'
    fi
    sleep 30
    sed -n '3,$p' "$fixtures/success-stream.ndjson"
    ;;
  api-retry-auth)
    consume_stdin
    echo "PID:$$" >>"$log"
    sed -n 1,2p "$fixtures/not-signed-in-stream.ndjson"
    cat "$fixtures/api-retry-auth.SYNTHETIC.ndjson"
    sleep 30
    sed -n '3,$p' "$fixtures/not-signed-in-stream.ndjson"
    exit 1
    ;;
  garbage)
    consume_stdin
    echo "this is not json"
    echo "<html>Something went wrong</html>"
    ;;
  line-then-hang)
    consume_stdin
    (trap '' TERM; exec sleep 3600) &
    echo "GRANDCHILD:$!" >>"$log"
    echo "first line"
    sleep 3600
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
  hang-ignore-term)
    consume_stdin
    # An ignored signal stays ignored across exec, so the sleep itself ignores SIGTERM.
    (trap '' TERM; exec sleep 3600) &
    echo "GRANDCHILD:$!" >>"$log"
    sleep 3600
    ;;
  orphan)
    consume_stdin
    sleep 3600 &
    echo "GRANDCHILD:$!" >>"$log"
    echo "fake reply"
    exit 0
    ;;
  linger)
    consume_stdin
    sleep 3600 </dev/null >/dev/null 2>&1 &
    echo "GRANDCHILD:$!" >>"$log"
    echo "fake reply"
    exit 0
    ;;
  exec-fail)
    echo "env: node: No such file or directory" >&2
    exit 127
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
