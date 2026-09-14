# 0035 — Aggregation engine + cross-meeting Ask-AI

- **Status:** Implemented 2026-07-03; manual smoke verified 2026-07-04 (post-amendment flow)
- **Owner agent(s):** rust-core-engineer (engine, gather, commands, events) + llm-pipeline-engineer (prompts, map-reduce, citations, evals) + frontend-engineer (Ask page, ⌘K action, egress preview)
- **Roadmap phase:** Later (0013 Wave 3a; absorbs the Ask-AI half of `specs/0021` per the [2026-07-01 roadmap review](../docs/reviews/2026-07-01-roadmap-review.md) §4)

> **Amendment 2026-07-04 — pre-send preview/consent gate REMOVED (product decision,
> dogfooding day one).** Everything below describing `api_ask_ai_preview`, the egress
> line, the "Send N meetings & ask" confirm button, or Enter-gating is historical: the
> user found the per-question gate excessive — *"if you're pointing at local models all
> your data remains local, and if you've decided to use cloud models your data is going
> to the cloud"*. The consent model is now **provider choice in Settings = egress
> consent**; Enter (or "Ask") starts the run immediately for any provider. The
> `api_ask_ai_preview` command, `EgressPreview`, the provider-locality helpers
> (`provider_is_local` / `is_loopback_endpoint`), the frontend `EgressLine`, and the
> consent-pinned `meetingIds` were deleted with it (git history has them if a passive
> egress indicator is ever wanted). Post-hoc accountability remains: the answer's
> Sources list shows every meeting sent, cited or not. Zero matches now surface as the
> run's own error ("No meetings matched…" — no LLM call is made).

## Context / Problem

