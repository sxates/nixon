# 0036 — Pre-call prep + Today view

- **Status:** Implemented 2026-07-04 (automated gate green; native manual smoke pending — see Verification)
- **Owner agent(s):** rust-core-engineer (series detection, brief cache, background generation, adoption) + llm-pipeline-engineer (pre-call-prep prompt, prep-notes-aware summary) + frontend-engineer (Today timeline, Prep tab)
- **Roadmap phase:** Later → **0013 Wave 4b ("pre-call prep")** + the greenfield "Today view" (0007 deferred it). Graduates ROADMAP "Pre-call prep v1".

## Context / Problem

The app today is built around *reaction* — you record a meeting, it transcribes and summarizes,
and the result drops into a "Recent recordings" list. Nothing helps you *prepare* for a meeting
before it starts. For the target user — a VP of product design running a standing cadence of
recurring reviews with leadership and the UX team — the highest-value question is almost always
**"what did we decide last time, and what's still open?"** just before walking into the next
occurrence of the same recurring meeting. That context exists in the app (prior summaries,
decisions, action items) but is scattered across separate meeting records and only reachable by
manual search after the fact.

Two concrete gaps:

1. **No pre-call surface.** An upcoming calendar event's only click target is Join & Record
   (`DayAgenda.tsx` `AgendaRow`, handlers at `DayAgenda.tsx:246-263`). There is no read-only
   "prep" view, and an upcoming event has **no `meetings` row at all** until recording starts
   (`api_create_meeting`, `api/api.rs:1109`; rows are minted only at record start / notes-only
   creation / Join & Record). So there is nowhere to show a brief, carry forward open action
   items, or jot "things I plan to cover."

2. **The home screen is reactive, not day-oriented.** `app/page.tsx` stacks a `<DayAgenda>`
   (upcoming only) above a "Recent recordings" list (`page.tsx:407-443`, fed by
   `api_get_meetings`). It answers "what did I just record" rather than "what does my day look
   like and what should I prep for." Recorded meetings immediately fall into the recent list,
   which stops being useful once "All meetings" (`/meetings`) exists.

