# ADR-0017: Subscription CLI providers (a new LLM provider class)

Status: Accepted
Date: 2026-11-05

## Context

Nixon summarizes through a user-chosen LLM provider: local (Ollama, the built-in model) or a
cloud API with a key in the Keychain (ADR-0009). Many users already pay for an AI coding
assistant and have its CLI installed and signed in. Letting them reuse that subscription
means no API key and no extra bill.

This is a new class of provider. It runs a third-party program on the user's Mac, it reads
the user's credentials (not ours), and the program is built to act as an agent: it has tools,
loads instruction files, and keeps session history. All of that is wrong for a one-shot
"summarize this transcript" call, and some of it would leak or act on the user's machine.

Claude Code ships first (spec 0085 W1). Codex follows (W2) and must pass its own isolation
spike before it ships. Gemini was dropped: no supportable path exists.

## Decision

Treat the installed, signed-in CLI as an LLM provider (`LLMProvider::ClaudeCode`, later
`CodexCli`), run as a **one-shot isolated subprocess**. The code is `frontend/src-tauri/src/cli_llm/`
(`discovery.rs`, `claude.rs`, `runner.rs`, `errors.rs`, `auth.rs`, `commands.rs`, `config.rs`).

### 1. Isolation contract

Every call, in `runner.rs` and `claude.rs`:

- **Flags.** `-p --safe-mode --restricted --tools "" --disallowedTools "mcp__*"
  --strict-mcp-config --mcp-config '{"mcpServers":{}}' --permission-mode dontAsk
  --permission-prompts none --no-session-persistence --max-turns 1 --system-prompt-file <file>
  --input-format stream-json --output-format stream-json --verbose --model sonnet`, plus
  `--json-schema` for extraction. The model is pinned to `sonnet`, with no picker.
  Overlapping flags are kept on purpose as defence in depth.
- **Clean environment.** `env_clear()`, then only `HOME`, `USER`, `LOGNAME`, `PATH` (keeps
  `/usr/bin`), `LANG`, `TERM=dumb`, `NO_COLOR`, `TMPDIR`, and the CLI's no-autoupdate,
  no-telemetry and no-prompt-history switches. Nothing from Nixon's environment is inherited.
- **Empty working directory.** A fresh `nixon-cli-XXXX/work` directory under a mode-0700
  root. The name never contains a meeting title. No project files, CLAUDE.md or AGENTS.md
  can be found there or in a parent.
- **Stdin only.** The prompt goes in on stdin, never in argv (argv is visible to every
  process via `ps`). The system prompt goes in a file inside the 0700 root.
- **No session persistence.** `--no-session-persistence` was verified with a scratch config
  directory: no transcript, history line or memory directory is written.
- **Own process group.** On cancel, timeout, oversized output or drop, the whole group is
  SIGKILLed (npm-installed CLIs fork). `kill_all()` at app quit sends one SIGKILL per group;
  a process forking at that instant may escape. Accepted.
- **Limits.** 300 s per call, 10 s for `--version` / auth probes, capped stdout. The CLI
  must be at least `min_version` (2.1.290 for Claude Code, which fixed two `--restricted`
  holes).

### 2. The init tripwire, and its accepted risk

The CLI emits `system/init` (listing `tools`, `mcp_servers`, `plugins`) only **after** it has
read the first user message, so a check before writing stdin is impossible. The check is a
tripwire instead:

1. Write the prompt to stdin.
2. Read events until `system/init` (about 0.16 s after spawn; the first answer takes 5 s or
   more).
3. Allow only `tools == []` (or `["StructuredOutput"]` when a schema is passed),
   `mcp_servers == []`, and plugins whose `path == "builtin"`. Anything else kills the
   process group at once and fails with `IsolationFailed`.

**Accepted risk.** If isolation is broken, the prompt has already reached the CLI, and
possibly the API, when the tripwire fires. The kill happens before any answer is used or
stored. This supersedes the original acceptance criterion that required the check before
stdin.

**Fail closed.** If a Claude Code release renames `tools`, `mcp_servers` or `plugins`, every
call fails with `IsolationFailed` until Nixon is updated. This is deliberate: a broken
isolation check must never degrade to "no check".

### 3. No credential handling, ever

