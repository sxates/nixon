# ADR-0018: Outside agents may read meeting data (read-only, opt-in, logged)

Status: Accepted
Date: 2026-11-08

## Context

Nixon's promise is that meeting audio, transcripts and notes stay on the Mac. The one
sanctioned exception so far is a user-chosen summarization provider (ADR-0009, ADR-0017).

People also want to ask an AI coding assistant about their meetings from another project
("what feedback came up about X?") and get an answer that links back to the moment it was
said. That means a second program reads meeting data on the user's behalf, and whatever it
reads then travels to that assistant's model provider. Nixon cannot stop that part, so the
decision is mostly about making the access deliberate, narrow, visible and reversible.

## Decision

Add a **read-only MCP server** that is a mode of the app binary itself, spoken over stdio.

- **Invocation.** `<app exe> --mcp --data-dir <dir> --link-scheme <nixon|nixon-dev>`,
  launched by Claude Code, not by Nixon. The code is `frontend/src-tauri/src/agent_access/`.
  Production registers as `nixon` with link scheme `nixon`; the dev build (`ai.vinyl.app.debug`)
  registers as `nixon-dev` with `nixon-dev`, so the two never read each other's data (ADR-0004).
- **Registration.** The Settings switch (off by default) runs `claude mcp add --scope user`.
  Switching it off removes the registration.
- **Gate per call.** `agent_access.json` in the data directory is re-read on every tool call.
  When it says off, the call is refused, so turning the switch off takes effect mid-session
  without restarting the assistant.
- **Read-only database.** The meetings database is opened read-only. Migrations never run in
  this mode, and nothing in it writes to `meeting_minutes.sqlite`. If the schema is behind the
  app's, the tool says to open Nixon once to finish updating.
- **Access log.** Every call that reaches the gate is recorded (tool, arguments, meetings read)
  in a separate store, `agent_access_log.sqlite`, kept 90 days (pruned when the mode starts).
  Settings shows it.
- **stdout discipline.** In this mode stdout carries only MCP JSON-RPC. Diagnostics go to stderr.
- **Prompt-injection hygiene.** Meeting text is wrapped in explicit untrusted-content markers,
  and any copy of the marker phrase inside the text is rewritten first, so meeting text cannot
  forge a closing marker. Calls with bad arguments return results flagged `isError`.
- **Deep links.** `nixon://meeting/<id>` and `nixon://meeting/<id>?t=<seconds>` (and
  `nixon-dev://`) open Nixon on that meeting, scrolled to the moment. Links only navigate; they
  never record, delete, summarize or change data.

### Rejected alternatives

- **A localhost HTTP/SSE server inside the app.** It opens a port any local process, or any web
  page via the browser, could try to reach, and it only works while Nixon is running.
- **A separate sidecar crate.** The repository code is entangled with the app crate, so a
  sidecar would duplicate or fork it. The app binary is already signed and notarized, so a mode
  of it adds no new artifact to sign.

## Consequences

- **Data leaves via the assistant's provider once read.** Opting in means the text the assistant
  reads is sent to its provider under the user's own plan. The Settings copy and the privacy
  policy say so. The default stays off.
- **Prompt injection is mitigated, not eliminated.** The markers help a well-behaved model tell
  data from instructions; they are not a guarantee. The server being read-only bounds the harm:
  a hostile transcript cannot make Nixon change anything, only mislead the assistant.
- **No new network surface** in Nixon: the transport is a pipe to a process the assistant starts.
- Known gaps at ship time:
  - Visibility of the registration in the Claude Code desktop app is not yet verified (owner
    smoke pending); the command-line client is the tested path.
  - Tool calls that fail the MCP layer's schema validation are rejected before the gate runs, so
    they are not in the access log. They return an error and read no data.