This spec adds the *preparation* half of the product: a **pre-call prep brief** for recurring
meetings (auto-generated in advance so it's instant on open), a **prep-notes agenda** you write
*before* the call, and a **Today view** — an 8am–6pm timeline that is the new home, replacing the
Day Agenda + Recent recordings stack.

## Grounded current state (what already exists to build on)

- **Aggregation engine (0035)** — `frontend/src-tauri/src/aggregation/`. Designed explicitly so
  "pre-call prep becomes a new prompt set over this same engine with zero new gather/chunk code"
  (`aggregation/mod.rs:7-9`). `run()` (`engine.rs:103-115`) takes an `AggregationPrompt`
  (`engine.rs:31-41`) + an injected LLM closure; `AggregationScope` (`scope.rs:12-31`) supports a
  **pinned `meeting_ids` set** — `SearchRepository::rank_pinned_ids` (`search.rs:387-436`) selects
  nothing outside the set, and a term-less question degrades to a pure newest-first metadata fetch
  over that set. This is exactly the "summarize these N specific meetings" primitive we need.
- **Action items (0034, Done)** — `action_items` table (`action_item.rs:30-56`):
  `assignee_person_id`, `assignee_is_self`, `status` (`open`/`completed`/`dismissed`),
  `meeting_id`, `due_hint`. 0034 **explicitly deferred** recurring roll-up to pre-call prep
  (`0034:107-108`). No `get_open_for_meetings(&[ids])` exists yet.
- **Series linkage keys** — `meetings.calendar_event_id` (migration `20260626000001`, nullable,
  indexed, **deliberately not unique**). Critically, series membership is source-dependent:
  - **EventKit**: `calendar_event_id` (= `eventIdentifier`) is **shared across all occurrences** of
    a recurring series (`meeting.rs:90-96`), and `external_id` (= `calendarItemExternalIdentifier`)
    is likewise series-level (`eventkit.rs:46-51`).
  - **Google (0032)**: `calendar_event_id` (= `gcal:…/<instanceId>`) is **unique per occurrence**
    (`singleEvents=true`, `sync.rs:705-709`), but `external_id` (= iCalUID) **is** series-level and
    shared across occurrences (`sync.rs:294-309`, tests `sync.rs:1110/1145`).
  - ⇒ **`external_id` is a series-level key for both sources**, but it is currently *dropped* at
    adoption and never persisted onto the `meetings` row. Normalized-title match
    (`LOWER(TRIM(title))`, the 0020 `suggest_template_for_title` approach, `meeting.rs:453-477`) is
    the only source-agnostic grouping available today.
- **Day Agenda** — `api_get_day_agenda` (`calendar/day_agenda.rs:289`) → `DayAgendaItem`
  (`day_agenda.rs:80-111` / TS `lib/day-agenda.ts:34-56`): merges calendar events + recordings,
  with `AgendaStatus` flags and `AgendaSource`. `AgendaRow` already renders a "Later today"
  vertical timeline (`DayAgenda.tsx:512-560`) and has (currently filtered-out) past-phase code
  (`DayAgenda.tsx:564-597`). No true hour grid exists anywhere.
- **Notes** — `meeting_notes(meeting_id PK, notes_markdown, notes_json, …)` (migration
  `20251223000000`), one row per meeting, `notes_markdown` fed into the summary
  (`summary/service.rs:535-539`) and action-item extractor (`extractor.rs:334`). The dormant
  `enhanced_*` columns (migration `20260623000000`) are the precedent for adding columns here.
- **Background-job pattern** — `tauri::async_runtime::spawn` + a `Lazy<Mutex<HashMap<…>>>`
  registry. Cleanest reference: action-item extraction fired after summary persistence, "never
  delays or fails the summary" (`summary/service.rs:763-788`). Timer-spawn precedents:
  `spawn_retention_sweeper` (`lib.rs:693`) and `spawn_background_sync` (Google, `sync.rs:236`).
- **Cache-ledger precedent** — `action_item_extractions` (one row per meeting: `summary_fingerprint`,
  `extracted_at`, `model_provider`/`model_name`; `action_item.rs:71-81`, `get_extraction`/
  `replace_extracted`) — the model for a fingerprint-invalidated, pre-generated cached artifact.
- **Adoption** — `find_adoptable_calendar_meeting(pool, calendar_event_id)` (`meeting.rs:97-117`)
  reuses an "empty + updated within 6h" calendar-linked row at record start (`api.rs:1153-1172`),
  deliberately narrow so it can't fold a genuine prior occurrence.

## Goals

- **Pre-call brief for recurring meetings.** For an upcoming occurrence of a recurring series,
  show a short synthesized brief over the **previous ≤2 completed occurrences**: recurring
  themes, **decisions made** (emphasized — the core "what did we decide" need), and open
  threads/"where we left off." Generated via the 0035 engine over a pinned occurrence set with a
  new `pre_call_prep_prompt`, with meeting-granularity `[M#]` citations back to the source
  occurrences.
- **Carried-over open action items.** Alongside the brief, the still-open action items from the
  series' prior occurrences, split into **mine** (`assignee_is_self`) and **owed by others**
  (roster `assignee_person_id`), so "what's still outstanding" is answerable at a glance.
- **Instant on open.** Briefs are **pre-generated in the background** for today's and tomorrow's
  recurring meetings and cached, so opening a meeting's prep shows the brief immediately (no LLM
  wait). Regenerated when an input occurrence's summary changes or a newer occurrence completes.
- **Prep notes (agenda).** A dedicated "things I plan to cover" editor on an upcoming meeting,
  separate from during-meeting notes. It stays visible during the meeting as an agenda, and is
  handed to the post-meeting summary as **intended-agenda context** so the summary can reflect
  what was covered vs. skipped.
- **Today view.** Replace the home screen with an **8am–6pm timeline** of today's calendar events
  and recordings, with a "now" indicator and past/now/upcoming states. Clicking an upcoming
  occurrence opens its **Prep**; clicking a past meeting opens its details (to review or fill in).
  Remove the "Recent recordings" section (history lives in `/meetings`).
- **One continuous meeting object.** An upcoming occurrence you prep becomes the *same* meeting
  record when you record it — prep notes and brief carry through — via a `scheduled` origin that
  Join & Record adopts.

## Non-goals

