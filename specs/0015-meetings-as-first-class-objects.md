# 0015 — Meetings as first-class objects (2.0 Cluster 1)

- **Status:** Draft (design for review)
- **Owner agent(s):** rust-core-engineer (lead: schema/IPC/delete cascade) + frontend-engineer
  (entry points + meeting-details/notes UI) + llm-pipeline-engineer (notes-only summary degrade)
- **Roadmap phase:** Phase 2 — first buildable cluster of the **2.0 program** (`specs/0013`)
- **Parent program:** `specs/0013-identity-and-organization-program.md`. This is the
  **no-identity, no-ADR** cluster — the "standalone content + meeting-object" primitives 0013 calls
  for (`meetings.origin`, `meetings.calendar_event_id`) that do **not** depend on the
  voiceprint/People keystone (that's Cluster 2 = 0013 Wave 1: `specs/0011` + `specs/0012`, later).

## Context / Problem

A `meetings` row today is implicitly *"a recording that happened"* — created only at recording start
(`useRecordingStart.ts::createMeetingForRecording`), titled with a timestamp
(`generateMeetingTitle` → `Meeting DD_MM_YY_HH_MM_SS`), and never linkable, never typed, never
deletable from disk. Three concrete gaps fall out of that, all of which 0013 sequenced into the
foundation before identity:

1. **You can't delete a meeting.** A recording you didn't want (a test, a personal call, a misfire)
   is stuck. There *is* a DB delete (`api_delete_meeting` → `MeetingsRepository::delete_meeting`,
   `database/repositories/meeting.rs:159`) wired into `lib.rs:741`, but it (a) has **no UI** and
   (b) **leaves the on-disk recording folder** (`meetings.folder_path` under
   `~/Movies/meetily-recordings/<meeting>/`) orphaned — the audio/WAV files survive a "delete."
2. **You can't capture an in-person / phone / browser meeting.** Vinyl only exists once it's
   recording system audio. A catch-up at a desk, a phone call, a Meet tab Vinyl can't tap — there's
   no way to open just the notepad and still get a summary. 0013 Wave 2a calls this **notes-only
   meetings** and makes it a small delta via `meetings.origin`.
3. **"Join & Record" creates an *unrelated* object.** `joinAndRecord` (`lib/calendar.ts:155`) opens
   Zoom then fires the generic start path, which mints a fresh timestamp-titled meeting with no
   memory of the calendar event. The recording is a sibling of the calendar meeting, not the same
   thing: wrong title, no attendees, no link back. Worse, the existing attendee association
   (`api_get_meeting_attendees`, `diarization/commands.rs:250`) has to **re-locate the event by
   title + recording-start instant** "since we don't persist an event id" — a documented brittleness
   this cluster removes.

### Grounding (verified in this repo, 2026-06-26)

- **DB delete exists and cascades the right tables — but not files.**
  `delete_meeting_with_transaction` (`meeting.rs:399`) deletes `transcript_chunks`,
  `summary_processes`, `transcripts`, `meeting_notes`, `speakers`, then `meetings`, all in one
  transaction (it deletes explicitly because the pool runs without `PRAGMA foreign_keys = ON`).
  **No file deletion.** `meetings.folder_path` (`migrations/20251006000000_add_audio_sync_fields.sql`)
  is the recording folder; `~/Movies/meetily-recordings/<meeting>/` is computed in
  `audio/recording_preferences.rs:59`. The frontend has no caller of `api_delete_meeting`.
- **Meeting creation is already a real persisted row at start.** `api_create_meeting(meetingTitle,
  folderPath)` → `MeetingsRepository::create_meeting(title, folder_path)` (`meeting.rs:34`) inserts
  `id,title,created_at,updated_at,folder_path`. `useRecordingStart` calls it with `folderPath: null`.
  A notes-only meeting is the **same INSERT** with no recording ever attached.
- **The notes layer is whole and meeting-keyed.** `meeting_notes(meeting_id PK, notes_markdown,
  notes_json, …)` (`migrations/20251223000000_add_meeting_notes.sql`); `NotepadPanel`
  (`app/_components/NotepadPanel.tsx`) + the lightweight `components/NoteEditor/NoteEditor.tsx`
  autosave to it via the existing notes IPC. No schema work needed for notes-only beyond the meeting
  row's type.
- **The summary already has a notes-grounding branch and tolerates a thin transcript.**
  `summary/service.rs:549` loads `meeting_notes` and passes them as `user_notes` into
  `processor.rs` (the `NOTES_GROUNDING_INSTRUCTIONS` path, `processor.rs:183`). The notes are folded
  into the **final** synthesis pass; the per-chunk pass operates on the transcript `text`. **Open
  verification:** the no-transcript path (`text == ""`) — `chunk_text` early-returns on empty
  (`processor.rs:268`) and `service.rs:250` guards on `folder_path`. Notes-only summary must run the
  notes-grounding final pass with an empty/absent transcript instead of aborting (see Risks).
- **Calendar already carries event id + attendees — the recording just doesn't keep them.**
  `UpcomingMeeting` (`calendar/eventkit.rs:29`) has a stable EventKit `id`; `DayAgendaItem`
  (`lib/day-agenda.ts:34`) and the Rust `Attendee` (`eventkit.rs:50`: name/email/isCurrentUser) are
  already in the agenda payload. `event_attendees(title, started_at)` (`eventkit.rs:406`) is the
  match-by-title-and-time hack we replace with a stored `calendar_event_id`.
- **`meetings` is the narrowest table to extend.** `meetings(id,title,created_at,updated_at,
  folder_path)` — no `origin`, no `calendar_event_id` (0008 listed the latter as a deferred
  nice-to-have, `specs/0008` §Data model; 0013 promotes it to "needed now").

## Goals

- **Delete a meeting, completely** — DB rows **and** the on-disk recording folder — from the UI,
  behind a confirm, reusing the existing transaction (extended for files).
- **Notes-only meetings** — create a `meetings.origin='notes_only'` row that opens the notepad with
  **no** audio capture, autosaves to `meeting_notes`, and produces a notes-grounded summary that
  degrades cleanly with no transcript.
- **Join & Record adopts the calendar meeting's identity** — the recording IS the calendar event:
  its **title**, its **attendees** (carried through for speaker association), and a stored
  **`meetings.calendar_event_id`** link, so attendee lookup stops guessing by title+time.
- All on-device; no new outbound traffic; no new dependency; no ADR (this cluster is deliberately the
  one that touches none of the privacy-sensitive seams).

## Non-goals

- **NO identity / voiceprint / People work.** No `people` table, no `voiceprints`, no
  cross-meeting matcher, no role-weighting. That is **Cluster 2** (`specs/0013` Wave 1 =
  `specs/0011` + `specs/0012`) and is explicitly out of scope here. We *carry attendees through* so
  Cluster 2 can consume them, but we do not build the person entity.
- **`origin='imported'` behavior.** We add the enum value (it's free in a `TEXT` column) and default
  correctly, but audio import already exists (`audio/import.rs`) and re-typing imported meetings is a
  follow-up, not this spec.
- **Topics, action items, analytics, pre-call prep, Google Calendar, Zoom-as-speaker** — later 0013
  waves.
- **Calendar *write*** (creating events), recurring-series detection, multi-select / bulk delete,
  trash/undo. Delete is immediate-with-confirm.
- **Editing `origin` after creation** (e.g. promoting a notes-only meeting to recorded). One-way at
  creation for now.

## Approach

**Make `meetings` a typed, linkable, deletable object via two nullable columns on the existing table,
and reuse every existing path rather than forking new ones.** ✅

- **Delete** = wire the *existing* `api_delete_meeting` to a confirm dialog in the UI, and **extend
  the existing transaction** to delete `folder_path` from disk (best-effort, after the DB commit, so
  a file error never leaves a half-deleted DB). One command, one new file step, real UI.
- **Notes-only** = a meeting row with `origin='notes_only'` and **no recording**. Creation reuses
  `api_create_meeting` (add an `origin` param) — it does *not* go through `startRecordingWithDevices`,
  so no Core Audio tap, no transcription engine, no `folder_path`. The notepad and summary already
  work meeting-keyed; the only behavioral change is "don't show a transcript tab / don't start audio
  when `origin='notes_only'`," and "summary grounds on notes with an empty transcript."
- **Join & Record identity** = at join time, pass the calendar event's `id`, `title`, and
  `attendees` into meeting creation: create the row up front with the event title + a stored
  `calendar_event_id`, set it as `currentMeeting`, then start recording against that existing id
  (mirroring how `useRecordingStart` already persists-at-start). `api_get_meeting_attendees` then
  resolves attendees by the stored event id (exact) instead of title+time (fuzzy).

Why this over alternatives: a separate "notes" table or a `meeting_type` join table would duplicate
the entire notes/summary/list machinery that is already keyed to `meetings(id)`; 0013's whole
argument is that `meetings.origin` makes notes-only a *delta*. A new `delete_recording_files` command
separate from the DB delete would risk drift between the two; folding files into the existing
transaction's success path keeps "delete" atomic from the user's view.

## Design

### Data model

Two forward-only migrations under `frontend/src-tauri/migrations/` (CLAUDE.md; additive `ALTER`s,
both nullable so existing rows are valid):

- **`…_add_meeting_origin.sql`** (new):
  ```sql
  -- 2.0 Cluster 1 (specs/0015): type a meeting by how it was created.
  -- 'recorded' (default, every existing + future recorded meeting) | 'notes_only' | 'imported'.
  -- Nullable with a default so existing rows read as 'recorded' without a backfill UPDATE.
  ALTER TABLE meetings ADD COLUMN origin TEXT NOT NULL DEFAULT 'recorded';
  ```
  (`NOT NULL DEFAULT` is safe on `ALTER ADD` in SQLite — existing rows take the default.)

- **`…_add_meeting_calendar_event_id.sql`** (new):
  ```sql
  -- 2.0 Cluster 1 (specs/0015): link a recording back to its calendar event so attendee
  -- lookup is exact (replaces the title+start-instant re-location in diarization/commands.rs).
  -- Nullable: ad-hoc/notes-only/pre-existing meetings have no event. Not UNIQUE — the same
  -- recurring event id can recur across occurrences; we don't dedupe here.
  ALTER TABLE meetings ADD COLUMN calendar_event_id TEXT;
  CREATE INDEX IF NOT EXISTS idx_meetings_calendar_event_id
      ON meetings(calendar_event_id) WHERE calendar_event_id IS NOT NULL;
  ```

**Attendee storage decision: do NOT persist attendees in this cluster.** Attendees are read live from
EventKit on demand (`event_attendees`) and, after this spec, resolved by the stored
`calendar_event_id` (exact match) rather than title+time. The durable home for attendees is the
**People** entity in Cluster 2 (`specs/0012`); creating a parallel `meeting_attendees` table now
would be the throwaway parallel structure 0013 warns against. The *flow-through* this spec guarantees
is: Join & Record stores `calendar_event_id` → `api_get_meeting_attendees` returns the event's
attendees exactly → `api_assign_speaker_to_attendee` (already writes `speakers.email`) is the bridge
Cluster 2 promotes to People. No new attendee table.

`MeetingModel` / `MeetingMetadata` (`database/models.rs:6`) and the list/detail query
(`get_meetings_enriched`, `meeting.rs:81`) gain `origin` and `calendar_event_id` selects so the
frontend can branch on type.

### Tauri IPC (register/extend in `frontend/src-tauri/src/lib.rs`)

- **`api_delete_meeting` (extend, no signature change):** after the DB transaction commits
  successfully, read the meeting's `folder_path` (fetched *before* the delete, inside the same call)
  and remove that directory recursively via `std::fs::remove_dir_all`, **best-effort** — log and
  return success even if the folder is already gone; never roll back the committed DB delete on a
  file error. Guard: only delete a path that is under the configured recordings root
  (`recording_preferences` root) to avoid deleting an arbitrary stored path.
- **`api_create_meeting` (extend):** add `origin: Option<String>` (default `'recorded'`) and
  `calendar_event_id: Option<String>`. `MeetingsRepository::create_meeting` gains both params and
  binds them in the INSERT. Existing callers (`useRecordingStart`) pass `origin: 'recorded'` /
  `null` — byte-compatible default behavior.
- **`api_get_meeting` / list (extend):** include `origin` + `calendarEventId` in the returned DTOs so
  the UI hides the transcript tab and audio controls for `notes_only`.
- **`api_get_meeting_attendees` (rework, `diarization/commands.rs:273`):** if the meeting has a
  `calendar_event_id`, fetch attendees by event id (add `eventkit::event_attendees_by_id(event_id)`
  alongside the existing title/time fallback). Keep the title+time path as the fallback for legacy
  recorded meetings with no stored id, so nothing regresses.

No new events. Notes-only and Join & Record reuse the existing notes IPC
(`api_get_meeting_notes`/`api_save_meeting_notes`) and recording start/stop flow respectively.

### UI (`frontend/src/`)

- **Delete affordance:** an overflow/"…" action on a meeting in the meetings list
  (`app/meetings/page.tsx`) and on the meeting-details header
  (`app/meeting-details/page-content.tsx`) → a confirm dialog ("Delete this meeting? This removes the
  recording, transcript, and notes. This can't be undone.") → `invoke('api_delete_meeting', { meetingId })`
  → toast + navigate away / refetch (`useSidebar().refetchMeetings`). Reuse the existing
  dialog/dropdown primitives (`components/ui/`).
- **New note entry point:** a "New note" action in the sidebar (next to / under the record affordance)
  and on Home → calls `api_create_meeting({ origin: 'notes_only', meetingTitle: <"Untitled note" or
  default> })`, sets `currentMeeting`, and routes to the meeting-details / notes view in **notes-only
  mode** (no recording started). Reuse `NoteEditor`/`NotepadPanel` for the editor surface.
- **Meeting-details, notes-only mode:** `page-content.tsx` already tabs Summary / Transcript / My
  notes (`page-content.tsx:178`). When `meeting.origin === 'notes_only'`: **drop the Transcript tab**,
  default to **My notes**, keep **Summary** (notes-grounded), and hide the recording/audio controls.
  Recorded meetings are unchanged.
- **Join & Record:** the agenda/upcoming components (`components/Calendar/DayAgenda.tsx`,
  `UpcomingMeetings.tsx`, `CalendarAlerts.tsx`) already have the event (`DayAgendaItem` with `id`,
  `title`, `attendees`, `zoomUrl`). Change `joinAndRecord` (`lib/calendar.ts:155`) + the start path so
  the click: (1) creates the meeting row with the event's `title` + `calendarEventId` (origin
  `'recorded'`), (2) sets it as `currentMeeting` so persist-at-start doesn't mint a second row, then
  (3) opens Zoom and starts recording against that id (honoring the existing
  `JOIN_AND_RECORD_DELAY_MS`). `useRecordingStart.createMeetingForRecording` must respect an
  already-created `currentMeeting` (its `meetingCreatedRef` guard) so it does not create a duplicate.

## Tasks (ordered; phased delete → meeting-object/notes-only → join&record-identity)

### Phase A — Delete a meeting (smallest, self-contained)
1. [ ] **rust-core-engineer** — extend `api_delete_meeting` (`api/api.rs:835`) to capture
   `folder_path` before the transaction and, on commit success, `remove_dir_all` the recording folder
   best-effort with a recordings-root guard (`audio/recording_preferences.rs` root). Keep the DB
   transaction (`meeting.rs:399`) as-is.
2. [ ] **frontend-engineer** — delete action + confirm dialog on the meetings list
   (`app/meetings/page.tsx`) and meeting-details header (`app/meeting-details/page-content.tsx`);
   wire to `api_delete_meeting`; toast + refetch/navigate.

### Phase B — Meeting-object model + notes-only meetings
3. [ ] **rust-core-engineer** — migrations `…_add_meeting_origin.sql`,
   `…_add_meeting_calendar_event_id.sql`; add `origin`/`calendar_event_id` to `MeetingModel` /
   `MeetingMetadata` (`database/models.rs`) and the list/detail queries (`meeting.rs`).
4. [ ] **rust-core-engineer** — extend `api_create_meeting` + `MeetingsRepository::create_meeting`
   with `origin` + `calendar_event_id` (default `'recorded'`/`null`); surface `origin`/`calendarEventId`
   in `api_get_meeting` and the list DTO.
5. [ ] **frontend-engineer** — "New note" entry points (sidebar + Home) → `api_create_meeting({
   origin: 'notes_only' })` → notes-only meeting-details view; hide Transcript tab + audio controls
   when `origin === 'notes_only'` (`page-content.tsx`); reuse `NoteEditor`/`NotepadPanel`.
6. [ ] **llm-pipeline-engineer** — confirm + harden the notes-only summary path: with no transcript,
   `process_transcript`/`service.rs` must run the `NOTES_GROUNDING_INSTRUCTIONS` final pass over the
   `meeting_notes` content and **not abort** on empty `text` (guard the empty-transcript / no
   `folder_path` early-returns at `service.rs:250`, `processor.rs:268`). Output a notes-only summary.

### Phase C — Join & Record adopts calendar identity
7. [ ] **rust-core-engineer** — `eventkit::event_attendees_by_id(event_id)`; rework
   `api_get_meeting_attendees` (`diarization/commands.rs:273`) to prefer the stored
   `calendar_event_id`, falling back to the existing title+time path for legacy rows.
8. [ ] **frontend-engineer** — change `joinAndRecord` (`lib/calendar.ts`) + the Join & Record callers
   (`DayAgenda.tsx`, `UpcomingMeetings.tsx`, `CalendarAlerts.tsx`) to create the meeting with the
   event `title` + `calendarEventId` and set `currentMeeting` before starting; ensure
   `useRecordingStart.createMeetingForRecording` honors the pre-created meeting (no duplicate row).

## Acceptance criteria (tie to the Definition of Done in `/CLAUDE.md`)

- **Delete:** deleting a meeting from the UI (after confirm) removes the `meetings` row and all
  dependent rows (`transcripts`, `meeting_notes`, `speakers`, `summary_processes`,
  `transcript_chunks`) **and** the `~/Movies/meetily-recordings/<meeting>/` folder; the meeting
  disappears from the list/sidebar; a file-system error does not leave the DB half-deleted.
- **Notes-only:** "New note" creates a `meetings.origin='notes_only'` row with **no** recording, no
  `folder_path`, no transcripts; the notepad autosaves to `meeting_notes`; the details view has no
  Transcript tab and no audio controls; generating a summary produces a notes-grounded summary with
  no transcript and does not error.
- **Join & Record identity:** clicking Join & Record on a calendar event produces **one** meeting row
  titled with the event title, with `calendar_event_id` set; `api_get_meeting_attendees` returns that
  event's attendees by id (no title/time guessing) and they are available to
  `api_assign_speaker_to_attendee` for the existing speaker legend.
- **No regression:** recorded meetings (manual record path) behave exactly as before — `origin`
  defaults to `'recorded'`, summary output is unchanged for meetings with a transcript and no notes.
- **Privacy:** no new outbound traffic; everything is local DB + local files + the existing
  user-chosen LLM summary call.
- **Gate (`/check`):** `cargo check` + `cargo clippy` clean (`frontend/src-tauri`); `pnpm lint` clean
  (`frontend`); app launches via `./clean_run.sh`; record → live transcript → summary smoke unchanged.

## Risks / open questions

- **Notes-only summary degrade (Phase B, Task 6) — primary risk.** The notes-grounding code injects
  notes into the *final* pass, but the pipeline assumes a transcript drives chunking and
  `service.rs:250` early-returns when `folder_path` is absent (notes-only has none). The fix is small
  but must be verified: route notes-only through a "no-transcript, notes-only" branch that still calls
  the final synthesis with `user_notes` set and `text` empty. **Open:** confirm the language-detection
  (`detect_summary_language`, `service.rs:267`) tolerates empty transcript text (fall back to notes
  language or default).
- **File-delete safety (Phase A).** `remove_dir_all` on a path read from the DB must be sandboxed to
  the recordings root so a malformed/legacy `folder_path` can never delete outside it. Best-effort
  after commit means a locked file (e.g. mid-write) won't block the DB delete — acceptable; log it.
- **Duplicate meeting row on Join & Record (Phase C).** Two creation paths (the new pre-create and
  `createMeetingForRecording`) must not both insert. Mitigation: set `currentMeeting` + rely on the
  existing `meetingCreatedRef` guard; test explicitly (0008 already flagged a double-record guard).
- **`calendar_event_id` uniqueness / recurring events.** A recurring event reuses an id across
  occurrences; we intentionally don't make it UNIQUE and don't dedupe. Recurring-series semantics are
  a later wave (pre-call prep). Note it; don't solve it here.
- **Bare dev binary has no calendar access** (`lib/calendar.ts` header): Join & Record identity is
  only fully testable in the bundled `Vinyl.app`/`Dev Vinyl` (EventKit needs a signed bundle). The
  notes-only and delete paths are testable in either.
- **`origin` is one-way at creation.** No promote/demote. If users want to "start recording" inside a
  notes-only meeting later, that's a follow-up (would flip `origin` + attach a recording).

## Verification

- **Delete:** create a throwaway recorded meeting; note its `folder_path`; delete via UI → confirm
  the row and `~/Movies/meetily-recordings/<meeting>/` are both gone and the list refreshes. Repeat
  with the folder pre-deleted to confirm best-effort success. Confirm dependent rows are gone
  (`sqlite3` count on `transcripts`/`meeting_notes`/`speakers` for that id = 0).
- **Notes-only:** New note → type notes → reopen (autosave persisted) → generate summary → confirm a
  notes-grounded summary with no transcript and no error; confirm no `folder_path`/transcripts and no
  Transcript tab.
- **Join & Record identity:** in the bundled app, click Join & Record on a calendar event with
  attendees → confirm one meeting row titled with the event, `calendar_event_id` set
  (`sqlite3`), and `api_get_meeting_attendees` returns the event's attendees; assign one to a speaker
  and confirm `speakers.email` is written.
- Run `/check` (cargo check/clippy in `frontend/src-tauri`; `pnpm lint` in `frontend`;
  `./clean_run.sh`; record → transcript → summary smoke).

## Sources

- Parent program + standalone-content/`origin`/`calendar_event_id` framing:
  `specs/0013-identity-and-organization-program.md` (Primitive 2; Wave 2a notes-only).
- Calendar/attendees seam: `specs/0008-calendar-zoom-integration.md` (deferred `calendar_event_id`);
  `frontend/src-tauri/src/calendar/eventkit.rs` (`UpcomingMeeting`, `Attendee`, `event_attendees`);
  `frontend/src/lib/calendar.ts` (`joinAndRecord`), `frontend/src/lib/day-agenda.ts` (`DayAgendaItem`).
- Delete cascade: `frontend/src-tauri/src/database/repositories/meeting.rs:159,399`;
  `frontend/src-tauri/src/api/api.rs:835`; `lib.rs:741`; recordings root
  `frontend/src-tauri/src/audio/recording_preferences.rs:59`; `meetings.folder_path`
  `migrations/20251006000000_add_audio_sync_fields.sql`.
- Meeting creation / recording start: `frontend/src/hooks/useRecordingStart.ts`
  (`createMeetingForRecording`, `generateMeetingTitle`); `MeetingsRepository::create_meeting`
  (`meeting.rs:34`).
- Notes + summary degrade: `migrations/20251223000000_add_meeting_notes.sql`;
  `frontend/src/components/NoteEditor/NoteEditor.tsx`, `frontend/src/app/_components/NotepadPanel.tsx`;
  `frontend/src-tauri/src/summary/service.rs:549` + `processor.rs:183` (notes grounding).
- Attendee→speaker bridge (Cluster 2 boundary): `frontend/src-tauri/src/diarization/commands.rs:220,250`
  (`api_assign_speaker_to_attendee`, `api_get_meeting_attendees` — the title+time re-location we replace).
- Schema baseline: `migrations/20250916100000_initial_schema.sql` (`meetings`, `transcripts`,
  `summary_processes`, `transcript_chunks`), `…_add_speakers_table.sql`.