Nixon never reads, copies, stores or sets credentials, tokens or `~/.claude` contents, never
sets the config-dir variable outside tests, and has no sign-in flow (no Terminal button, no
commands). It only asks the CLI whether it is signed in (`auth status`, parsing `loggedIn`
and `authMethod` alone) and runs it under the user's own login. The CLI reads its own
Keychain item as it does when the user runs it. On an auth failure Nixon tells the user to
make sure they are signed in to that CLI. A "Locate" override for the binary path is the only
configuration stored (no secrets).

### 4. Privacy

The transcript is sent to the vendor under the user's own account, the same egress class as
a cloud API provider, and choosing the provider is the consent. Claude Code additionally
attaches the **account email, the working-directory path and machine details** (platform,
shell, OS version, date) to each request; no flag removes this. It goes to the vendor who
already holds it, but the privacy copy says so. Prompts must not ask the model to echo its
instructions. Nixon never logs prompt, stdin, stdout or stderr bodies.

### 5. Never the default

A CLI provider is never the provider on a fresh database, never the recommended model
(`builtin_ai_get_recommended_model`), and never offered by onboarding. It appears only in
Settings, after the user chooses it, and the login-shell `PATH` probe runs only after that
choice. Unit tests assert each of these.

### 6. Scheduling through the gate

CLI calls go through `summary/llm_gate.rs` (`is_local` becomes `is_scheduled` and covers the
CLI providers). They are serialized because each is a heavyweight process that spends the
user's plan, not because a GPU is shared. Background work (prep briefs, auto-summaries) waits
for interactive calls and is never preempted (`supports_preemption` stays Ollama-only).
Interactive calls never wait on background ones. Background prep briefs may run on a CLI
provider.

### 7. Error classification contract

`cli_llm/errors.rs::CliError` is the only vocabulary: `NotFound`, `TooOld`, `NotSignedIn`,
`UsageLimit`, `RateLimited`, `IsolationFailed`, `Timeout`, `ProbeTimeout`, `Cancelled`,
`BadOutput`, `OutputTooLarge`, `Crashed`, `Unknown`.

- Classify on `is_error`, the assistant `error` field, the result text and the exit code,
  **never on `subtype`** (a signed-out run reports `subtype: "success"` with `is_error: true`).
- Usage limits match `hit your ... limit` generically (the pinned model has its own limit),
  with the `rate_limit_event` as a second signal and its reset time shown.
- The user-facing message strings are pinned to the markers that
  `summary/llm_retry.rs::is_retryable_llm_error` matches, so a limit or sign-in error is
  retried by hand (Retry) while a transient one backs off. A failed call never writes a
  summary row.
- Extraction treats a missing `structured_output` as a failure, not zero items.

### 8. Terms posture

Owner decision (2026-11-05): ship as designed. Nixon runs the **unmodified** binary under the
user's **own** login, on the user's own Mac, for that user, and never touches credentials. The
vendor's terms bar third parties from offering the vendor's login in their own apps or routing
requests through plan credentials on behalf of users, while saying they do not prevent an end
user from signing in to the unmodified CLI with their own subscription. Whether Nixon's use
counts as "routing" is not ours to decide, and there is enforcement precedent against
third-party harnesses. We record this as an unresolved risk, not a conclusion. Consequences
for the product: never default (above); UI copy describes it as "Claude Code (your
subscription)", never as a Nixon feature name; plan and privacy copy are explicit; the
provider can be removed quickly if the terms change.

## Consequences

- Users with a subscription get summaries, action items, Ask AI and briefs with no API key.
- Nixon depends on a vendor CLI's flags and event shapes. A removed flag fails loudly
  (non-zero exit) and a renamed init field fails closed, so a CLI update can disable the
  provider until Nixon ships a fix. `min_version` is a floor, not a pin.
- The prompt reaches the vendor before the init tripwire can fire (accepted, section 2).
- Each call costs about 0.2 s of process start plus roughly 1.1k tokens of vendor-added
  context, so most meetings stay single-pass (context budget 180k for Claude Code).
- Codex ships only after its own spike shows its instruction files can be suppressed, or the
  disclosure ("Codex may add your own instructions from `~/.codex/AGENTS.md`") ships with it.
- Terms risk remains open; see section 8.

## Alternatives considered

- **Long-lived CLI session.** Rejected. Cold start is about 0.2 s against a 6 to 9 s answer,
  and a persistent agent session keeps state we don't want.
- **Anthropic or OpenAI API with a subscription token.** Rejected. It would mean handling
  credentials, which the terms forbid and this ADR rules out.
- **Check init before writing stdin.** Impossible: init is emitted only after the first
  message is read.
- **Prompt in argv.** Rejected: visible in the process list.