- **Analytics / talk-time / "my week"** (ROADMAP "Meeting analytics v1") — a separate item.
- **Fuzzy/semantic series matching.** Series grouping is `external_id` (series key) → normalized
  exact-title fallback. Paraphrased/renamed recurring titles that share no series key won't group;
  accepted (same posture as 0020's title match).
- **Prep for one-off (non-recurring) meetings' briefs.** A meeting with no prior occurrence of the
  same series gets the prep-notes editor + roster, but no brief (nothing to synthesize).
- **Editing/curating the brief.** The brief is generated, cached, and re-generable; it is not a
  hand-editable document. Prep *notes* are the editable surface.
- **A clock-driven persistent join notification** (BACKLOG "Persistent meeting-start
  notification") — related and a natural companion, but its own small spec; referenced, not built
  here.
- **New egress destinations** — only the user-chosen summary/LLM provider, per the `/CLAUDE.md`
  privacy invariant and ADR-0010 (calendar metadata only).

## Approach

**The brief is a new `AggregationPrompt` over the 0035 engine, not a new pipeline.** The ROADMAP
framed pre-call prep v1 as "a join … not the full context engine," but the user's actual need —
synthesize *two* prior occurrences into topics/decisions/open-threads — is genuinely multi-meeting
synthesis, which is precisely what the engine exists for. We keep the *action-items* half a cheap
direct SQL join (no LLM), and we pay the engine's cost **in the background** (pre-generated +
cached), so on-open is instant and cost is bounded to today+tomorrow's recurring meetings. This is
the design 0035/0013 anticipated ("pre-call prep is a prompt set over the engine").

**Series identity is a persisted `calendar_series_key` (= `external_id`), with title fallback.**
Because `external_id` (EventKit `calendarItemExternalIdentifier` / Google iCalUID) is series-level
for *both* sources but is dropped today, we persist it onto the `meetings` row. Series detection is
then `WHERE calendar_series_key = ?` (calendar-linked) or `LOWER(TRIM(title))` (fallback for
manual/older rows). This avoids the EventKit "shared `calendar_event_id` across occurrences" trap
and the Google "unique-per-occurrence `calendar_event_id`" trap in one stroke.

**A `scheduled`-origin `meetings` row is the home for prep, and it becomes the recording.** When
prep is first needed for an occurrence (background pre-gen, or the user opening it), we mint a
`meetings` row with `origin='scheduled'`, `created_at` = the occurrence's scheduled start,
`calendar_event_id` = the event id, and `calendar_series_key` = its series key. Prep notes and the
cached brief attach to this row. At Join & Record, adoption reconciles the scheduled row **by
`(calendar_event_id, same occurrence day)`** — flipping it to `origin='recorded'`, keeping its id
so prep carries through. This gives the user one continuous object (upcoming → prepped → recorded)
matching the Today-view mental model, and disambiguates EventKit's shared series id by occurrence
day. An occurrence that's never recorded leaves a harmless scheduled row (optionally GC'd).

**The Today view is the new Home.** `app/page.tsx` becomes an hour-grid timeline (default 8a–6p,
auto-expanded to fit outliers) sourced from an extended `api_get_day_agenda` that now includes past
phases and a `prep` availability flag. Recent recordings is removed. Prep and details both live in
the existing **meeting-details route** via a new **"Prep" tab** — a scheduled meeting shows Prep
(+ My notes); a recorded meeting shows Prep + Summary + Transcript + My notes — so we reuse the
identity header, participants panel, and notes editor rather than building a parallel surface.

Alternative considered and rejected: a separate calendar-keyed prep store with no `meetings` row
until recording (copy prep-notes forward at record time). Cleaner isolation, but it fragments the
object the user thinks of as "the meeting," complicates the Today timeline (two id spaces), and the
copy-forward step is its own race. The `scheduled`-origin approach reuses adoption, notes, FTS, and
meeting-details with a bounded adoption change.

