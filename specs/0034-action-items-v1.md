# 0034 — Action items v1: extraction + task hub

- **Status:** Done (implemented + review-hardened + dogfood-verified 2026-07-04)
- **Owner agent(s):** llm-pipeline-engineer (extraction prompt, parsing, evals) + rust-core-engineer (migration, repository, diff engine, commands, trigger wiring) + frontend-engineer (meeting section + task hub)
- **Roadmap phase:** Next #4 (2026-07 roadmap; `specs/0013` Wave 3c; [2026-07-01 roadmap review](../docs/reviews/2026-07-01-roadmap-review.md) §2/§4/§5.3)

## Context / Problem

Meetings produce commitments, and Vinyl currently drops them on the floor once the summary
scrolls by. This is the largest remaining gap against the granola.ai bar (Granola extracts
action items by default), and every prerequisite shipped in 1.0: owners resolve through the
`people` table (`migrations/20260628000000_add_people_table.sql`), meetings carry a durable
participant roster (`migrations/20260629000000_add_meeting_participants.sql`) and
`calendar_event_id` (`migrations/20260626000001`), and the summary templates already elicit
action-item sections. Action items are also the prerequisite that makes pre-call prep
(0013 Wave 4b, "what do I owe this meeting?") worth building later.

**Grounded current state:**

- **Summaries are the extraction source and they get regenerated constantly.** The final
  summary is one JSON blob in `summary_processes.result` —
  `{"markdown": …, "english_cache": {…}, "summary_status": {…}}` built by
  `build_summary_result_json` (`frontend/src-tauri/src/summary/service.rs:161-175`) and
  persisted by `SummaryProcessesRepository::update_process_completed`
  (`database/repositories/summary.rs:121-152`). Regeneration (template change, notes edit,
  language change, "Transcribe now" rewrite) resets the process row and stashes a
  `result_backup` (`repositories/summary.rs:85-119`). Naive re-extraction on each completion
  would duplicate every task or wipe completion state — the action-item analog of the
  "manual renames survive diarization re-runs" lesson (0029 WS3.2,
  `repositories/transcript_speaker_overrides.rs`). The codebase has paid for this lesson once.
- **All generation paths converge on one function.** `api_process_transcript`
  (`summary/commands.rs:331`) and `api_generate_summary_for_meeting`
  (`summary/commands.rs:456` — regenerate, auto-summarize, notes-only degrade) both spawn
  `SummaryService::process_transcript_background` (`summary/service.rs:323`), whose success
  arm calls `update_process_completed` at `service.rs:781`. That call site is the single
  natural extraction trigger. The function already receives an unused `_app: AppHandle<R>`
  "(for future use)" (`service.rs:324`) — this feature is the future use.
- **The planning invariant applies** (ROADMAP, new since 1.3): record-only mode + deferred
  transcription mean a meeting may have no transcript — and therefore no summary — for days,
  and "Transcribe now" rewrites transcript rows
  (`audio/retranscription.rs::replace_meeting_transcripts`). Extraction must be event-driven
  off the artifact it extracts from (the summary write), never off "meeting end". Triggering
  at the `update_process_completed` call site satisfies this for free: no summary → no
  extraction; late summary → late extraction; rewritten transcript → user regenerates the
  summary → re-extraction *diff*.
- **Templates already ask for action items, but as free markdown.** `TemplateSection`
  (`summary/templates/types.rs:4-22`) renders section titles + instructions into the prompt;
  `templates/standard_meeting.json`, `project_sync.json`, `retrospective.json`, and
  `sales_marketing_client_call.json` all carry "Action Items"/owner-due sections, some with
  `item_format` markdown tables (e.g. `| **Owner** | Task | Due | … |`). Templates are now
  user-editable and deletable (`specs/0020`), so the presence and shape of those sections is
  **not** a contract we can parse against.
- **Owner resolution material exists.** `MeetingParticipantsRepository::list`
  (`database/repositories/meeting_participant.rs:55`) returns the roster joined to `people`
  (display_name, email). Important quirk: **the app owner is excluded from the roster by
  design** (`migrations/20260629000000` header — "You" is implicit), so "assigned to me"
  cannot be a `people` FK and needs its own flag. `transcripts.channel`
  (`migrations/20260703000002`) and speaker attribution flow into the summary text already
  (`service.rs:344-360` builds a speaker-attributed transcript), so the summary the extractor
  reads generally names who committed to what.
