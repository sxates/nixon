# 0003 — Note-enhancement pipeline (flagship)

- **Status:** ✅ DONE 2026-06-24 (basic smoke passed; deeper testing with real meetings to follow).
  Pivoted mid-build to a **notes-aware summary** (see "## Revision" below) — the separate "enhanced
  notes" artifact + `/notes/[id]` page + auto-enhance trigger were retired; manual notes now feed
  the meeting summary. Verified live: notes persist through stop, summary auto-generates on stop and
  via manual re-generate, notes are loaded server-side and injected into the summary, all on local
  Ollama (`gemma4:26b`). Commits `77cc5e3` (pivot) + `92a7f75` (dev webview chunk fix). Possible
  follow-up: tune the notes-grounding prompt strength after real-meeting use.
- **Owner agent(s):** frontend-engineer + rust-core-engineer + llm-pipeline-engineer
- **Roadmap phase:** Phase 2

## Context / Problem
Granola's defining feature: you jot rough notes during a meeting, and the AI **enhances your
notes** using the transcript — your notes are the backbone, the transcript fills in detail,
decisions, and action items you didn't capture. Meetily only does transcript → template
report and never touches user notes.

Grounding (verified):
- `meeting_notes` table exists (migration `20251223000000_add_meeting_notes.sql`: `meeting_id`
  PK, `notes_markdown`, `notes_json`, `created_at`, `updated_at`) but has **no Tauri commands
  and no real UI** — the `frontend/src/app/notes/[id]/page.tsx` is a static placeholder with
  hardcoded sample data. So notes persistence + UI are greenfield.
- Summary flow: `api_process_transcript` (`summary/commands.rs:326`) → `process_transcript_background`
  (`summary/service.rs:294`) → `generate_meeting_summary` (`summary/processor.rs:323`) →
  `generate_summary` (`summary/llm_client.rs:113`). `custom_prompt` is injected at
  `processor.rs:485-493`. There is **no mode/flag** distinguishing "summarize" vs "enhance".
- Transcript text is concatenated on the **frontend** from paginated segments
  (`usePaginatedTranscripts` / `api_get_meeting_transcripts`) and passed as `text`.
- Editor: `components/BlockNoteEditor/Editor.tsx` (reusable, `editable` prop),
  `lib/blocknote-markdown.ts` (`blocksToMarkdownSafely`), `components/AISummary/BlockNoteSummaryView.tsx`
  (markdown↔blocks, format detection) — reusable for both a live notepad and rendering output.

## Goals
- A **live notepad** during recording, beside the live transcript; autosaved to `meeting_notes`.
- An **"Enhance notes"** action producing enriched notes = *(user notes + transcript)*, where
  the user's structure/intent is preserved and the transcript adds grounded detail.
- The user's **original notes remain intact and recoverable** (never overwritten in place).
- Works with a **local Ollama model by default**; long transcripts handled via chunking.

## Non-goals
- Replacing the existing template-report summary (keep both; enhancement is a separate path).
- Real-time enhancement *during* the meeting (v1 enhances on demand after stop).
- Templated enhancement (v1 follows the user's own note structure, not a template).
- Speaker attribution in notes (depends on `specs/0004`/diarization).

## Approach
Add an **independent "enhance" path** that reuses the existing LLM plumbing
(`generate_summary`, chunking, provider/Ollama handling) but: (a) reads the user's notes +
the meeting transcript, (b) uses a distinct enhance prompt, and (c) writes to **new
`enhanced_*` columns** on `meeting_notes` so the original notes are preserved. The notepad and
result rendering reuse the existing BlockNote components.

Rationale for a separate command over a `mode` flag on `api_process_transcript`: that command
is transcript-only and stores into `summary_processes`; the enhance path has different inputs
(notes), different storage (`meeting_notes`), and different prompt — a dedicated command keeps
both flows clean and avoids regressing the report path. (Alternative — a `mode` param —
briefly considered; rejected for coupling two different storage/IO shapes.)

## Design

### Data model
New migration `frontend/src-tauri/migrations/2026MMDD000000_add_enhanced_notes.sql` adds to
`meeting_notes`:
- `enhanced_markdown TEXT`, `enhanced_json TEXT` (BlockNote blocks), `enhanced_at TEXT`,
  `enhanced_model TEXT`.
Original `notes_markdown`/`notes_json` stay as the user's raw notes (source of truth, always
recoverable). One row per meeting (upsert). *Alternative considered:* a revisions table —
deferred; not needed for v1.

### Tauri IPC (register in `frontend/src-tauri/src/lib.rs`)
- `api_save_meeting_notes(meeting_id, notes_markdown, notes_json)` → upsert raw notes
  (debounced autosave from the notepad). New repository methods in
  `database/repositories/` (new `meeting_note.rs` mirroring existing repos).
- `api_get_meeting_notes(meeting_id)` → `{ notes_markdown, notes_json, enhanced_markdown,
  enhanced_json, enhanced_at, enhanced_model }`.