Background pre-generation runs for **all** recurring meetings today+tomorrow regardless of provider
(the user's decision — consistent with the 0035 posture that "provider choice in Settings is the
egress consent"; cloud briefs incur bounded background egress). A Settings toggle can disable
background prep generation (falls back to on-open generation) for users who want zero background
egress.

No new *outbound* destination, no new dependency. New tables/columns follow ADR-0004 (forward-only
migrations, per-identifier isolation); LLM egress uses the Keychain-stored key (ADR-0009) and the
configured summary provider (ADR-0010 keeps calendar read-only/metadata). → **No new ADR needed**;
the series-key + scheduled-origin decisions are recorded here.

## Design

### Data model

**Migrations** (`frontend/src-tauri/migrations/`, forward-only, ADR-0004):

1. **`20260708000000_add_meeting_series_key.sql`** — `ALTER TABLE meetings ADD COLUMN
   calendar_series_key TEXT;` (nullable) + partial index `WHERE calendar_series_key IS NOT NULL`.
   Populated at row creation from the calendar event's `external_id` (EventKit
   `calendarItemExternalIdentifier` / Google iCalUID). Series-level for both sources. Existing rows
   stay NULL and fall back to title match.

2. **`20260708000100_add_prep_notes.sql`** — `ALTER TABLE meeting_notes ADD COLUMN prep_markdown
   TEXT; ADD COLUMN prep_json TEXT; ADD COLUMN prep_updated_at TEXT;` (mirrors the dormant
   `enhanced_*` columns). Prep notes are stored **separately** from `notes_markdown` so they don't
   silently become live notes; the summary prompt reads them as a labeled "intended agenda" block.
   Extend the `meeting_notes_fts` trigger set (0033 FTS migration) to index `prep_markdown` too, so
   prep agendas are searchable in ⌘K.

3. **`20260708000200_create_meeting_briefs.sql`** — new table, modeled on
   `action_item_extractions`:
   ```sql
   CREATE TABLE meeting_briefs (
     meeting_id        TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
     status            TEXT NOT NULL,          -- 'pending' | 'ready' | 'failed' | 'none'
     brief_markdown    TEXT,                   -- synthesized brief ([M#] markers, post-processed)
     sources_json      TEXT,                   -- JSON array of SourceMeeting (the prior occurrences)
     source_fingerprint TEXT,                  -- hash(prior occurrence ids + each summary_fingerprint)
     model_provider    TEXT,
     model_name        TEXT,
     generated_at      TEXT,
     created_at        TEXT NOT NULL,
     updated_at        TEXT NOT NULL
   );
   ```
   One row per **scheduled/target** meeting. `status='none'` records "no prior occurrences → no
   brief" so we don't re-attempt. `source_fingerprint` is the idempotency + staleness guard: the
   background job regenerates only when it changes (a prior occurrence's summary changed, or a newer
   occurrence completed). No separate "seen" ledger is needed — the fingerprint is the guard.

**`meetings.origin` gains `'scheduled'`** — no schema change (free TEXT column, migration
`20260626000000`); the value is documented and added to the frontend `MeetingOrigin` union
(`types/index.ts:139-140`). Lifecycle: `scheduled` → (Join & Record adopts) → `recorded`; a
scheduled row that's recorded keeps its id, prep notes, and brief.

### Series detection + prior-occurrence query (Rust, `repositories/meeting.rs`)

New finder returning the pinned occurrence set for a brief:
```rust
/// Prior COMPLETED occurrences of the same recurring series, newest first, that have
/// content to summarize (a completed summary, else notes, else transcript). `series_key`
/// is the meeting's calendar_series_key when present; title is the normalized fallback.
pub async fn find_prior_series_occurrences(
    pool: &SqlitePool,
    series_key: Option<&str>,
    normalized_title: &str,
    before: &str,          // ISO-8601 UTC — the target occurrence's start
    limit: usize,          // 2
) -> anyhow::Result<Vec<String>>; // meeting ids
```
Match rule: `calendar_series_key = ?series_key` when `series_key` is `Some`, else
`LOWER(TRIM(title)) = ?normalized_title`; `AND created_at < ?before AND origin IN ('recorded')`
`AND EXISTS (completed summary OR transcript OR notes)`; `ORDER BY created_at DESC LIMIT ?`. Reuses
the `normalize_title` helper (`meeting.rs:553-555`).

Also add `find_by_series(series_key | normalized_title) -> Vec<MeetingListRow>` for the "did an
occurrence already get a scheduled row" de-dupe and `upsert_scheduled_meeting(...)` (mint-or-return
a `scheduled` row for `(calendar_event_id, occurrence_start_day)`).

### Action-item carryover (Rust, `repositories/action_item.rs`)