- **Dead column, do not reuse:** the legacy meetily schema has `transcripts.action_items TEXT`
  (`migrations/20250916100000_initial_schema.sql:15`), referenced only by a struct field in
  `database/models.rs:93` and written by nothing. Per-segment storage is the wrong shape;
  leave it dead.
- **LLM plumbing to reuse:** `generate_summary` (`summary/llm_client.rs:137-151`) is a
  provider-agnostic prompt→text call (Ollama/BuiltInAI local, OpenAI/Claude/Groq/OpenRouter/
  CustomOpenAI cloud) with cancellation and per-provider key lookup
  (`SettingsRepository::get_api_key`, `service.rs:416-424`). Extraction is one more call
  through it — no new provider stack.
- **Frontend anchors:** meeting details is a tabbed page (`app/meeting-details/page-content.tsx:90,
  259-267` — summary/transcript/notes); the sidebar nav is the `NAV_ITEMS` array
  (`components/Sidebar/index.tsx:28-54`); summary completion is learned by polling
  (`hooks/meeting-details/useSummaryGeneration.ts:174-190` via `startSummaryPolling`), but
  Rust→frontend events are established elsewhere (`audio/retranscription.rs:123,134,601`),
  so extraction can emit an event instead of adding another poller.

## Goals

- Every generated summary yields structured action items: description, assignee (resolved to
  a `people` row, "me", or an unresolved raw name), a freeform due hint, and a link back to
  the source meeting.
- **Regeneration-safe by design:** re-extraction computes a diff against existing items.
  User-completed, user-dismissed, user-edited, and manual items are **never modified,
  deleted, or duplicated** by a re-run — machine-owned pristine items are the only thing a
  re-run may rewrite.
- Per-meeting UI: an Action items section on the meeting page — check off, edit, dismiss,
  add manual items.
- Cross-meeting task hub: one page listing items across meetings, filterable by status and
  person, each deep-linking to its source meeting. Manual items can also exist standalone
  (no meeting).
- Extraction is background, non-blocking, and failure-isolated: it never delays or fails a
  summary, and it uses the user's configured LLM provider (local Ollama by default posture) —
  no new egress scope beyond the provider that already saw the transcript.
- Old meetings are extractable retroactively via a manual trigger (no bulk backfill job in v1).

## Non-goals

- **Live during-meeting detection** — explicitly cut from v1 (ROADMAP Someday); revisit once
  the extraction path has proven itself.
- Due-date *parsing*, reminders, or notifications. v1 stores the extracted hint verbatim
  (`due_hint`, e.g. "by Friday"); calendar math comes later if ever.
- External sync/export (Apple Reminders, Things, Todoist…). Local-first; nothing leaves the
  machine. Decided 2026-07-03: even manual export is out of v1; revisit only on demonstrated pull.
- FTS indexing of action items. The 0033 external-content pattern
  (`migrations/20260706000000_add_fts5_search.sql`, `database/repositories/search.rs`) makes
  this a small follow-on if search feedback ever asks for it — do not gold-plate v1.
- Cross-meeting dedup ("Alice already owes this from last week") and recurring-series
  roll-ups — that's pre-call prep / aggregation-engine territory (0013 Waves 3a/4b).
- Editing the summary markdown to reflect item state. The summary stays a document; the
  `action_items` table is the source of truth for task state.

## Approach

**Extraction = one structured-output LLM pass over the final summary markdown + the user's
notes, triggered when the summary is persisted; persistence = a diff against the existing
rows, never a wipe-and-reload.**

Why this extraction source over the alternatives:

1. **Parse the summary's "Action Items" markdown section** — rejected. Templates are
   user-editable/deletable (0020), the six built-ins disagree on section names and
   `item_format` shapes, and LLMs honor markdown-table format hints inconsistently. A parser
   here is a permanent brittleness tax and silently dies when a user edits a template.
