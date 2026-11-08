# Claude Code CLI fixtures (spec 0085 W0)

Captured from `claude` 2.1.319 on macOS, signed in with a claude.ai subscription, using the
fictional `--demo` meeting 1 transcript. Every file is sanitized: UUIDs, session ids, message
and tool-use ids are zeroed; email and org fields are dropped; plan tier, rate-limit
utilization and reset times are placeholders; local paths are replaced with
`/Users/user/...` and `/private/tmp/nixon-cli-XXXXXX/work`. stderr was empty in every run
below, so no stderr samples are kept.

Isolation flags used for every `-p` capture:

```
-p --safe-mode --restricted --tools "" --disallowedTools "mcp__*"
--strict-mcp-config --mcp-config '{"mcpServers":{}}'
--permission-mode dontAsk --permission-prompts none --no-session-persistence
--max-turns 1 --system-prompt-file <tmp>/in/system.txt --model sonnet
```

| File | What | Exit |
|---|---|---|
| `success-text.stdout` | default `--output-format text`, summary prompt on stdin | 0 |
| `success-stream.ndjson` | `--input-format stream-json --output-format stream-json --verbose`, summary | 0 |
| `success-json-schema.ndjson` | as above + `--json-schema` (action-items schema); `structured_output` has 4 items | 0 |
| `not-signed-in-text.stdout` | text mode, `CLAUDE_CONFIG_DIR` pointed at an empty dir | 1 |
| `not-signed-in-stream.ndjson` | stream-json mode, same signed-out config | 1 |
| `auth-status-signed-in.json` | `claude auth status` (JSON is the default) | 0 |
| `auth-status-signed-out.json` | `claude auth status`, empty `CLAUDE_CONFIG_DIR` | 1 |
| `auth-status-signed-out-text.stdout` | `claude auth status --text`, signed out | 1 |
| `usage-limit-stream.SYNTHETIC.ndjson` | **SYNTHETIC.** The not-signed-in capture with the documented usage-limit string swapped in; the `rate_limit_event` with `status: "rejected"` and `error: "rate_limit"` are guesses, not observed | n/a |
| `usage-limit-text.SYNTHETIC.stdout` | **SYNTHETIC.** Documented text-mode string | n/a |
| `api-retry-auth.SYNTHETIC.ndjson` | **SYNTHETIC.** One `system/api_retry` event built from the documented field table | n/a |

Strings for the synthetic files come from https://code.claude.com/docs/en/errors and
https://code.claude.com/docs/en/headless (checked 2026-11-05).