New query (there is no set-scoped open-items method today):
```rust
pub async fn get_open_for_meetings(pool, meeting_ids: &[String]) -> anyhow::Result<Vec<ActionItem>>;
// SELECT_ITEM projection WHERE meeting_id IN (…) AND status = 'open'
```
The Prep view groups the result into **mine** (`assignee_is_self`) and **owed by others** (grouped
by `assignee_person_id`, resolved to `people.display_name`).

### Brief generation (Rust, new `aggregation` consumer + `summary`-style background job)

- **Prompt** — `pre_call_prep_prompt() -> AggregationPrompt` in `aggregation/prompts.rs`, modeled on
  `ask_ai_prompt` (`prompts.rs:85-125`):
  - *map* (per prior occurrence): extract **decisions made**, topics discussed, and open
    commitments/unresolved threads — verbatim-biased, emit `NOTHING RELEVANT` when empty (reuse the
    `MAP_SYSTEM` discipline, `prompts.rs:38-52`). No `[M#]` in map output.
  - *reduce*: synthesize a **short** brief (a handful of bullets, glance-before-the-call length) with
    sections **Where we left off / Decisions / Open threads**, each claim citing `[M#]` to the
    occurrence it came from. Reuse `postprocess_citations` (`prompts.rs:224-293`).
- **Execution** — `execute_pre_call_prep(pool, target_meeting_id, prior_ids, llm, budget, cancel,
  on_progress)` mirroring `execute_ask_ai` (`commands.rs:107-137`): `gather` with
  `AggregationScope { meeting_ids: Some(prior_ids), .. }` and a **term-less** question (pure
  metadata fetch, newest-first), `pre_call_prep_prompt()`, `engine::run`, `postprocess_citations`.
  Reuses `resolve_context_budget` + `resolve_provider_config` + `generate_summary_with_retry`, and
  follows the configured summary provider — exactly the Ask-AI plumbing.
- **Background generator** — `aggregation/prep_jobs.rs`: `spawn_prep_generator(app)` wired at
  `lib.rs` startup like `spawn_retention_sweeper` (first pass ~1 min after launch, then every N
  min, plus a run on window focus). Each pass:
  1. Read today+tomorrow's upcoming occurrences (the Day-Agenda calendar path, both EventKit and
     Google active source).
  2. For each occurrence that is **recurring** (has ≥1 prior series occurrence with content):
     ensure a `scheduled` `meetings` row exists (`upsert_scheduled_meeting`), compute the
     `source_fingerprint`, and if `meeting_briefs` is missing/stale for it, generate + cache the
     brief (status `pending`→`ready`/`failed`). Occurrences with no prior content get
     `status='none'`.
  Single-flight guard (`Lazy<Mutex<…>>` like `SYNC_LOCK`); fire-and-forget, errors `warn!`-logged
  only — never blocks anything (the 0034 extraction contract). Honors a Settings
  `prep_autogenerate` toggle (default on).

### Prep-notes-aware summary (llm-pipeline)