2. **Dedicated structured pass over the raw transcript** — rejected for v1. It re-pays the
   whole chunking/accounting pipeline (`summary/processor.rs`) for hour-long transcripts,
   is slow on local Ollama (summaries already cost minutes), and duplicates work: the
   summary pass has *already* distilled commitments with speaker attribution.
3. **Make templates emit structured JSON action items inline** — rejected. It couples the
   feature's correctness to every template, including user-edited ones, and mixes two output
   contracts (prose + JSON) in one generation, which degrades both on small local models.
4. **Chosen: a second, small LLM call over the generated summary + notes** (the roadmap
   review §5.3 direction). ~1–3k input tokens regardless of meeting length, one un-chunked
   call through the existing `generate_summary`, template-agnostic, and privacy-neutral —
   the same provider already saw the full transcript, so sending it the summary expands
   nothing. Known trade-off: recall is bounded by summary quality. Acceptable because all
   built-in templates elicit action/next-step content, notes are included as a second
   source, and manual add covers misses.

The LLM returns candidates only; **Rust owns identity, owner resolution, and the diff**:
assignee strings are resolved against the participant roster in code (never trusting the
model with IDs), and candidates are matched to existing rows by a normalized content
fingerprint with a fuzzy fallback. Protected rows (user-touched in any way, or manual) are
invisible walls: candidates matching them are dropped; nothing updates them. A per-meeting
extraction ledger records the summary fingerprint so re-running against an unchanged summary
is a no-op (idempotence), reusing the `stable_text_fingerprint` scheme
(`summary/service.rs:105-115`).

This "machine may only rewrite machine-owned, pristine derivatives" rule is the same pattern
as 0029's speaker overrides; if a third feature needs it (topic suggestions will), promote it
to an ADR then.

## Design

### Data model

New forward-only migration `frontend/src-tauri/migrations/20260707000000_add_action_items.sql`
(naming/idempotence conventions per the 0016/0017 migrations; FKs are documentation-only —
no `PRAGMA foreign_keys` on the pool — so cascades are explicit in the delete transactions):

```sql
CREATE TABLE IF NOT EXISTS action_items (
    id                 TEXT PRIMARY KEY,            -- "ai-<uuid>"
    meeting_id         TEXT,                        -- → meetings.id; NULL = standalone manual to-do
    description        TEXT NOT NULL,
    assignee_person_id TEXT,                        -- → people.id; NULL = me / unresolved / unassigned
    assignee_is_self   INTEGER NOT NULL DEFAULT 0,  -- 1 = the app owner ("me"); owner is NOT a people row
                                                    -- (roster excludes self, migrations/20260629000000)
    assignee_raw       TEXT,                        -- name as extracted when unresolved (display fallback)
    due_hint           TEXT,                        -- verbatim extracted hint ("Friday", "2026-07-10"); no parsing in v1
    status             TEXT NOT NULL DEFAULT 'open',-- 'open' | 'completed' | 'dismissed'
    source             TEXT NOT NULL DEFAULT 'extracted', -- 'extracted' | 'manual'
    user_edited        INTEGER NOT NULL DEFAULT 0,  -- 1 = user changed description/assignee/due → protected
    content_key        TEXT NOT NULL,               -- fingerprint of normalized description (diff identity)
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    completed_at       TEXT                         -- set when status → 'completed'
);
CREATE INDEX IF NOT EXISTS idx_action_items_meeting  ON action_items(meeting_id);
CREATE INDEX IF NOT EXISTS idx_action_items_assignee ON action_items(assignee_person_id);
CREATE INDEX IF NOT EXISTS idx_action_items_status   ON action_items(status);

-- One row per meeting: the extraction ledger (idempotence + debuggability).
CREATE TABLE IF NOT EXISTS action_item_extractions (
    meeting_id          TEXT PRIMARY KEY,           -- → meetings.id
    summary_fingerprint TEXT NOT NULL,              -- stable_text_fingerprint(summary markdown + notes)
    extracted_at        TEXT NOT NULL,
    model_provider      TEXT NOT NULL,
    model_name          TEXT NOT NULL,
    item_count          INTEGER NOT NULL
);
```

