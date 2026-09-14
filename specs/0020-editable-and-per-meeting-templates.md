# 0020 — Editable & per-meeting summary templates

- **Status:** ✅ Implemented (2026-07-02) — re-grounded the same day per the 2026-07-01
  roadmap review §3; originally drafted post-1.0. Note: `api_get_template_details` kept its
  `sections` (titles) field and gained `section_details` (full objects) instead of changing
  the field shape; the manual record → summarize smoke pass is tracked in ROADMAP
  "Awaiting verification".
- **Owner agent(s):** llm-pipeline-engineer + rust-core-engineer + frontend-engineer
- **Roadmap phase:** Next #1 (post-1.3.1)

## Context / Problem

From 1.0 testing (note 10 in `specs/0019`), re-asked in the 1.2 round: the user cannot tell
which summary template a meeting used, the template prompts aren't editable in-app, there's
no way to add a template, and nothing auto-selects a sensible template for recurring
meetings.

**What 0029 WS4.3 already shipped (struck from this spec's scope):**
- `meetings.template_id` (migration `20260703000001_add_meeting_template.sql`; NULL = default).
- Repo read/write: `get_meeting_template` / `set_meeting_template`
  (`database/repositories/meeting.rs:397-441`, with tests).
- Commands `api_get_meeting_template` / `api_set_meeting_template`
  (`summary/template_commands.rs:136-200`), registered in `lib.rs:845-849`.
- Frontend per-meeting persistence: `hooks/meeting-details/useTemplates.ts` loads/saves the
  choice (queue-before-row-id, late-load-clobber guard; tested in
  `hooks/__tests__/useTemplates.test.ts`) and the record screen has a live picker
  (`app/record/page.tsx:497-526`).

**Current state (re-grounded 2026-07-02):**
- Templates are JSON: 6 files in `frontend/src-tauri/templates/` (`daily_standup`,
  `standard_meeting`, `project_sync`, `retrospective`, `sales_marketing_client_call`,
  `psychatric_session` — note the misspelled filename). Only the first two are embedded
  in the registry (`summary/templates/defaults.rs:7-21`); the other four exist solely as
  bundled resources found by the runtime dir scan (`loader.rs:144-200`), so they work in a
  packaged app but are second-class (no embedded fallback, not in
  `list_builtin_template_ids`).
- Schema `Template { name, description, sections[] }`, section `{ title, instruction,
  format, item_format?, example_item_format? }` with validation (`types.rs:5-71`;
  `format ∈ {paragraph, list, string}`).
- Resolution order: custom dir (`app_data_dir/templates`, `loader.rs:24-26`) → bundled dir →
  embedded built-ins (`loader.rs:97-120`).
- Commands today: `api_list_templates`, `api_get_template_details`,
  `api_validate_template` (+ the 0029 pair). **No save/write command** — editing/creating
  templates is unsupported.
- **Template consumption is frontend-param-only.** `api_process_transcript` takes
  `template_id` and defaults to `"daily_standup"` when absent (`summary/commands.rs:356`);
  `api_generate_summary_for_meeting` (Day-Agenda one-click + auto-summarize) **hardcodes**
  `"daily_standup"` (`commands.rs:520`) and never reads `meetings.template_id`. The
  frontend default is `standard_meeting` (`useTemplates.ts:8`) — so the two halves disagree,
  and the persisted per-meeting choice is silently ignored on every non-manual path.
- UI: the meeting-details "Template" dropdown trigger is a static label
  (`SummaryGeneratorButtonGroup.tsx:322-351`); only the open menu shows a checkmark. No
  settings surface for templates (settings page tabs: General / Recordings / Transcription /
  Summary / Beta — `app/settings/page.tsx:15-21`).
- No auto-selection; no shared title normalizer (ad-hoc `trim().to_lowercase()` in
  `calendar/day_agenda.rs:171-194` and `calendar/eventkit.rs:473-479`).

## Goals

- The persisted per-meeting template is honored by **every** generation path (manual,
  Day-Agenda one-click, auto-summarize) with one shared default.
- Show the active template in the UI outside the dropdown (label = selected name).
- Changing the template on a meeting that already has a summary offers to regenerate —
  **confirm-before-regenerate** (decided per roadmap review §3: summaries cost minutes on
  local Ollama; never fire silently).
- Template prompts **editable in-app**; **add** (and delete custom) templates from settings.
- The four orphaned bundled templates become first-class registered templates.
- **Auto-select** a template for a new meeting from the most recent prior meeting with the
  same normalized title.

## Non-goals

- Exposing/editing the hardcoded synthesis scaffolding prose (`processor.rs:356-396` renders
  the template into the prompt; the surrounding prose stays fixed); only the JSON
  per-section fields are user-editable.
- A template marketplace or sharing/export beyond local files.
- Live-during-meeting template *content* changes affecting an in-flight summary.

## Design

### Backend (rust-core-engineer)

1. **Register the four orphaned templates** — embed via `include_str!` in `defaults.rs`
   alongside the existing two, so all six are built-ins with embedded fallback (rename the
   `psychatric_session` **display name** properly; keep the file/id stable for
   back-compat).
2. **One default everywhere: `standard_meeting`.** Replace both `daily_standup` fallbacks
   (`commands.rs:356`, `:520`) with a single `DEFAULT_TEMPLATE_ID` const in
   `summary/templates/mod.rs`; frontend keeps `standard_meeting` (already matches).
3. **Backend resolves the persisted template.** In `api_process_transcript`: explicit
   `template_id` param wins (caller intent), else read `meetings.template_id`, else
   default. In `api_generate_summary_for_meeting`: read `meetings.template_id`, else
   default. This makes the 0029 persistence actually load-bearing on all paths.
4. **`api_save_template { id?, template }`** in `template_commands.rs`: validate via
   `Template::validate()`, slugify the id from `name` when absent (lowercase,
   `[a-z0-9_]`), write JSON to the custom dir (`loader.rs` custom dir; create if missing),
   return `{id, template}`. Saving with an id that matches a built-in creates a **custom
   override** (existing loader precedence already prefers custom — no new mechanism).
5. **`api_delete_template(id)`**: deletes the custom-dir JSON only. Deleting an override
   reverts to the built-in; deleting a built-in id with no override is an error. Also
   clear `meetings.template_id` references? **No** — dangling ids already fall back to
   default at resolution time (get_template errors → default); document that.
6. **`api_suggest_template_for_title(title)`**: new repo query — most recent meeting
   (by `created_at`) with `LOWER(TRIM(title)) = LOWER(TRIM(?))`, a non-NULL
   `template_id`, and a different meeting id; returns `Option<String>`. Add a shared
   `normalize_title` helper used by this query (leave the two calendar call sites alone —
   refactoring them is out of scope).

### Frontend (frontend-engineer)

7. **Active-template label**: the dropdown trigger in `SummaryGeneratorButtonGroup.tsx`
   shows the selected template's *name* (falling back to "Template" while loading). The
   record-screen picker (`record/page.tsx:497-526`) already shows the name — unchanged.
8. **Confirm-before-regenerate**: in meeting details, when the user picks a different
   template AND a summary already exists, open a confirm dialog (same controlled-`Dialog`
   pattern as `DeleteMeetingDialog.tsx:34-86`): "Regenerate the summary with
   <name>? This replaces the current summary." Confirm → persist choice + call
   `handleRegenerateSummary` (`useSummaryGeneration.ts:798-817`). Cancel → **revert the
   selection** (the label must keep matching the summary that's actually shown). With no
   existing summary, persist silently (today's behavior). Record-screen picker: never a
   dialog (no summary exists during recording).
9. **Templates settings tab**: add a "Templates" tab to `app/settings/page.tsx` (TABS +
   TabsContent). List all templates (`api_list_templates`) with built-in/custom/override
   badges; edit opens the section editor (name, description, ordered sections with
   title/instruction/format/item_format/example_item_format; `format` as a select);
   validation errors from `api_validate_template`/save surfaced inline; "New template"
   starts from a copy of `standard_meeting`; delete only for custom/override entries
   (confirm dialog, revert semantics explained in the dialog body for overrides).
10. **Auto-select-by-title**: in `useTemplates`, when the meeting has **no persisted**
    `template_id`, call `api_suggest_template_for_title` with the meeting title; a hit
    becomes the *displayed* selection and is persisted immediately (it's a default the
    user can change; persisting keeps every generation path consistent). The
    `userSelectedRef` guard already prevents clobbering an explicit user choice.

## Tasks

1. ~~Migration: `meetings.template_id`; repo read/write~~ — **shipped in 0029 WS4.3**.
2. [x] Register the 4 orphaned templates as built-ins; fix `psychatric_session` display
   name (rust-core).
3. ~~Persist template per meeting; load in `useTemplates.ts`~~ — **shipped in 0029 WS4.3**.
4. [x] Single `DEFAULT_TEMPLATE_ID = standard_meeting`; backend resolves persisted
   template in both generation commands (rust-core).
5. [x] `api_save_template` + `api_delete_template` + register (rust-core).
6. [x] `api_suggest_template_for_title` + `normalize_title` helper + repo query + tests
   (rust-core).
7. [x] Active-template label on the dropdown trigger (frontend).
8. [x] Confirm-before-regenerate dialog + revert-on-cancel wiring (frontend).
9. [x] Templates settings tab: list/edit/add/delete with validation (frontend).
10. [x] Auto-select default in `useTemplates` via suggest command (frontend).

## Acceptance criteria

- Day-Agenda one-click and auto-summarize use the meeting's persisted template; with none
  set, every path uses `standard_meeting`.
- The meeting-details UI shows the active template's name at a glance; reopening a meeting
  shows the template it actually used, surviving restart.
- Changing the template on a summarized meeting asks before regenerating; cancel leaves
  both the summary and the shown selection unchanged.
- A user can edit any template's sections, add a new one, and delete a custom one from
  Settings → Templates; invalid templates are rejected inline with the validator's message;
  an edited built-in behaves as an override and can be reverted by deleting it.
- A new meeting titled like a prior templated meeting defaults to that template.
- DoD per `/CLAUDE.md` (cargo fmt/clippy/test, `pnpm lint`/`test`, app launches, summary
  smoke path).

## Risks / open questions (decided)

- ~~How aggressively should regenerate-on-change fire~~ → **confirm-before-regenerate**
  (roadmap review §3).
- Title normalization for auto-select → v1 is `LOWER(TRIM(title))` exact match; recurring
  calendar events repeat titles verbatim, which is the target case. Fuzzier matching is
  future work.
- Editing a built-in creates a custom override of the same id → **yes**, precedence already
  exists in the loader; settings UI labels it "edited" with revert-via-delete.
- Dangling `meetings.template_id` after a custom template is deleted → falls back to
  default at generation time; no cleanup pass.

## Verification

`cargo test` template loader/validation/repo fixtures (extend for save/delete/suggest);
vitest for `useTemplates` auto-select + confirm-flow logic; manual: set template → one-click
summarize from Day Agenda uses it → edit template in settings → regenerate uses the edit →
create a same-title meeting → confirm auto-selected default.