When a `scheduled` meeting is recorded and summarized, load `meeting_notes.prep_markdown` and pass
it to the summary prompt as a labeled **"Intended agenda (what the user planned to cover)"** block,
distinct from the live notes block (`summary/service.rs:535-539`). Prompt guidance: note which
agenda items were covered vs. not discussed. Prep notes are *not* passed to the action-item
extractor (they're intentions, not commitments). Behavior for meetings with no prep notes is
unchanged (existing summary tests stay green).

### Adoption change (Rust)

Extend the record-start adoption path (`api.rs:1153-1172` / `find_adoptable_calendar_meeting`,
`meeting.rs:97-117`) so a **`scheduled`** row for the same `calendar_event_id` **whose `created_at`
falls on the same calendar day as the occurrence being recorded** is adopted regardless of the 6h
recency window: flip `origin` to `recorded`, keep the id (prep notes + brief stay attached). The
same-day constraint disambiguates EventKit's shared `calendar_event_id` across occurrences. The
existing "empty + recent" adoption for non-scheduled rows is unchanged.

### Tauri IPC (`frontend/src-tauri/src/lib.rs` registration)

Commands:
- `api_ensure_scheduled_meeting(calendarEventId, seriesKey, title, occurrenceStart) -> meetingId` —
  mint/return the `scheduled` row for an occurrence (called when the user opens an upcoming event's
  prep). Idempotent by `(calendar_event_id, occurrence day)`.
- `api_get_prep(meetingId) -> PrepView` — returns `{ brief: { status, markdown, sources }, myOpenItems,
  othersOpenItems, roster, prepNotesMarkdown }`. If the brief is missing/stale, kicks off async
  generation and returns `status:'pending'` (events below drive the fill-in). Pure-read when cached.
- `api_regenerate_prep_brief(meetingId) -> runId` — manual refresh (registry + cancel like Ask-AI).
- `api_save_prep_notes(meetingId, prepMarkdown, prepJson)` / `api_get_prep_notes(meetingId)` —
  autosave for the prep-notes editor (dedicated so prep never lands in `notes_markdown`).
- `api_get_today_timeline()` — extend/reuse `api_get_day_agenda` to include **past** phases and a
  `prep: { available: bool, status }` flag per item; the Today view consumes this.

Events (kebab-case, camelCase payloads, mirroring `ask-ai-*`):
- `prep-brief-progress` `{ meetingId, stage, current, total }`
- `prep-brief-complete` `{ meetingId, markdown, sources }`
- `prep-brief-error` `{ meetingId, message }`
- `prep-briefs-updated` (broadcast, no payload) — emitted when a background pass changes any brief,
  so the Today view / open Prep tab refresh.

### UI (`frontend/src/`)

- **Today view = new Home** (`app/page.tsx`): an **8am–6pm hour-grid timeline** (auto-expand to fit
  any event outside the window), items positioned by start/end, with:
  - a **"now" line** at the current time and past/now/upcoming visual states;
  - per-item status chips (upcoming · *Prep ready* / recorded · *Open* / *joinable* · Join & record);
  - click routing: upcoming → `api_ensure_scheduled_meeting` then `/meeting-details?id=…` on the
    **Prep** tab; past recorded → `/meeting-details` (existing); now/joinable → existing Join & Record.
  - **Recent recordings removed.** Earlier-today meetings remain visible in the grid (click to fill
    in details); older history is in `/meetings` ("All meetings"). Reuse time-format helpers
    (`lib/calendar.ts:431-448`) and the `AgendaRow` timeline styling (`DayAgenda.tsx:512-560`) as the
    visual starting point; the hour grid itself is net-new.
- **Prep tab** in meeting-details (`app/meeting-details/page-content.tsx`, tabs at `:260-271`): a new
  tab present for calendar-linked meetings. Contents: the **brief** (`[M#]` chips linking to source
  occurrences, reuse `AskAI/AnswerMarkdown.tsx`), **carried-over open action items** (mine / owed by
  others, reuse the action-items row components + `PersonFilterDropdown`), and the **prep-notes
  editor** (BlockNote, autosave via `api_save_prep_notes`, mirroring `NotepadPanel.tsx`). For an
  `origin='scheduled'` meeting, Prep is the default tab and Summary/Transcript are hidden; for a
  recorded meeting, Prep remains available (see what you prepped + a "prep notes" recap). A
  `status:'pending'` brief shows a spinner and fills in on `prep-brief-complete`.
- **During-meeting agenda**: on `/record`, surface the prep notes as a pinned read-only agenda
  panel beside the live notepad (small addition to the record screen), so the agenda stays in view.

## Tasks

1. [ ] **Series key + prior-occurrence detection** — migration `add_meeting_series_key`; populate
   `calendar_series_key` from `external_id` on every meeting-row creation path (record start,
   Join & Record, scheduled mint); `find_prior_series_occurrences` + `find_by_series` +
   `upsert_scheduled_meeting` + the `'scheduled'` origin; adoption change (same-day scheduled
   adoption). Unit tests in `repositories/meeting.rs`. *(rust-core-engineer)*
2. [ ] **Action-item carryover** — `get_open_for_meetings(&[ids])` in `action_item.rs`; grouping
   helper (mine vs. others, resolve people). Unit test. *(rust-core-engineer)*
3. [ ] **Brief prompt + engine consumer** — `pre_call_prep_prompt` in `aggregation/prompts.rs`
   (map: decisions/topics/open-threads extraction; reduce: short cited brief); `execute_pre_call_prep`
   mirroring `execute_ask_ai`; prompt-eval fixtures (two canned occurrence summaries → brief cites
   `[M1]`/`[M2]`, emphasizes decisions). *(llm-pipeline-engineer)*
4. [ ] **Brief cache + background generator** — migration `create_meeting_briefs`;
   `MeetingBriefsRepository` (get/upsert, fingerprint); `aggregation/prep_jobs.rs`
   `spawn_prep_generator` (today+tomorrow, recurring-only, fingerprint-gated, single-flight,
   fire-and-forget) wired in `lib.rs`; Settings `prep_autogenerate` toggle. *(rust-core-engineer)*
5. [ ] **Prep IPC** — `api_ensure_scheduled_meeting`, `api_get_prep`, `api_regenerate_prep_brief`
   (+ registry/cancel), `api_save_prep_notes`/`api_get_prep_notes`; `prep-brief-*` +
   `prep-briefs-updated` events; register in `lib.rs`. Integration tests
   `tests/pre_call_prep.rs` (fixtures: two prior occurrences → brief cites both via fake LLM; no
   prior → `status:'none'`; stale fingerprint regenerates; carryover open items grouped).
   *(rust-core-engineer + llm-pipeline-engineer)*
6. [ ] **Prep notes → summary** — migration `add_prep_notes`; `prep_markdown`/`prep_json` storage;
   extend `meeting_notes_fts` triggers to index prep; pass prep as "intended agenda" to the summary
   prompt (`summary/service.rs`); confirm the extractor ignores prep. Test: a summary with prep
   notes labels covered-vs-skipped; summary without prep notes unchanged. *(llm-pipeline-engineer)*
7. [ ] **Today view** — rewrite `app/page.tsx` as the 8am–6pm timeline (hour grid, now-line,
   past/now/upcoming states, click routing); remove Recent recordings; extend `api_get_day_agenda`
   for past phases + `prep` flag (`day_agenda.rs`, `lib/day-agenda.ts`). Vitest for time→row layout
   and click routing. *(frontend-engineer)*
8. [ ] **Prep tab + agenda** — new Prep tab in `meeting-details/page-content.tsx` (brief via
   `AnswerMarkdown`, carried action items, prep-notes BlockNote autosave); scheduled vs. recorded
   tab visibility; pinned prep-agenda panel on `/record`. Vitest for the tab states + pending→ready
   fill-in. *(frontend-engineer)*
9. [ ] **Docs & gate** — CHANGELOG entry; `specs/INDEX.md` (0036 row) + ROADMAP status flip;
   `/check` (full Definition of Done) + the manual smoke below. *(any owner)*

## Acceptance criteria

Tied to the Definition of Done in `/CLAUDE.md` (cargo check/clippy/test + pnpm lint/test clean; app
launches; record → transcript → summary smoke intact — the summary path must be unaffected for
meetings without prep notes):

- Opening an upcoming occurrence of a recurring series shows a brief synthesizing its **previous
  ≤2 completed occurrences** — themes, decisions, open threads — each claim citing a source
  occurrence whose `[M#]` chip opens that meeting (automated with a fake LLM; manual with a real
  provider).
- The brief is **pre-generated**: with a fixture set of today's recurring meetings, the background
  generator populates `meeting_briefs` (status `ready`) without any user action, and re-opening is
  instant; changing a prior occurrence's summary flips the fingerprint and regenerates on the next
  pass; an occurrence with no prior content records `status='none'` and never spins.
- Carried-over **open** action items appear, correctly split into mine vs. owed-by-others, scoped to
  the series' prior occurrences (`get_open_for_meetings`); completed/dismissed items are excluded.
- **Prep notes** save to a store separate from live notes, are searchable in ⌘K, survive into the
  recording as an agenda, and reach the summary as intended-agenda context (a summary over a
  meeting with prep notes reflects covered-vs-skipped; a meeting without prep notes summarizes
  identically to today).
- A prepped upcoming occurrence, when recorded via Join & Record, becomes the **same** meeting
  record (same id) — its prep notes and brief remain attached; no duplicate row; the correct
  occurrence is adopted even for an EventKit series that shares one `calendar_event_id`.
- The **Today view** replaces Home: an 8am–6pm timeline with a now-line shows today's calendar
  events and recordings; upcoming items route to Prep, past recorded items to details; the "Recent
  recordings" section is gone and history remains reachable via "All meetings".
- Series detection works across sources: EventKit occurrences group by shared `external_id`; Google
  occurrences (unique `calendar_event_id`) group by shared iCalUID; manual/older rows fall back to
  normalized title.
- Background generation honors the Settings toggle; with it off, briefs generate on-open only. No
  new outbound destination beyond the configured LLM provider + (opt-in) calendar.

## Risks / open questions

- **EventKit shared `calendar_event_id` across occurrences** is the central correctness hazard.
  Mitigated by keying occurrence identity on `(calendar_event_id, occurrence day)` and grouping by
  the series-level `external_id`; both are covered by the adoption + series tests. If a series has
  two occurrences on the same calendar day (rare for the target cadence), same-day adoption could
  ambiguate — accepted for v1, noted.
- **Series key absent on older/manual meetings** → title fallback only; a renamed recurring meeting
  won't group. Accepted (0020 posture). Backfilling `calendar_series_key` for existing
  calendar-linked rows is possible later but out of scope.
- **Background cloud egress + cost** (the user chose "always pre-generate"): bounded to
  today+tomorrow's recurring meetings, fingerprint-gated so each brief generates once per input
  change; the Settings toggle is the escape hatch. Summaries-first sourcing (0035) keeps tokens
  small. No per-call cost display (no price tables in-app).
- **Brief quality on small local models** — multi-occurrence synthesis with citations is near the
  ceiling for small Ollama models; the map (extraction) stage carries the load, same as Ask-AI. The
  prompt-eval fixtures (task 3) are the guardrail.
- **Scheduled-row clutter** — occurrences prepped but never recorded leave `scheduled` rows visible
  in "All meetings." Option: a GC sweep for empty `scheduled` rows past their date (fold into the
  retention sweeper). Deferred; note in the spec, decide during build if it's noisy.
- **`api_get_day_agenda` extension vs. new command** — extending risks touching the live Day-Agenda
  consumers; if the past-phase/prep-flag change proves invasive, ship `api_get_today_timeline` as a
  sibling instead. Decide in task 7.
- **Prep tab on a recorded meeting** — showing a stale pre-meeting brief post-hoc could confuse;
  label it "Prep (before this meeting)" and keep it secondary to Summary. UX detail for task 8.

## Verification

Automated (all green via `/check`):

```bash
cd frontend/src-tauri && source ~/.cargo/env && \
  cargo check && cargo clippy --features metal --all-targets -- -D warnings && \
  cargo test --features metal --lib \
    --test pre_call_prep --test aggregation_engine --test db_lifecycle
cd frontend && pnpm lint && pnpm test
```

- `tests/pre_call_prep.rs`: prior-occurrence detection (series key + title fallback), the brief over
  a pinned set with a fake LLM (cites both occurrences, decisions-emphasized), `status='none'` for
  no-prior, fingerprint staleness → regenerate, `get_open_for_meetings` grouping, and the same-day
  scheduled adoption.
- Vitest: Today-view time→row layout + click routing; Prep-tab pending→ready fill-in; prep-notes
  autosave.

Manual smoke (dev build, `./dev-vinyl.sh`, dogfood DB):

1. Have two prior completed occurrences of a recurring meeting (same calendar series or same title),
   each summarized. Open the app → the Today timeline shows today's occurrence of that series as
   *upcoming · Prep ready* (after a background pass).
2. Click it → Prep tab: a short brief citing both prior occurrences (decisions emphasized, `[M#]`
   chips open the right meeting), the carried-over open action items split mine/others, the roster.
3. Write prep notes ("cover: roadmap slip, hiring backfill, Q3 OKRs"); they autosave and appear in
   ⌘K search.
4. Join & Record from the timeline → it adopts the same meeting (same id); the prep agenda is pinned
   beside the live notepad. Record a short session, stop, summarize → the summary reflects the
   intended agenda (covered vs. skipped).
5. Turn off Wi-Fi with Ollama configured → briefs still generate on-device for a recurring meeting.
6. Regression: a one-off (non-recurring) upcoming meeting shows the prep-notes editor + roster but
   no brief; a normal record → transcript → summary for a meeting with no prep notes is unchanged.
