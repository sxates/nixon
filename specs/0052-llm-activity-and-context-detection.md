# 0052 — Background LLM activity indicator + served-context detection

- **Status:** Implemented (pending owner smoke)
- **Owner agent(s):** rust-core-engineer + frontend-engineer
- **Roadmap phase:** Phase 2 (reliability / observability)

## Context / Problem

A user reported sustained fan noise and GPU load from Ollama during meetings, with no
visible explanation — the app showed nothing, and it was not summarizing. Diagnosis traced
it to the background pre-call-prep generator (specs/0036) and three independent defects:

1. **`max_tokens` is unreachable for Ollama.** `provider_config.rs:75` hardcodes
   `LLMProvider::Ollama => (…, None, None, None)` for `(max_tokens, temperature, top_p)`;
   only the `CustomOpenAI` branch ever populates them. `ChatRequest.max_tokens` is
   `Option<u32>` and skipped when `None` (`llm_client.rs:39`), so every Ollama request
   ships with **no output cap**. Generation is bounded only by
   `REQUEST_TIMEOUT_DURATION` (`llm_client.rs:8`, 300s). Observed: a 1,619-token prompt
   producing 17k+ tokens of degenerate output, killed at the 5-minute deadline.

2. **A failing brief retries forever.** `prep_jobs.rs:262` records `status = 'failed'`
   with **no fingerprint and no failure count**, deliberately so the next pass retries.
   There is no give-up. The pass runs every 30 minutes (`REFRESH_INTERVAL`), and each
   attempt burns up to 3 × 300s via `generate_summary_with_retry`
   (`MAX_LLM_RETRIES = 2`, `processor.rs:14`) — roughly a 50% GPU duty cycle,
   indefinitely.

3. **The context budget is derived from the wrong number.** `service.rs:81` computes
   `metadata.context_size - 300`, where `context_size` comes from `/api/show` →
   `<family>.context_length` (`metadata.rs:236-241`) — the model's **architectural
   maximum**, not what the server actually allocates. For `gemma4` that is 262,144, so
   Vinyl budgets ~261,844 tokens against a server serving 65,536. Prep is unaffected (its
   prompts are ~1.6k), but full meeting summarization never reaches the chunking path, so
   anything between the served size and 261k is **silently context-shifted**.

Underlying all three: background LLM work is completely invisible in the UI. There is no
way to see that a task is running, what it is, or that it has been failing for months.

## Goals

- Bound every Ollama generation with an explicit `max_tokens`.
- Stop the prep loop from retrying a permanently-failing brief forever.
- Derive the context budget from the context the server actually serves.
- Surface background LLM activity — running, succeeded, failed — in the UI, with enough
  history to notice a repeating failure.

## Non-goals

- **Suppressing prep during recording.** Owner decision: background generation may run
  while a meeting is recording. The indicator is the mitigation, not a guard.
- **Fixing the degenerate generation itself.** A 1,619-token prompt producing 17k tokens
  at temperature 0.15 is a real bug, but this spec only *bounds* it. Root-causing the loop
  is separate work; the registry's error history is the instrument for it.
- **Replacing existing progress UI.** `ChunkProgressDisplay`, `DeferredBacklogIndicator`,
  and Ask AI's inline progress stay as they are.
- Changing the retry behavior inside `generate_summary_with_retry` (already bounded).

## Approach

Four independent changes plus one new observability surface.

The registry tracks **tasks**, not individual LLM calls: `generate_summary` is the single
choke point for every provider call (both `generate_summary_with_retry` and
`action_items/extractor.rs:486` funnel through it), but it is too low-level to *name* a
unit of work — one summary is many calls. So jobs declare a task and the LLM calls report
progress within it.

All LLM work is recorded, but only `Origin::Background` tasks are surfaced. This gives a
single source of truth without producing two spinners for one job, and lets foreground
tasks be surfaced later without rearchitecting.

## Design

### Data model

Migration `frontend/src-tauri/migrations/<ts>_add_meeting_briefs_failure_count.sql`:

```sql
ALTER TABLE meeting_briefs ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
```

Semantics: incremented on each failed generation pass; reset to 0 on success **or**
whenever `source_fingerprint` changes. A brief with `failure_count >= 3` is skipped by the
background pass until its fingerprint changes or the user retries manually.

"Fingerprint changed" is evaluated in `generate_brief_for_target`, which already computes
the current fingerprint and compares it against the stored row (`prep_jobs.rs:176-183`).
The reset happens there: if the freshly computed fingerprint differs from the stored one,
`failure_count` is zeroed *before* the skip check, so new input always gets a fresh set of
attempts.

Task history is **in-memory only** — no table. It lives in `AppState`, is capped, and is
cleared on restart. The prep loop repeats every 30 minutes, so a single session is more
than enough to reveal a repeating failure, and this keeps prompt/error text off disk.

### Context detection

`ModelMetadata` (`ollama/metadata.rs:13`) gains `served_context: Option<usize>`, populated
from `/api/ps` → the loaded runner's `context_length` (the **allocated** context, which
reflects `OLLAMA_CONTEXT_LENGTH` / Ollama's VRAM-based default, unlike `/api/show`).