Deviations from the 0013 Primitive-2 sketch, deliberate: `owner_person_id` →
`assignee_person_id` + `assignee_is_self` ("owner" already means the app user throughout the
codebase, specs/0018, and the roster's exclude-self design forces the flag);
`due_date` → `due_hint` (honest about no date parsing in v1); `status` gains `'dismissed'`
(rejecting an extracted item must be persistent, or the next re-run resurrects it);
`source_meeting_id` → `meeting_id` (consistent with `meeting_notes`, `meeting_participants`,
`summary_processes`).

**Cascades (explicit, transactional):**
- Meeting delete: add `DELETE FROM action_items WHERE meeting_id = ?` and
  `DELETE FROM action_item_extractions WHERE meeting_id = ?` to
  `MeetingsRepository::delete_meeting_with_transaction`
  (`database/repositories/meeting.rs`, the block around `:580-602` that deletes
  transcripts/summaries/notes).
- Person delete: in `PeopleRepository::delete` (`database/repositories/people.rs:230`), do
  `UPDATE action_items SET assignee_person_id = NULL WHERE assignee_person_id = ?` — the item
  outlives the person; `assignee_raw` keeps the display name.

**Diff algorithm** (pure Rust, unit-testable without an LLM; lives in
`src/action_items/diff.rs`):

1. Skip-check: if `action_item_extractions.summary_fingerprint` for the meeting equals the
   fingerprint of the current summary markdown + notes, do nothing (regeneration that
   produced identical output is a no-op).
2. `content_key(description)` = `stable_text_fingerprint` (FNV-1a, `service.rs:105-115`
   scheme — extract the helper into a shared location or duplicate the 10 lines) over the
   normalized description: NFC, lowercase, punctuation stripped, whitespace collapsed.
3. Partition existing rows for the meeting: **protected** = `source = 'manual'` OR
   `user_edited = 1` OR `status != 'open'`; **pristine** = the rest (extracted, open,
   untouched).
4. Match each candidate, in order: (a) exact `content_key` equality against ALL existing
   rows (protected first); (b) token-set Jaccard similarity ≥ 0.7 (over the normalized
   words) against still-unmatched existing rows — catches the LLM rephrasing "send the deck
   to Alice" as "send deck to Alice".
5. Apply:
   - candidate ↔ **protected** row → drop the candidate (row untouched; no duplicate).
   - candidate ↔ **pristine** row → update that row's `description`/`assignee_*`/`due_hint`/
     `content_key`/`updated_at` in place (id, `created_at`, `status` preserved).
   - unmatched candidate → INSERT (`source = 'extracted'`).
   - unmatched **pristine** row → DELETE (machine-owned content superseded by the new
     summary — exactly how regeneration already treats the summary markdown itself).
   - unmatched **protected** row → keep, untouched, forever.
6. All writes + the ledger upsert happen in one transaction.

**Owner resolution** (Rust, `src/action_items/extractor.rs`): the prompt embeds the roster —
participant display names + emails from `MeetingParticipantsRepository::list`
(`meeting_participant.rs:55`) plus the literal option `"me"` for the meeting owner — and
instructs the model to copy an assignee string from that list or leave it null. Resolution:
case-insensitive match on display name or email → `assignee_person_id`; `"me"`/owner-name →
`assignee_is_self = 1`; anything else → `assignee_raw` only. The model never sees or emits IDs.

**LLM contract** (`src/action_items/extractor.rs`, llm-pipeline-engineer): one call through
`generate_summary` (`llm_client.rs:137`) with `temperature = Some(0.0)`, system prompt
demanding a bare JSON array of `{"description": str, "assignee": str|null, "due": str|null}`
with no prose. Parsing is defensive: strip code fences, find the first `[`…last `]`, strict
`serde_json` parse; on failure retry once with a "return ONLY the JSON array" reminder; on
second failure log a warning and record nothing (background feature — never a user-facing
error, never a failed summary). Provider/model/api-key = **the exact values the summary run
just used** (already in scope in `process_transcript_background`); no separate extraction
provider setting in v1. Decided 2026-07-03: same-as-summary stays the rule — no per-feature
model override setting.

### Tauri IPC

New module `frontend/src-tauri/src/action_items/` (`mod.rs`, `extractor.rs`, `diff.rs`,
`commands.rs` — mirrors the `people/` module layout) plus repository
`database/repositories/action_item.rs`. Commands registered in `lib.rs` next to the
participant block (`lib.rs:998-1000`):

| Command | Signature (conceptual) | Notes |
|---|---|---|
| `api_get_action_items` | `(meeting_id) -> Vec<ActionItem>` | per-meeting section |
| `api_list_action_items` | `(status?, person_id?, mine_only?) -> Vec<ActionItemWithMeeting>` | hub query; joins `meetings(title, created_at)`; default filter `status = 'open'` |
| `api_create_action_item` | `(meeting_id?, description, assignee_person_id?, assignee_is_self?, due_hint?) -> ActionItem` | `source='manual'`; `meeting_id` nullable → standalone to-do from the hub |
| `api_update_action_item` | `(id, description?, assignee_person_id?, assignee_is_self?, assignee_clear?, due_hint?) -> ActionItem` | any content change sets `user_edited = 1` and recomputes `content_key` |
| `api_set_action_item_status` | `(id, status) -> ActionItem` | `'open'\|'completed'\|'dismissed'`; manages `completed_at`; any non-open status protects the row from re-extraction |
| `api_delete_action_item` | `(id) -> bool` | hard delete; UI offers it for **manual** items only (deleting an extracted item would just get re-proposed — that's what *dismiss* is for) |
| `api_extract_action_items` | `(meeting_id) -> u32` | manual trigger: runs the same extract+diff against the stored summary; enables retroactive extraction for pre-0034 meetings and re-try after LLM flakiness; returns item count |

**Event:** `action-items-updated` with payload `{ "meeting_id": string }`, emitted after any
background extraction transaction commits (pattern: `retranscription.rs:123,134,601`). The
meeting page and hub refresh on it; user-initiated commands just use their return values.

**Trigger wiring** (rust-core-engineer): in `process_transcript_background`'s success arm,
immediately after the `update_process_completed` call succeeds (`service.rs:781-793`),
`tauri::async_runtime::spawn` the extraction with the pool, app handle (rename `_app` →
`app`), meeting_id, final markdown, and the run's provider/model/api-key/endpoints. Failure
is logged, never propagated. The user-edit path (`api_save_meeting_summary` →
`update_meeting_summary`, `summary/commands.rs:75`) deliberately does **not** trigger
extraction — hand-edited prose shouldn't churn tasks; the manual command covers intent.

### UI

- **Per-meeting section** — `frontend/src/components/MeetingDetails/ActionItemsSection.tsx`,
  rendered on the **Summary tab** of `app/meeting-details/page-content.tsx` (below the
  AISummary content, `page-content.tsx:395-398` region): items derive from the summary, so
  they live with it (Granola's layout). Rows: checkbox (open ↔ completed), description
  (inline-editable), assignee chip (picker over roster + "Me" + free text; reuse the
  Participants picker components under `components/Participants/`), due-hint text, overflow
  menu (dismiss for extracted / delete for manual). "Add item" row at the bottom. Empty
  states: no summary yet → "Action items appear after the summary is generated"; summary but
  zero items → "None found" + add row + a "Scan again" button (`api_extract_action_items`,
  also the retroactive path for old meetings). Listens for `action-items-updated`.
- **Task hub** — new route `frontend/src/app/tasks/page.tsx` (+ `_components/` as needed).
  Sidebar entry in `NAV_ITEMS` (`components/Sidebar/index.tsx:28-54`), label "Action items",
  `isActive: p?.startsWith('/tasks')`. Decided 2026-07-03: top-level sidebar item between
  "All meetings" and "People", labeled "Action items".
  Layout: filter bar (status segmented control defaulting to
  Open; person filter: Anyone / Me / a person from `api_get_people`), grouped by source
  meeting (standalone items in an "Unattached" group), each meeting header deep-linking to
  `/meeting-details?id=…`. Same row component as the meeting section. "New item" creates a
  standalone manual to-do.
- **Types:** `ActionItem` / `ActionItemWithMeeting` in `frontend/src/types/` alongside the
  existing meeting types.

## Tasks

1. [ ] **rust-core-engineer** — migration
   `frontend/src-tauri/migrations/20260707000000_add_action_items.sql` (tables + indexes as
   above, header comments per house style); explicit cascades in
   `database/repositories/meeting.rs::delete_meeting_with_transaction` and
   `database/repositories/people.rs::delete`.
2. [ ] **rust-core-engineer** — `database/repositories/action_item.rs`
   (`ActionItemsRepository`: get/list-with-filters/create/update/set_status/delete +
   `replace_extracted` transaction used by the diff) + registration in
   `database/repositories/mod.rs`; sqlx unit tests in the file (pattern:
   `meeting_participant.rs:228+` test pool).
3. [ ] **rust-core-engineer** — `src/action_items/diff.rs`: normalization, `content_key`,
   Jaccard matcher, the apply rules, the ledger skip-check; exhaustive unit tests (see
   Verification) — this lands **before** any LLM work so the safety property is proven in
   isolation.
4. [ ] **llm-pipeline-engineer** — `src/action_items/extractor.rs`: prompt (JSON-array
   contract, roster embedding, "me" convention), defensive parse + one retry, owner
   resolution; unit tests over canned LLM outputs (clean JSON, code-fenced, prose-wrapped,
   invalid) and over fixture summaries from all six built-in templates
   (`frontend/src-tauri/templates/*.json` shapes).
5. [ ] **rust-core-engineer** — trigger wiring in `summary/service.rs` (spawn after
   `:781-793`, thread provider/model/key/endpoints through), `src/action_items/commands.rs`
   (7 commands), `action-items-updated` event, module + command registration in `lib.rs`.
6. [ ] **frontend-engineer** — `frontend/src/types/` additions;
   `components/MeetingDetails/ActionItemsSection.tsx` wired into
   `app/meeting-details/page-content.tsx` (Summary tab), event listener, empty states,
   "Scan again".
7. [ ] **frontend-engineer** — `app/tasks/page.tsx` hub (filters, grouping, deep-links,
   standalone add), sidebar `NAV_ITEMS` entry; Vitest coverage for filter/grouping logic
   (pattern: existing `__tests__` under `app/meeting-details/`).
8. [ ] **llm-pipeline-engineer + rust-core-engineer** — end-to-end fixture test
   `frontend/src-tauri/tests/action_items_lifecycle.rs`: mocked-LLM extract → user
   mutations → re-extract diff assertions (the Verification matrix); CHANGELOG entry under
   `[Unreleased]`.

## Acceptance criteria

Tied to the Definition of Done in `/CLAUDE.md` (gates 1–4 all apply; layers 1–2 run in CI):

1. `cargo check` / `clippy` / `cargo test` clean in `frontend/src-tauri`; `pnpm lint` /
   `pnpm test` clean in `frontend`; app launches via `./clean_run.sh`; the record → live
   transcript → summary smoke path is unchanged.
2. Generating a summary for a meeting whose transcript contains clear commitments produces
   action items within seconds of summary completion, with assignees resolved to roster
   people (verified against a real meeting on local Ollama — no cloud key configured — and
   `assignee_is_self` set for first-person commitments).
3. **The regeneration safety property:** with items extracted, (a) completing one, (b)
   editing another's description, (c) dismissing a third, then regenerating the summary
   (same or different template) leaves all three exactly as the user left them — same ids,
   same status, same text — and creates **zero duplicates** of them. Pristine items are
   updated or replaced to match the new summary. Covered by the
   `action_items_lifecycle` cargo test *and* one manual pass.
4. Re-running extraction against an unchanged summary is a no-op (ledger fingerprint check;
   asserted in tests — zero row churn, `updated_at` untouched).
5. Extraction failure (provider down, malformed JSON twice) leaves the summary intact and
   visible, logs a warning, and the meeting shows the empty-state with "Scan again" —
   no user-facing error, no failed summary process.
6. A summary generated for a record-only meeting transcribed days later still triggers
   extraction (trigger is the summary write — demonstrated by the deferred-transcription
   path reaching the same `update_process_completed` call site).
7. The hub lists items across ≥2 meetings, filters by status and by person (including
   "Me"), deep-links to the source meeting, and supports a standalone manual item
   (`meeting_id IS NULL`). A manual item on a meeting survives that meeting's re-extraction.
8. Deleting a meeting removes its action items and ledger row; deleting a person nulls the
   assignee but keeps the item with `assignee_raw` displayed.
9. No egress beyond the single configured-LLM extraction call (inspection of the code path:
   only `generate_summary` is invoked, with the summary+notes text).

## Risks / open questions

- **Recall bounded by summary quality.** A template with no action-oriented section (or a
  weak local model) yields thin extractions. Mitigations: notes included as a second source,
  "Scan again", manual add. If dogfooding shows real misses, the v2 lever is a
  transcript-grounded pass — deliberately deferred.
- **Small-model JSON discipline.** Local models sometimes wrap JSON in prose. The
  fence-strip + bracket-slice + one retry parser is the mitigation; the extractor eval
  fixtures (Task 4) must include Ollama-typical malformed outputs.
- **Fuzzy-match thresholds.** Jaccard 0.7 is a starting point; too low merges distinct
  tasks (updating a pristine row with the wrong candidate), too high duplicates rephrased
  ones. The diff tests pin behavior at the boundary; tune with fixture pairs from real
  regenerations during dogfooding. Failure is bounded: it only ever mis-handles *pristine*
  rows — the user-owned set is protected by identity rules, not similarity.
- **Roster gaps.** Ad-hoc meetings may have empty rosters; assignees then land in
  `assignee_raw` unresolved. Acceptable v1 degrade; the assignee picker lets the user fix it
  (which sets `user_edited` and protects the row).
- **Pattern generalization.** The protected/pristine diff rule will be needed again (topic
  suggestions, any regenerable derivative). If/when a third consumer appears, write the ADR
  ("machine may only rewrite machine-owned pristine derivatives") — not required for this
  spec alone.
- **Decided 2026-07-03 (owner accepted recommendations):** hub is a top-level sidebar item
  "Action items" between "All meetings" and "People"; external sync/export fully out of v1;
  extraction uses the summary run's exact provider/model — no separate setting.

## Verification

**Automated (no mic, no LLM — mocked extractor output):**

```bash
cd frontend/src-tauri && source ~/.cargo/env
cargo test --features metal action_items          # diff + extractor + repo unit tests
cargo test --features metal --test action_items_lifecycle
cd ../ && pnpm test                                # hub filter/grouping Vitest
```

The `action_items_lifecycle` fixture matrix (each row = a test):

| # | Setup | Re-run input | Expected |
|---|---|---|---|
| 1 | fresh meeting, 3 candidates | — | 3 rows inserted, `source='extracted'`, ledger written |
| 2 | #1, then same summary re-run | identical fingerprint | no-op: zero churn, `updated_at` unchanged |
| 3 | #1, complete item A | new summary still containing A (reworded ≥0.7 sim) | A untouched (`completed`, same id); **no duplicate of A** |
| 4 | #1, edit item B's text | new summary containing original-B phrasing | B untouched (`user_edited=1`); no duplicate |
| 5 | #1, dismiss item C | new summary containing C | C stays `dismissed`; no duplicate |
| 6 | #1 pristine item D | new summary omitting D | D deleted |
| 7 | #1 pristine item E | new summary rephrasing E (Jaccard ≥ 0.7) | E updated in place, id/created_at kept |
| 8 | manual item M on meeting | any re-run | M untouched |
| 9 | assignee "Alice" in roster / "Bob" not / "me" | extraction | person_id set / `assignee_raw='Bob'` / `assignee_is_self=1` |
| 10 | LLM returns fenced/prose/invalid JSON | extraction | fenced+prose parse; invalid → retry → warn, zero rows, summary intact |
| 11 | delete meeting / delete person | — | cascades per acceptance #8 |

**Manual smoke (dev build, `./dev-vinyl.sh`, local Ollama):** record a short meeting speaking
three explicit commitments ("I'll send the deck Friday; Alice will book the room") with Alice
on the roster → summary completes → items appear on the Summary tab with correct assignees →
complete one, edit one, dismiss one → switch template and regenerate → the three survive
untouched, no duplicates → hub shows the open remainder, filters by Me, deep-links back →
create a standalone to-do from the hub → record-only meeting: record without transcription,
"Transcribe now" later, generate summary → items appear then.
