# 0021 — Meeting search & cross-meeting Ask-AI

- **Status:** Draft
- **Owner agent(s):** rust-core-engineer + llm-pipeline-engineer + frontend-engineer
- **Roadmap phase:** Post-1.0 (graduated from `specs/0019` WS5)

## Context / Problem

From 1.0 testing (notes 11 & 12 in `specs/0019`):
- The prominent "Search meetings" input does nothing.
- The user wants to "Ask AI" across all meetings — analysis across a timeframe, meeting type, or
  person.

**Current state (grounded):**
- The dashboard "Search meetings" pill is **decorative**: `aria-hidden`, a ⌘K cue, not an input
  (`frontend/src/app/page.tsx:325-334`; duplicated in `design-preview/page.tsx:414`).
- The real ⌘K `CommandPalette` (`components/CommandPalette/index.tsx`) loads meetings via
  `api_get_meetings` and filters **titles only**, client-side (`:65,133-134`). No body search.
- Orphaned transcript-search backend exists but **nothing renders it**: `api_search_transcripts`
  (`api/api.rs:466-494`) → `TranscriptsRepository::search_transcripts` (`transcript.rs:235-269`,
  naive `LIKE` + `get_match_context`), exposed via `SidebarProvider` (`:200-219`) with zero
  consumers. **No SQLite FTS** anywhere (no FTS5 in migrations).
- **No** Ask-AI / chat / cross-meeting query feature (all "cross-meeting" code is speaker/voice
  identity, unrelated). Reusable LLM plumbing: provider-agnostic `generate_summary()`
  (`summary/llm_client.rs:113`); chunk/combine (`processor.rs:514+, 640-660`); data access
  `api_get_meetings`/`_meeting_transcripts`/`_summary`/`_meeting_notes`; person filtering via
  speaker→person joins.

## Goals

- Real meeting search that matches titles **and** transcript/summary bodies, surfaced in a usable
  UI with result snippets.
- An "Ask AI across meetings" feature: free-text question scoped by timeframe, meeting
  type/title, and/or person, answered by the user's configured LLM provider.

## Non-goals

- Semantic/vector search (keep to keyword/FTS this round; revisit embeddings later).
- Streaming token-by-token UI if it complicates delivery — polling is acceptable initially.
- Sending data to any provider beyond the user-chosen one (privacy invariant per `/CLAUDE.md`).

## Approach

**Search:** build SQLite **FTS5** over transcripts (and summaries) with populate triggers, replace
the `LIKE` query, and wire a real search surface (dashboard input + command palette) that renders
snippets and links to the meeting. Prefer FTS over the existing `LIKE` for quality and speed.

**Ask-AI:** a new command gathers content across a filter, assembles a chunked prompt reusing the
summary chunk/combine pattern, calls `generate_summary()`, and returns an answer (with the
meetings it drew from). New frontend surface for the query + scope controls.

## Design

### Data model
- FTS5 virtual table(s) over `transcripts` (text, meeting_id) and optionally `summaries`, with
  `AFTER INSERT/UPDATE/DELETE` triggers to keep them in sync; migration under
  `frontend/src-tauri/migrations/`. Backfill existing rows in the migration.

### Tauri IPC
- Replace/extend `api_search_transcripts` (`api/api.rs:466`, `transcript.rs:235`) to use FTS
  (`MATCH`) with ranked snippets; add a meetings-level search returning meeting hits.
- New `api_ask_meetings({ question, timeframe?, titleFilter?, personId? })` in a new module (or
  alongside `summary/commands.rs`): resolve the meeting set, gather transcripts/summaries, chunk,
  call `generate_summary()` (`llm_client.rs:113`), return `{ answer, sourceMeetingIds }`. Reuse
  the cancellation/polling pattern from `summary/service.rs` / `SidebarProvider`.

### UI
- Make the dashboard pill (`page.tsx:325`) a real input, or route ⌘K `CommandPalette` to call the
  FTS search and render snippet results that deep-link to the meeting + matched line.
- New Ask-AI surface (command-palette action or a dedicated page) with scope controls (date range,
  title/type, person picker — reuse `api_list_people`) and an answer panel listing source meetings.

## Tasks
1. [ ] FTS5 migration + triggers + backfill (rust-core-engineer).
2. [ ] FTS-backed search query + ranked snippets; update/replace `search_transcripts` (rust-core).
3. [ ] Wire a real search input + results UI (dashboard and/or CommandPalette) (frontend-engineer).
4. [ ] `api_ask_meetings`: scope resolution, gather, chunk, LLM call, sources (rust-core + llm-pipeline).
5. [ ] Ask-AI frontend surface with scope controls + answer/sources panel (frontend-engineer).
6. [ ] Large-corpus handling: chunk/combine + token budgeting; cancellation (llm-pipeline-engineer).

## Acceptance criteria
- Typing in the search surface returns meetings matching title or body, with snippets, linking to
  the meeting; the previously-dead input now works.
- Ask-AI answers a question scoped to a timeframe / title / person using only the configured
  provider, and lists which meetings informed the answer.
- Works on a large corpus without exceeding context (chunking) and is cancelable.
- DoD per `/CLAUDE.md`.

## Risks / open questions
- FTS5 availability in the bundled SQLite (verify the sqlx/bundled build enables FTS5).
- Trigger/backfill cost on large existing DBs (run once in migration; measure).
- Ask-AI scope when "meeting type" isn't a stored field — derive from title/template until a real
  type exists (see `specs/0020` template work).
- Privacy: ensure Ask-AI only sends to the user-chosen provider; surface which provider is used.

## Verification
`cargo test` search fixtures (FTS hit/snippet, person filter); manual: search a known phrase →
correct meeting + snippet; Ask-AI "what did we decide about X across last month" → coherent answer
citing sources; cancel mid-run.