`resolve_context_budget` (`summary/service.rs:69`) becomes:

```
budget = min(arch_max, served_ctx) - RESERVE      // RESERVE = 300, unchanged
```

Resolution order when `/api/ps` returns no entry (model idle):
1. last in-memory reading for that model
2. last **persisted** reading (`<app-data>/ollama-context.json`, keyed `"{endpoint}|{model}"`)
3. otherwise a conservative floor of 8192

**Persistence was added after smoke testing** (2026-08-18) found the in-memory-only cache
made *every* cold start take the floor — the model is never loaded at app launch, so the
first Ollama call of each session budgeted 8192 instead of the real served size. Observed
live: `Context budget for gemma4:26b: 8192 tokens (architectural 262144, served None)` — a
32x under-budget that chunks long meetings needlessly. That was a regression against
pre-0052 behavior (which used the architectural max). With the store, only a genuinely
first-ever run hits the floor. The store is best-effort: a missing or corrupt file costs one
conservative pass, never an error.

The `/api/ps` reading refreshes **after a successful LLM call**, when the model is
guaranteed loaded — so detection never forces a cold load (~11s) just to probe. Cached in
the existing `METADATA_CACHE` (5-minute TTL, `service.rs:27`).

### Token caps

`llm_client.rs` gains `DEFAULT_OLLAMA_MAX_TOKENS: u32 = 4096`, applied exactly the way
`DEFAULT_SUMMARY_TEMPERATURE` already is (`llm_client.rs:249`) — so `max_tokens` is never
`None` for Ollama again. Call sites that know their output shape override it:

| Call site | Cap | Rationale |
|---|---|---|
| Prep brief (`prep_jobs.rs`) | 2000 | p90 of healthy runs was ~1,800 |
| Summary chunk map (`processor.rs`) | 1500 | per-chunk summaries are short |
| Final reduce / report | 8192 | full reports are legitimately long |
| Action items (`extractor.rs`) | *default 4096* | see below |

**Action items deliberately do not get a tight cap.** `extractor.rs:490` passes
`max_tokens: None` with the comment *"an explicit cap risks truncated JSON."* That
reasoning holds — a truncated JSON payload is a worse failure than a slow one. It moves
from unbounded to the 4096 default, which bounds the runaway while leaving ample room for
the payload to close.

Effect on the runaway: 4096 at ~60 tok/s is ~70s, versus the 300s timeout today.

### Prep failure backoff

- `prep_jobs.rs:262` (`Err` arm): increment `failure_count` instead of only writing
  `'failed'`.
- `ensure_brief_for_event`: skip a target whose `failure_count >= 3`.
- Reset to 0 on success (`upsert_ready`) and whenever `source_fingerprint` changes.
- Manual retry from the indicator clears the count and forces one pass.

Bounds a permanently-broken brief at ~45 minutes of GPU instead of forever, while still
self-healing from transient failures (Ollama restarting, model swapping).

### LLM activity registry

New module `frontend/src-tauri/src/llm_activity/` — new code goes in new modules per the
specs/0042 file-size ratchet; `processor.rs` and `llm_client.rs` are already large.

```
llm_activity/
  mod.rs        — re-exports
  registry.rs   — LlmTaskRegistry, TaskHandle, TaskKind, Origin, TaskRecord
  commands.rs   — api_llm_activity_snapshot, api_llm_activity_dismiss,
                  api_llm_activity_retry
```

RAII handle so a panicking or early-returning job cannot leave a phantom running task:

```rust
let task = registry.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1");
task.progress(Stage::Mapping { current, total });
task.finish(result);            // Drop without finish => recorded as failed (abandoned)
```

State: a map of running tasks + a `VecDeque<TaskRecord>` of the **last 20 finished
outcomes** (kind, label, started/ended, result, error text). Held in `AppState`.

`TaskKind`: `PrepBrief`, `MeetingSummary`, `ActionItems`, `NoteEnhancement`, `AskAI`,
`Rollup`. `Origin`: `Background` | `Foreground`.

Registry operations must never propagate failure into generation — same contract as the
specs/0034 extraction path. A broken indicator must not break summarization.

### Tauri IPC

Commands registered in `registry.rs` (per specs/0042, **not** `lib.rs`):

- `api_llm_activity_snapshot() -> LlmActivityView` — initial mount
- `api_llm_activity_dismiss()` — clear the sticky failure badge
- `api_llm_activity_retry(meeting_id: String)` — reset `failure_count`, force a prep pass

**Retry is prep-brief-only.** Prep is the one background task with a durable, addressable
artifact (a `meeting_briefs` row keyed by meeting) and a give-up rule to clear. Other task
kinds have no re-runnable handle, so the popover shows **Retry** only on failed
`TaskKind::PrepBrief` records; every other failed record offers **Dismiss** alone.

Event: `llm-activity-changed`, emitting the full `LlmActivityView` on every transition.
Mirrors the existing `prep-brief-*` event pattern.