- `api_enhance_notes(meeting_id, model, model_name, ollama_endpoint?, summary_language?)` →
  background task (mirror `process_transcript_background`): load raw notes + load transcript
  (server-side concat via a transcript repository helper, so enhancement doesn't depend on the
  frontend having all pages), build the enhance prompt, call `generate_summary`, write
  `enhanced_*`. Emit progress/completion via Tauri events (follow the existing summary
  status/event pattern) so the UI can show progress and stream if feasible.

### Prompt design (llm-pipeline-engineer)
New builder in `summary/processor.rs` (e.g. `generate_enhanced_notes`) parallel to
`generate_meeting_summary`, reusing `chunk_text` and `generate_summary`:
- **System:** "You enhance the user's own meeting notes using the transcript. Their notes are
  the source of truth for structure, intent, and ordering — preserve their outline, headings,
  and voice. Use the transcript ONLY to add detail, fill gaps, correct/expand points, and
  surface decisions and action items they didn't write down. Do **not** invent anything not
  supported by the notes or transcript; if unsure, omit. Output Markdown that follows the
  user's note structure."
- **User message:** `<user_notes>…</user_notes>` + `<transcript>…</transcript>`.
- **Long transcripts (local models):** map-reduce on the **transcript** (notes always fit) —
  per-chunk extract salient facts/decisions/actions, combine, then a single enhance pass with
  `<user_notes>` + `<combined_facts>`. Keep notes whole across passes.
- **Anti-hallucination:** instruction above + keep temperature low; never fabricate
  attendees/dates/decisions. Add a couple of transcript+notes fixtures for eyeball/eval checks.

### UI (frontend-engineer)
- **Recording screen** (`frontend/src/app/page.tsx`): split layout — a `BlockNoteEditor`
  notepad pane (`editable`) beside the live transcript. Debounced autosave via
  `api_save_meeting_notes`; current meeting from `components/Sidebar/SidebarProvider.tsx`.
- **Notes/meeting-details:** replace the static `app/notes/[id]/page.tsx` with a real view
  loading via `api_get_meeting_notes`; an **"Enhance notes"** button invoking `api_enhance_notes`
  with progress; render the result with `BlockNoteSummaryView`; a toggle between **My notes**
  (original) and **Enhanced**. New hook `useNoteEnhancement` parallel to
  `hooks/meeting-details/useSummaryGeneration.ts`.

## Tasks (ordered)
1. [x] **rust-core-engineer** — migration `add_enhanced_notes`; `meeting_note.rs` repository
   (upsert/get/update_enhanced); `api_save_meeting_notes` + `api_get_meeting_notes`; registered
   in `lib.rs`. DONE (commit `0101dee`, branch `feature/0003-note-enhancement`, cargo check clean).
2. [x] **rust-core-engineer** — server-side transcript concatenation helper in
   `database/repositories/transcript.rs` (`get_full_transcript`, ordered nulls-last). DONE (commit `c1cdd95`).
3. [x] **llm-pipeline-engineer** — `generate_enhanced_notes` in `processor.rs` (enhance prompt
   + map-reduce on transcript, temp 0.1), reusing `generate_summary`; +5 unit tests + eval
   fixtures under `tests/fixtures/note_enhancement/`. DONE (commit `c1cdd95`).
4. [x] **rust-core-engineer** — `api_enhance_notes` + `enhance_notes_background` (mirror
   `process_transcript_background`); reuses summary model settings; writes `enhanced_*`; emits
   `note-enhancement-{progress,complete,error}`; skips when no notes/transcript; registered in
   `lib.rs`. DONE (commit `c1cdd95`).
5. [x] **frontend-engineer** — live notepad pane (`app/_components/NotepadPanel.tsx`) on the
   recording screen + debounced (1s) autosave via `api_save_meeting_notes`; load on mount.
   DONE. NOTE: the DB meeting row only exists after stop+save (during recording the id is the
   `'intro-call'` placeholder), so in-recording notes are held in memory and flushed on the
   placeholder→real id transition.
6. [x] **frontend-engineer** — `useNoteEnhancement` hook; **auto-enhance trigger** in
   `useRecordingStop.ts` gated on `transcriptionComplete` (transcript finalization, not the stop
   event), skips when no notes; rewrote `app/notes/[id]/page.tsx` + `NotesPageContent.tsx` with
   My-notes/Enhanced toggle + manual re-enhance; renders via `BlockNoteSummaryView`. DONE.
7. [~] **(all)** — Rust gate GREEN (`cargo check` + `clippy` clean, enhance unit tests pass);
   frontend `tsc --noEmit` clean. **`pnpm lint` cannot run** — eslint/`eslint-config-next` were
   never added to `frontend/package.json` (pre-existing upstream gap; `next lint` drops to an
   interactive setup). Manual record→enhance smoke (spec Verification) still TODO by the owner.