The user asked to "Ask AI across all meetings" in 1.0 dogfooding (`specs/0019` note 12) —
analysis across a timeframe, meeting type, or person. `specs/0021` bundled that with search;
the review split it: search shipped as FTS5 (`specs/0033`, done), and Ask-AI was deliberately
held back so it could be built **as the first consumer of the 0013 Wave 3a aggregation
engine** rather than as a second, one-off gather/chunk path that topic roll-ups and pre-call
prep would then duplicate (review §4: "the multi-meeting gather/chunk/answer core is built
once and Ask-AI, topic roll-ups, and pre-call prep are three prompts over it").

**Grounded current state — the parts already exist, disconnected:**

- **Gather (relevance):** `SearchRepository` (`frontend/src-tauri/src/database/repositories/search.rs`)
  queries three external-content FTS5 tables (`transcripts_fts`, `summaries_fts`,
  `meeting_notes_fts`, migration `20260706000000_add_fts5_search.sql`) with bm25 ranking,
  best-hit-per-(meeting, source) grouping, and a hardened MATCH sanitizer
  (`sanitize_match_query`, `search.rs:93-112`). Exposed as `api_search_meetings`
  (`api/api.rs:391-416`) returning `MeetingSearchHit` (`api/api.rs:61-75`, includes the
  best-matching `transcript_id` per meeting). **Caveat for this spec:** the sanitizer ANDs
  every token — correct for search-as-you-type, fatal for a natural-language question
  ("what did we decide about the pricing page" would require all seven words to co-occur).
  The gather stage needs an OR-semantics variant.
- **Gather (metadata):** `meetings.created_at` for date ranges; `meeting_participants`
  (`migrations/20260629000000`, `person_id → people.id`) and `speakers.person_id`
  (`migrations/20260628000001`) for person scoping; `meetings.calendar_event_id`
  (`migrations/20260626000001`) for recurring-series grouping (a *future* consumer's key —
  pre-call prep — not used by Ask-AI v1). People enumerate via `api_list_people`
  (`lib.rs:1007`, `PeopleRepository::list`, `repositories/people.rs:103`).
- **Chunk/answer:** the summary pipeline already has provider-agnostic completion with
  cancellation and retry — `generate_summary()` (`summary/llm_client.rs:137`, all six
  providers incl. the BuiltInAI sidecar), `generate_summary_with_retry`
  (`summary/processor.rs:138`, currently private), script-aware `rough_token_count`
  (`processor.rs:560`), `chunk_text` (`processor.rs:603`), and per-provider context budgets:
  Ollama sized from live model metadata (`METADATA_CACHE`, `summary/service.rs:501-522`),
  BuiltInAI from the model registry (`:523-545`), cloud from `cloud_context_threshold`
  (`service.rs:44-54`). All of that sizing logic is **inlined in
  `SummaryService::process_transcript_background`** and needs extraction to be reusable.
- **Content per meeting:** the final summary is `summary_processes.result` JSON
  (`{"markdown": …, "english_cache": …}`, built by `build_summary_result_json`,
  `summary/service.rs:161-175`), with a `summary_text` generated column extracting
  `$.markdown` (added by the 0033 migration). Notes live in `meeting_notes.notes_markdown`
  (the canonical markdown twin). Raw transcript text via
  `TranscriptsRepository::get_full_transcript` (`repositories/transcript.rs:255`).
- **Progress to the frontend:** summaries are DB-polled (`summary_processes` status +
  `startSummaryPolling`, `frontend/src/hooks/meeting-details/useSummaryGeneration.ts:190`).
  Ask-AI has no process row to poll; the codebase's other long-running work uses Tauri
  events (`transcription-progress`, `transcription-queue-complete` — kebab-case names
  emitted from Rust). Ask-AI should use events, not a new polling table.
- **UI:** the ⌘K palette (`frontend/src/components/CommandPalette/index.tsx`) has a
  data-driven Actions tier (`:234-254`) and already carries the user's typed query in state
  (`:125`) — the natural launch point.

**The privacy problem this spec must design for (roadmap requirement, not optional):** a
cross-meeting question sends *many meetings' content* to the configured provider in one
request burst — the largest single egress the app can make, far bigger than a one-meeting
summary. The UI must surface scope before anything leaves the machine ("this will send N
meetings to {provider}"), and the provider default when summaries use a cloud provider is
roadmap open question 3 (decided 2026-07-03 — see Design).

## Goals

- **One reusable aggregation engine** (Rust, no UI of its own): scope selection → gather
  (FTS5 relevance + metadata filters) → source selection per meeting → budget-bounded
  packing → map-reduce prompting → answer with per-meeting citations. Topic roll-ups
  (0013 3b) and pre-call prep (0013 4b) later become *new prompt sets over this same
  engine* with zero new gather/chunk code.
- **Ask-AI as its first consumer and the only UI shipped here:** free-text question,
  optional scope (date range / person / explicit meetings), answered by the chosen
  provider, with the source meetings cited and linked.
- **Egress preview before send:** when the effective provider is non-local, the user sees
  "This will send N meetings (~X tokens) to {provider}" and must confirm; local providers
  show an on-device notice instead. Nothing is sent before confirmation.
- **Streaming progress:** stage-level Tauri events (gathering → per-meeting map i/N →
  reducing → done) so a multi-minute local-model run is visibly alive and cancelable.
- Works fully offline with Ollama/BuiltInAI.

## Non-goals

- **Topic roll-up summaries and pre-call prep** — named future consumers of the engine;
  their prompts and UI ship in their own specs (0013 3b/4b). This spec only keeps the
  engine API general enough for them.
- **Vector embeddings / semantic retrieval** — keyword FTS gather only; revisit embeddings
  later if recall proves inadequate (explicitly deferred, as in 0021/0033).
- **Chat history / threads / follow-up questions** — v1 is single question → single
  answer, ephemeral (nothing persisted). Conversation threads are a possible v2; the
  engine API takes a single prompt spec so threads would layer on top without rework.
- **Token-by-token answer streaming** — `generate_summary()` is request/response for all
  six providers today; adding SSE streaming across them is its own project. v1 streams
  *stage* progress only.
- **New egress destinations** — only the user-chosen LLM provider, per the privacy
  invariant in `/CLAUDE.md`.

## Approach

Build a new `frontend/src-tauri/src/aggregation/` module that composes existing parts
rather than inventing parallel ones: gather candidate meetings with a **recall-oriented
(OR-semantics) FTS5 query** over the 0033 index intersected with metadata scope filters;
build one **meeting doc** per selected meeting using a **summaries-first source policy**
(summary markdown → notes fallback → raw-transcript fallback, plus transcript excerpts
around the FTS-best segment when the hit was transcript-only — summaries are ~10–50x
cheaper in tokens than transcripts and are already the distilled record); pack docs under
the provider's real context budget (extracted from `SummaryService`'s existing sizing
logic); answer in a single pass when everything fits, otherwise **map per meeting, reduce
once** — meeting granularity (not arbitrary text chunks) is what makes citations exact and
is the key structural difference from the intra-meeting `chunk_text` path, which is kept
only for the rare oversized single-transcript doc.

The engine is parameterized by an `AggregationPrompt` (map + reduce prompt pair) and takes
the LLM call as an injected async closure — that is the whole "consumers are just prompts"
contract, and it makes the map-reduce logic unit-testable without HTTP. Ask-AI supplies a
question-answering prompt pair that enforces `[M#]` citation markers.

IPC is a **preview/confirm pair**: `api_ask_ai_preview` runs gather+packing only (no LLM,
no egress) and returns the meeting list + token estimate + provider locality;
`api_ask_ai_run` executes, emitting `ask-ai-progress` / `ask-ai-complete` / `ask-ai-error`
events, cancelable via a `CancellationToken` registry like `api_cancel_summary`. The UI is
the smallest real surface: one new `/ask` page plus a ⌘K action that carries the typed
query into it — no dashboard changes, no new sidebar section beyond a link.

Alternative considered and rejected: reusing `summary_processes` + DB polling for progress
(what summaries do). Ask-AI has no meeting id to key a process row on and needs no
persistence; events are less plumbing and match the transcription-progress precedent.
Also rejected: hosting the answer inside the ⌘K dialog — a modal palette is the wrong
container for a confirm step, a multi-minute run, and a scrollable cited answer.

No new dependencies, no schema changes, no new egress destination → **no ADR needed**; the
provider-default posture (the one lasting decision) is recorded in this spec per the
roadmap review §3 ("spec-level design requirement rather than a full ADR").

## Design

### Data model

**No migrations.** The engine is read-only over existing tables:

- Candidate ranking: `transcripts_fts` / `summaries_fts` / `meeting_notes_fts`
  (0033 migration) via a new gather query in `repositories/search.rs`.
- Scope filters: `meetings.created_at` (date range); person scope =
  `meeting_participants.person_id = ?` UNION `speakers.person_id = ?` (a person counts if
  they were on the roster *or* actually spoke).
- Doc content, in preference order per meeting:
  1. `summary_processes.summary_text` (the 0033 generated column — display markdown only,
     never `english_cache`/`result_backup`);
  2. `meeting_notes.notes_markdown` (+ `enhanced_markdown` when present);
  3. `TranscriptsRepository::get_full_transcript` (raw transcript), only when neither
     summary nor notes exist (e.g. recorded but never summarized).
  Additionally, when the meeting's best FTS hit for the question is transcript-source
  (i.e. the phrase was *said* but never made the summary/notes), append a bounded excerpt:
  ±N segments (default 15) around the hit's `transcript_id`, ordered by `timestamp`. This
  is the "summaries-first with transcript fallback" policy: raw transcript chunks are
  pulled **only** (a) as the doc body when no summary/notes exist, or (b) as a small
  excerpt when the question's evidence lives only in the transcript.
- Nothing is written. No chat history, no cached answers (v1).

### Engine API (new module `frontend/src-tauri/src/aggregation/`)

```rust
// aggregation/scope.rs
pub struct AggregationScope {
    pub meeting_ids: Option<Vec<String>>, // explicit set — restricts (doesn't skip) FTS ranking²
    pub date_from: Option<String>,        // ISO-8601 UTC instant, inclusive¹
    pub date_to: Option<String>,          // ISO-8601 UTC instant, EXCLUSIVE¹
    pub person_id: Option<String>,        // people.id
}
// ¹ As built: the UI converts local calendar-day bounds to UTC instants (start of first
//   local day → start of the day after the last), compared with datetime() — day-string
//   comparison against the UTC created_at was off by up to a day for non-UTC users.
// ² As built: explicit ids with a usable question still run the FTS ranking restricted to
//   the set (best-hit source/excerpts preserved, order stable); ids without hits append
//   newest-first; dropped is always 0. The run pins the previewed ids this way, so what is
//   sent can never drift from what was consented to (review finding).

// aggregation/gather.rs
pub struct MeetingDoc {
    pub meeting_id: String,
    pub title: String,
    pub created_at: String,
    pub source: DocSource,        // Summary | Notes | Transcript
    pub text: String,             // per source-selection policy above
    pub excerpt: Option<String>,  // transcript window when evidence is transcript-only
}
pub struct GatherResult {
    pub docs: Vec<MeetingDoc>,    // ranked, capped (default max 10 meetings)
    pub dropped: usize,           // matched but over the cap — surfaced in the preview
}
pub async fn gather(pool: &SqlitePool, question: &str, scope: &AggregationScope,
                    max_meetings: usize) -> anyhow::Result<GatherResult>;

// aggregation/engine.rs — the reusable primitive
pub struct AggregationPrompt {          // consumers ARE this struct + a scope
    pub map_system: String,
    pub map_user: fn(&MeetingDoc) -> String,      // or template fill
    pub reduce_system: String,
    pub reduce_user: fn(&str /* numbered doc block or map outputs */) -> String,
}
pub struct AggregationAnswer {
    pub markdown: String,                          // [M#] markers resolved/validated
    pub sources: Vec<SourceMeeting>,               // meeting_id, title, created_at, cited: bool
}
pub async fn run<F>(llm: F, prompt: &AggregationPrompt, docs: &[MeetingDoc],
                    budget_tokens: usize, cancel: &CancellationToken,
                    on_progress: impl Fn(Stage)) -> anyhow::Result<AggregationAnswer>
where F: async Fn(&str, &str) -> Result<String, String>;  // injected → testable, provider-free
```

Engine mechanics (llm-pipeline-engineer owns the prompt semantics, rust-core the plumbing):

- **Gather ranking:** new `SearchRepository::rank_meetings_for_question` beside the
  existing `SEARCH_SQL` (`search.rs:37-75`): same three-way FTS UNION and
  best-per-meeting `MIN(bm25)` grouping, but the MATCH expression is built by a new
  `build_recall_match_expr` — quoted tokens joined with `OR` (reusing the `"…"` escaping
  discipline of `sanitize_match_query`, minus the trailing `*` since questions are complete
  words), after dropping a small English stopword list (what/did/we/the/…). Explicit
  `meeting_ids` scope bypasses ranking; a scope with no usable query terms falls back to
  metadata-only selection ordered by recency, capped at `max_meetings`.
- **Budget + packing:** extract the provider context sizing from
  `SummaryService::process_transcript_background` (`service.rs:501-556`) and
  `cloud_context_threshold` (`service.rs:44`) into a shared
  `summary::resolve_context_budget(provider, model, ollama_endpoint) -> usize`, used by
  both the summary service (behavior unchanged) and this engine. Docs are counted with
  `rough_token_count`; single-pass when `Σdocs + prompt overhead < budget`, else map-reduce.
  A single doc exceeding the budget (raw-transcript fallback case) is pre-reduced with the
  existing `chunk_text` (`processor.rs:603`) + per-chunk map before joining the meeting-level
  reduce.
- **Map stage:** one LLM call per `MeetingDoc` — "extract everything relevant to the
  question from this meeting, verbatim quotes preferred, or say NOTHING RELEVANT" — output
  stays tagged with its meeting number. Uses `generate_summary_with_retry`
  (`processor.rs:138`, visibility raised to `pub(crate)`) for the same bounded-retry
  behavior summaries get; cancellation checked between calls exactly like the chunk loop
  (`processor.rs:900-910`).
- **Reduce stage:** one call over the numbered map outputs (or raw docs when single-pass):
  answer the question in markdown, cite every claim with `[M#]`, say "not found in the
  selected meetings" rather than inventing.
- **Citations:** post-process the answer — validate `[M#]` markers against the doc list,
  strip out-of-range markers, mark each source `cited: true/false`. The frontend renders
  `[M2]` as a link chip to `/meeting-details?id=…`. Uncited sources still appear in a
  "searched but not cited" footnote so egress is fully accounted for.

### Tauri IPC

New `aggregation/commands.rs`, registered in `frontend/src-tauri/src/lib.rs` alongside the
summary commands:

- `api_ask_ai_preview(question: String, scope: AggregationScope) -> EgressPreview` —
  runs gather + source selection + token counting **only**; zero LLM calls, zero egress.

  ```rust
  pub struct EgressPreview {
      pub meetings: Vec<SourceMeeting>,  // what would be sent, listed by title/date
      pub dropped: usize,                // matched beyond the cap
      pub estimated_tokens: usize,       // rough_token_count over packed docs + overhead
      pub provider: String,              // effective provider (settings + TODO default rule)
      pub model: String,
      pub is_local: bool,                // Ollama | BuiltInAI; CustomOpenAI iff loopback host
  }
  ```

  `is_local`: `BuiltInAI` → true; `Ollama` → true only when the configured
  `ollamaEndpoint` is unset (localhost default) or loopback; `CustomOpenAI` → true only
  when the endpoint host is loopback (`localhost`, `127.0.0.0/8`, `::1`); all else false.
  (Tightened from the draft during review: a user-configured *remote* Ollama endpoint is
  real egress and must get the cloud confirm gate.)
- `api_ask_ai_run(question: String, scope: AggregationScope) -> String /* run_id */` —
  re-resolves the provider from `SettingsRepository::get_model_config`
  (`repositories/setting.rs:46`), spawns the engine on `tauri::async_runtime`, stores the
  `CancellationToken` in an `AppState` run registry (same pattern as `api_cancel_summary`,
  `summary/commands.rs:579`).
- `api_cancel_ask_ai(run_id: String)`.
- **Events** (kebab-case, matching `transcription-progress` conventions; payloads
  `rename_all = "camelCase"`):
  - `ask-ai-progress` `{ runId, stage: "gathering" | "mapping" | "reducing", current, total }`
  - `ask-ai-complete` `{ runId, answerMarkdown, sources: [{ meetingId, title, createdAt, cited }] }`
  - `ask-ai-error` `{ runId, message }` (also emitted on cancellation with a distinct flag)

### UI

Smallest real surface — one page plus one palette action (frontend-engineer):

- **`frontend/src/app/ask/page.tsx`** (new; reads `?q=` for a pre-filled question):
  1. Question input + scope row: date-range preset (All time / 30d / 7d / custom), person
     picker fed by `api_list_people` (reuse the picker pattern from
     `components/Participants/`), optional — defaults to "all meetings, ranked by
     relevance".
  2. On question entry (debounced) call `api_ask_ai_preview` and render the **egress
     line**: local → "Runs on this Mac — nothing leaves your machine ({model})"; cloud →
     "**This will send {N} meetings (~{tokens} tokens) to {provider}**" with an
     expandable list of exactly which meetings, and the primary button reading
     "Send {N} meetings & ask" (the count is *in* the consent action, not beside it).
     `dropped > 0` shows "top {N} of {N+dropped} matches".
  3. Run: listen (`@tauri-apps/api/event`) for the three events; show stage progress
     ("Reading meeting 3 of 6…"), a Cancel button wired to `api_cancel_ask_ai`, then the
     answer rendered as markdown with `[M#]` chips linking to
     `/meeting-details?id=…`, and the source list (cited vs. searched-but-uncited).
- **⌘K action** in `components/CommandPalette/index.tsx`: append to the `actions` array
  (`:234-254`) — "Ask AI about your meetings…" → `router.push('/ask?q=' +
  encodeURIComponent(query))`, carrying whatever the user already typed (the palette holds
  it in `query` state, `:125`). No changes to the search tiers.
- No other surfaces (dashboard, sidebar nav link optional — follow the existing nav
  pattern in `components/Sidebar/` if trivial, otherwise skip in v1).

### Provider default (roadmap open question 3)

> **Decided 2026-07-03 (owner accepted the recommendation): Ask-AI follows
> the configured summary provider — no silent local override — but a cloud provider always
> requires the explicit egress confirm above (preview is blocking, count-in-the-button),
> while local providers run without a confirm step.** Rationale: (a) the provider trust
> decision was already made per-provider when the user configured summaries — the
> *quantity* is what changed, and quantity is exactly what the mandatory preview surfaces
> and gates; (b) a silent local default would route a hard task (multi-document synthesis
> with citations) to the weakest model the user has, slowly — making the flagship-adjacent
> feature look broken while *appearing* to be a privacy win the user never sees; (c)
> every-time explicit confirmation of "N meetings to {provider}" is stronger informed
> consent than a default the user must discover. Alternative (the roadmap's suggestion):
> default to Ollama whenever it's configured, with a "use {cloud provider} instead"
> switch on the Ask page — safest posture, worse answers by default; it was not chosen,
> but if revisited, the preview/confirm design above is unchanged and only the
> effective-provider resolution in `api_ask_ai_preview`/`_run` differs. The decision lands
> in this spec's implementation, not an ADR (review §3).

## Tasks

1. [x] **Extract `resolve_context_budget`** from `SummaryService::process_transcript_background`
   (`summary/service.rs:44-54, 501-556`) into a shared helper in `summary/` (service
   behavior unchanged — covered by existing tests); raise `generate_summary_with_retry`
   (`summary/processor.rs:138`) to `pub(crate)`. *(rust-core-engineer)*
2. [x] **Gather:** `build_recall_match_expr` + stopword list +
   `SearchRepository::rank_meetings_for_question` in
   `database/repositories/search.rs` (OR-semantics MATCH, per-meeting best rank, metadata
   scope WHERE clauses: `created_at` range, `meeting_participants`/`speakers` person
   union); unit tests beside `sanitize_tests`. *(rust-core-engineer)*
3. [x] **`aggregation/` module** (`mod.rs`, `scope.rs`, `gather.rs`, `engine.rs`):
   `MeetingDoc` source-selection policy (summary → notes → transcript; transcript excerpt
   window around the best hit's `transcript_id`), budget packing, single-pass vs.
   map-reduce, injected-LLM-closure design, cancellation + progress callback; register
   module in `lib.rs`. *(rust-core-engineer)*
4. [x] **Ask-AI prompt pair + citation post-processing** (`aggregation/prompts.rs`):
   map extraction prompt, reduce QA prompt with `[M#]` citation contract and explicit
   "not found" behavior; marker validation/stripping; a small prompt-eval fixture set
   (question + canned map outputs → reduce output asserted to cite correctly, run with
   the injected fake LLM). *(llm-pipeline-engineer)*
5. [x] **IPC:** `aggregation/commands.rs` — `api_ask_ai_preview`, `api_ask_ai_run`,
   `api_cancel_ask_ai`, run registry in `AppState`, the three `ask-ai-*` events; register
   commands in `lib.rs`; implement the provider-default rule per the decided posture
   (follow summary provider + blocking cloud confirm).
   *(rust-core-engineer)*
6. [x] **Integration tests** `frontend/src-tauri/tests/aggregation_engine.rs` (follow
   `db_lifecycle.rs`/`fts_search.rs` patterns): (a) two fixture meetings whose summaries
   each hold half of an answer → gather ranks both, fake-LLM run cites `[M1]` and `[M2]`;
   (b) person + date scope filters; (c) summaries-first policy (transcript not included
   when a summary exists; excerpt appended when the phrase is transcript-only);
   (d) no-summary meeting falls back to notes, then transcript; (e) preview token
   count/N stable and zero-egress (fake LLM never called); (f) cancellation mid-map.
   *(rust-core-engineer + llm-pipeline-engineer)*
7. [x] **UI:** `app/ask/page.tsx` (question, scope controls, egress preview + confirm
   gating, event-driven progress, cited answer + source links), ⌘K action carrying the
   query; Vitest for the egress-line rendering (local vs. cloud vs. dropped>0) and the
   `[M#]`-chip markdown rendering. *(frontend-engineer)*
8. [x] **Docs & gate:** CHANGELOG entry, `specs/INDEX.md` status flip, `/check` (full
   Definition of Done) + the manual smoke below. *(any owner)*

## Acceptance criteria

Tied to the Definition of Done in `/CLAUDE.md` (1: `cargo check`/`clippy`/`test` clean in
`frontend/src-tauri`; 2: `pnpm lint`/`pnpm test` clean in `frontend`; 3: app launches via
`./clean_run.sh`; 4: record → live transcript → summary smoke intact — the summary path
must be unaffected by the `resolve_context_budget` extraction):

- Asking a question whose answer spans two fixture meetings returns one coherent markdown
  answer citing both meetings, each citation linking to the right meeting (automated with
  the fake LLM; manually with a real provider).
- `api_ask_ai_preview` reports exactly the meetings that `api_ask_ai_run` would send —
  count, titles, and token estimate — and performs zero LLM calls / zero network egress
  (fixture e).
- With a cloud provider, the UI blocks the run behind "Send {N} meetings & ask" showing
  provider name and token estimate; with Ollama/BuiltInAI it states the run is on-device
  and no meeting content leaves the machine (verifiable offline: disconnect network,
  Ask-AI over Ollama completes).
- Source-selection policy holds: a meeting with a summary contributes its summary text,
  not its transcript; a transcript-only evidence hit contributes a bounded excerpt; a
  never-summarized meeting degrades summary → notes → transcript (fixtures c/d).
- Date-range and person scopes exclude out-of-scope meetings from both preview and answer
  (fixture b); a question matching more than the cap surfaces "top N of M" in the preview.
- Progress events fire per stage, cancel stops the run promptly without a stuck UI, and
  an LLM failure surfaces a user-readable error (never a silent hang).
- The engine is consumable without Ask-AI: `aggregation::run` accepts any
  `AggregationPrompt` + injected LLM closure (proven by the tests, which use a non-Ask
  prompt in at least one case) — the contract topic roll-ups and pre-call prep will use.
- Existing summary generation behavior is unchanged (existing summary tests still green).

## Risks / open questions

- **Provider default (roadmap open question 3) — decided 2026-07-03:** follow the
  configured provider + mandatory cloud confirm (see the Design block); the
  default-to-local alternative is documented there if ever revisited.
- **Keyword-only recall:** OR-of-question-terms over FTS5 will miss paraphrases ("pricing"
  vs. "cost"). Accepted for v1 (same posture as 0021/0033); the gather stage is the single
  seam where embeddings would slot in later without touching the engine or UI.
- **Local-model answer quality:** multi-document synthesis with citations is near the
  ceiling for small Ollama models; the map stage (extraction, an easier task) is designed
  to carry most of the load. The prompt-eval fixtures (task 4) are the guardrail; if
  reduce quality is unacceptable locally, that evidence feeds the TODO decision.
- **Token estimates are rough:** `rough_token_count` is a heuristic (script-aware,
  `processor.rs:551-560`); the preview labels the number "~". Budget packing already
  reserves overhead headroom, matching the summary pipeline's tolerance.
- **bm25 not comparable across the three FTS tables** (inherited from 0033, accepted
  there): per-source MIN + per-meeting best keeps ranking sane; the meeting cap (10) and
  the preview's visible list bound the damage of a mis-ranked gather.
- **Long map phases on cloud providers cost real money** (N+1 LLM calls). The preview's
  token estimate is also the cost proxy; summaries-first sourcing keeps N small. Per-call
  cost display is out of scope (no price tables in the app).
- **Cancellation vs. in-flight HTTP:** `generate_summary` checks the token between
  retries/calls but can't abort a request already at the provider; same limitation as
  summary cancellation today. Accepted.
- **v2 candidates (noted, not designed):** chat threads over the same engine; embeddings
  in gather; token streaming; topic roll-ups (0013 3b) and pre-call prep (0013 4b) as the
  next `AggregationPrompt` consumers.

## Verification

Automated (all green via `/check`):

```bash
cd frontend/src-tauri && source ~/.cargo/env && \
  cargo check && cargo clippy && \
  cargo test --features metal --test aggregation_engine --test fts_search --test db_lifecycle
cd frontend && pnpm lint && pnpm test
```

- `aggregation_engine.rs` fixtures (a)–(f) from task 6, all with the injected fake LLM —
  no network in CI. (Fixture (e) retargeted to gather + `estimate_run_tokens` by the
  2026-07-04 amendment.)
- Recall-expression and scope-filter unit tests in `repositories/search.rs`; citation
  post-processing + prompt-eval fixtures in `aggregation/`.
- Vitest: Enter-key/scope helpers, `[M#]` chip rendering.

Manual smoke (dev build, `./dev-vinyl.sh`, dogfood DB — updated for the 2026-07-04
amendment: no preview/confirm step exists):

1. Record or reuse two meetings that discuss the same project on different days; ensure
   both have summaries. ⌘K → type "what did we decide about <project>" → "Ask AI about
   your meetings…" → the Ask page opens pre-filled.
2. Press Enter → the run starts immediately (no confirm) → stage progress → answer cites
   both meetings; each `[M#]` chip opens the right meeting; the Sources list shows
   everything sent, cited or not.
3. Switch to Ollama, turn off Wi-Fi: same question completes on-device.
4. Scope check: set a date range excluding one of the two meetings → the answer no
   longer cites the excluded meeting. Person scope: pick a participant present in only
   one meeting → same narrowing.
5. Cancel a run mid-map (visible with Ollama's pace) → UI returns to idle, no stray
   answer or error after cancellation. A question matching nothing shows the "No
   meetings matched" error without any LLM call.
6. Regression: generate a normal single-meeting summary end-to-end (DoD item 4 —
   confirms the context-budget extraction changed nothing).