### UI

- `frontend/src/contexts/LlmActivityProvider.tsx` — `listen` + snapshot on mount, modeled
  on `DeferredBacklogProvider.tsx` (23 lines). Mounted in `layout.tsx` alongside the
  existing providers.
- `frontend/src/components/LlmActivity/LlmActivityRow.tsx` — sidebar footer row:
  - running → spinner + task label (`◌ Preparing brief — Weekly 1:1`)
  - idle → `· Background AI idle`
  - unacknowledged failure → red dot + `● 1 task failed`, **sticky until dismissed**
- `frontend/src/components/LlmActivity/LlmActivityPopover.tsx` — last 20 outcomes with
  error text, **Dismiss**, and **Retry**.

The row is added to `Sidebar/index.tsx` (233 lines — comfortable). Sidebar placement
avoids the bottom-center region already owned by `DeferredBacklogIndicator`
(`layout.tsx:302`), which was explicitly designed as "no nagging".

## Tasks

1. [ ] Migration: `meeting_briefs.failure_count` + repository accessors — *rust-core-engineer*
2. [ ] `served_context` on `ModelMetadata` + `/api/ps` fetch + post-call refresh — *rust-core-engineer*
3. [ ] Clamp `resolve_context_budget` to `min(arch, served)` with cached/floor fallback — *rust-core-engineer*
4. [ ] `DEFAULT_OLLAMA_MAX_TOKENS` + per-call-site overrides — *llm-pipeline-engineer*
5. [ ] `failure_count` increment / skip-at-3 / reset rules in `prep_jobs.rs` — *rust-core-engineer*
6. [ ] `llm_activity/` registry module + RAII handle — *rust-core-engineer*
7. [ ] Instrument prep, summary, action-item, rollup jobs with `registry.start(...)` — *rust-core-engineer*
8. [ ] IPC commands + `llm-activity-changed` event in `registry.rs` — *rust-core-engineer*
9. [ ] `LlmActivityProvider` + sidebar row + popover — *frontend-engineer*

## Acceptance criteria

Ties back to the Definition of Done in `/CLAUDE.md`.

1. `cargo check`, `cargo clippy`, `cargo test` clean in `frontend/src-tauri`.
2. `pnpm lint`, `pnpm test` clean in `frontend`; `scripts/check-file-size.sh` passes.
3. No Ollama request is ever sent without `max_tokens` — asserted by extending the
   existing `chat_request_omits_none_optionals_but_keeps_temperature` test
   (`llm_client.rs:419`).
4. `resolve_context_budget` returns `min(arch, served) - 300`, and falls back to
   cache → 8192 floor when `/api/ps` is empty.
5. A brief failing 3 consecutive passes is skipped on pass 4; a `source_fingerprint`
   change or manual retry resumes it; success resets the count to 0.
6. A running background task appears in the sidebar within one event round-trip; success
   auto-clears; failure persists until dismissed.
7. Dropping a `TaskHandle` without calling `finish` records the task as failed.
8. The record → live transcript → summary smoke path still works.

## Risks / open questions

- **`/api/ps` field shape on Ollama 0.32.12 is unverified.** The `context_length` field
  was confirmed on 0.24.0. If it moved, or is reported per-slot under
  `OLLAMA_NUM_PARALLEL > 1`, detection degrades to the cache/floor path — safe, but
  produces a too-small budget. **Verify on the work laptop early in implementation.**
- Per-slot vs total context under `NUM_PARALLEL > 1` needs confirming before trusting the
  reading. (Separately: Vinyl dispatches chunks strictly serially — `engine.rs:186`,
  `processor.rs:890` are plain `for` loops with awaits, no `join_all` anywhere — so
  `OLLAMA_NUM_PARALLEL=1` is the correct server setting regardless.)
- A 4096 cap could truncate an unusually long final report. Mitigated by the 8192
  override on the reduce path.
- In-memory history is lost on restart, so a failure that only occurs overnight may go
  unseen. Accepted: the prep loop repeats every 30 minutes.

## Verification

```bash
cd frontend/src-tauri && source ~/.cargo/env
cargo test --features metal          # registry, clamp math, failure_count, max_tokens
cd ../ && pnpm lint && pnpm test     # provider reducer + row states
./scripts/check-file-size.sh
```

Manual smoke:

1. `./dev-vinyl.sh`, configure Ollama as the summary provider.
2. Confirm the sidebar row shows `· Background AI idle` at rest.
3. Trigger a prep pass with an upcoming recurring meeting; confirm the row shows the
   spinner and the meeting title, then clears on success.
4. Point Vinyl at a bad model name to force failure; confirm the red dot persists, the
   popover shows the error, Dismiss clears it, and Retry re-runs.
5. Confirm in the Ollama server log that requests now carry `max_tokens` and that no
   request runs to the 300s deadline.
6. With `OLLAMA_CONTEXT_LENGTH=65536`, confirm the logged budget is ~65,236 rather than
   ~261,844.