## Revision 2026-06-24 (pivot to notes-aware summary)

First smoke (2026-06-24) surfaced two things: (1) notes typed during recording were **lost** —
during recording the meeting has no DB row (placeholder id `intro-call`), and on stop
`useRecordingStop` navigates to `/meeting-details` and unmounts the notepad before it can flush;
(2) the post-meeting destination is `/meeting-details` (TranscriptPanel + SummaryPanel), which
never showed the notes — the Task-6 `/notes/[id]` page was orphaned.

User decisions (2026-06-24):
- **Notes-aware summary, one artifact.** The user's manual notes become an **input to the
  meeting summary** (transcript + notes → summary). The separate "enhanced notes" artifact
  (`enhanced_*`, `generate_enhanced_notes`, `api_enhance_notes`, `note-enhancement-*` events)
  and the standalone `/notes/[id]` page are **retired**.
- **Auto on stop + manual re-generate.** This is already the existing summary behavior
  (`shouldAutoGenerate` → `handleGenerateSummary`; manual via the summary button groups). No new
  trigger needed — just make generation notes-aware and ensure notes are saved first.
- **Post-meeting layout:** summary as the main pane; **right sidebar with tabs**: `Transcript`
  and `My Notes` (editable, autosaved). Editing notes + Re-generate refreshes the summary.

New design:
- **Backend (rust-core):** the summary path loads the meeting's notes from `meeting_notes`
  server-side (`MeetingNotesRepository::get_notes`) and injects them into the summary prompt as
  high-priority grounding (preserve template structure; notes are authoritative for what
  mattered; transcript fills detail; anti-hallucination language reused from the retired
  enhance prompt). No new IPC args — notes are picked up from the DB at generation time. Remove
  `api_enhance_notes`/`enhance_notes_background`/`generate_enhanced_notes`; `enhanced_*` columns
  left dormant (forward-only migrations; documented).
- **Frontend (frontend-engineer):**
  - **Persistence fix:** lift the in-progress notepad content into a shared store (context) so
    `useRecordingStop` can `await api_save_meeting_notes(realId, …)` **before** `router.push` —
    so notes are in the DB before the auto-generate runs.
  - **`/meeting-details` layout:** SummaryPanel main + right sidebar `Tabs` (`Transcript`,
    `My Notes`). My Notes uses the BlockNote editor (editable), debounced autosave via
    `api_save_meeting_notes`, loads via `api_get_meeting_notes`.
  - Keep the recording-screen notepad (it feeds the draft store).
  - **Retire:** `/notes/[id]` enhanced view, `useNoteEnhancement`, the auto-enhance trigger in
    `useRecordingStop`, and the `note-enhancement-*` listeners.

Retained from the original build: `meeting_notes` table + `notes_*` columns,
`api_save_meeting_notes`/`api_get_meeting_notes`, `MeetingNotesRepository` (get/upsert),
`get_full_transcript`, the recording-screen `NotepadPanel`, and the enhance prompt's
anti-hallucination wording (folded into the summary prompt).

## Acceptance criteria
- During a recording, typed notes persist to `meeting_notes` and survive app restart.
- "Enhance notes" produces notes that **follow the user's outline** while incorporating
  transcript-grounded detail (decisions/action items the user didn't type).
- The **original notes remain available** (My-notes/Enhanced toggle); enhancement never
  overwrites `notes_markdown`/`notes_json`.
- Works end-to-end with a local Ollama model (e.g. `gemma4:26b`); long transcripts don't error.
- `cargo check` + `clippy` + `pnpm lint` clean (Definition of Done in `/CLAUDE.md`).

## Decisions (resolved 2026-06-23)
- **Trigger: auto-enhance on recording stop.** When a recording stops *and* its transcript has
  finalized, the enhance flow runs automatically (no manual button as the primary path). Keep a
  "re-enhance" affordance for manual re-runs. **Sequencing note:** transcription can still be
  draining when recording stops, so the trigger must wait for transcript finalization before
  calling `api_enhance_notes` (gate on the transcript-complete signal, not the stop event);
  skip auto-enhance when the meeting has no user notes.
- **Model: reuse the summary model settings** (provider/model already configured for summaries,
  e.g. Ollama/`gemma4:26b`). No separate picker in v1.

## Risks / open questions
- **Quality risk:** small local models may drift from the user's structure or hallucinate on
  long transcripts — mitigated by the map-reduce + low temperature + fixtures; revisit prompt
  after first real use.
- **Dependency:** transcript *coverage* gaps (`specs/0004`) directly degrade enhancement
  quality — worth fixing soon after, though enhancement can be built/tested in parallel.

## Verification
Record a short meeting while typing ~5 bullet notes; stop; "Enhance notes" with local Ollama;
confirm the enriched notes keep the user's bullets/outline and add transcript-grounded detail
(a decision + an action item the user didn't write). Toggle back to original notes. Re-open the
meeting after restart to confirm persistence. Run `/check`.
