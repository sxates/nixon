# 0026 — Dismiss / Ignore Calendar Events

- **Status:** Implemented on `fix/0024-1.1-feedback` (pending GUI verification). Migration +
  repository + 3 commands + agenda filtering landed with tests; UI hide/unhide + "Show hidden"
  toggle wired. See **Implementation notes** below.
- **Owner agent(s):** frontend-engineer + rust-core-engineer
- **Roadmap phase:** Post-1.1 hardening (graduated from `specs/0024` WS5.2)

## Context / Problem

Graduated from `specs/0024` (1.1 feedback note 8). The home agenda surfaces non-meetings (e.g. a
colleague's doctor's appointment, personal blocks). The user wants to dismiss/ignore such items so
they stop appearing in the upcoming list.

No dismiss/ignore mechanism exists for events today. The only "dismiss" in the calendar UI is the
connect-calendar *prompt* banner (localStorage `vinyl-calendar-connect-dismissed`,
`UpcomingMeetings.tsx:37,171` / `DayAgenda.tsx:58,575,608`), and `ZoomAutoDetect`'s "Ignore" only
closes a single auto-detect toast. The agenda renders backend items with only phase/recording
filters and no per-event exclusion (`DayAgenda.tsx:777-794`; `PHASE_ORDER` `:72`).

Broken out because it needs a persisted store (migration + commands) and its own UX (hide/undo,
recurring semantics) — larger than a one-line tweak.

## Goals

- Let the user hide a specific calendar event from the agenda; it stays hidden across restarts.
- Provide a way to reveal/undo hidden events.
- Handle recurring events sensibly (dismiss the occurrence by default).

## Non-goals

- Auto-classifying "meeting vs not-a-meeting" heuristically — this is an explicit user action.
- Editing the calendar itself (we never write to the user's calendar).

## Approach

Persist per-event dismissal keyed by the EventKit event id (`DayAgendaItem.id`, `day-agenda.ts:35`)
in a small SQLite table (so it survives and can be filtered backend-side), exposed via Tauri
commands, and filter dismissed ids in the agenda build. Prefer SQLite over localStorage for parity
with the rest of the calendar/meeting data and to filter at the source.

## Design

### Data model

New migration under `frontend/src-tauri/migrations/` (next timestamp), mirroring
`20260626000001_add_meeting_calendar_event_id.sql`:

```sql
CREATE TABLE dismissed_calendar_events (
  event_id   TEXT PRIMARY KEY,   -- EventKit DayAgendaItem.id (occurrence id by default)
  dismissed_at TEXT NOT NULL
);
```

### Tauri IPC

- `api_dismiss_calendar_event(event_id)` and `api_undismiss_calendar_event(event_id)` (and an
  `api_list_dismissed_calendar_events` for the "show hidden" affordance), registered in
  `frontend/src-tauri/src/lib.rs`. Implement in `frontend/src-tauri/src/calendar/`.

### UI

- Filter dismissed ids in the agenda build (`frontend/src-tauri/src/calendar/day_agenda.rs`) or in
  `DayAgenda` `refresh`/`grouped` (`DayAgenda.tsx:673-689,777-794`).
- Per-row "…" menu → "Hide this event"; a lightweight "Show hidden" toggle / undo
  (`frontend/src/components/Calendar/DayAgenda.tsx`, `frontend/src/lib/day-agenda.ts`).

## Tasks

1. [x] (rust) Migration `20260703000000_add_dismissed_calendar_events.sql`; `DismissedCalendarEventsRepository`
   (`dismiss`/`undismiss`/`all`); `api_dismiss_calendar_event` / `api_undismiss_calendar_event` /
   `api_list_dismissed_calendar_events`; registered in `lib.rs`.
2. [x] (rust) `build_agenda` takes the dismissed set and sets a `dismissed` flag on each item
   (only for UNrecorded calendar rows — a recorded event is never hidden). Unit-tested.
3. [x] (frontend) `dismissCalendarEvent`/`undismissCalendarEvent` in `day-agenda.ts`; a subtle
   per-row "hide" (EyeOff) on non-meeting calendar rows across all three agenda variants; a muted
   "Unhide" row; a foot-of-list "Show N hidden events" toggle; Undo toast on hide. Home count
   excludes dismissed.
4. [x] Tests: `day_agenda::dismissed_unrecorded_event_is_flagged_recorded_one_is_not` (Rust);
   migration applies cleanly (db_lifecycle 6/6). Manual GUI verification still pending.

## Implementation notes

- **Backend filtering vs. a flag:** chose to return dismissed items with a `dismissed: bool` flag
  (not omit them) so the frontend can offer "Show hidden" + per-row "Unhide" without a second
  query that lacks titles. The home agenda and count both exclude `dismissed` by default.
- **Recorded events are never hidden:** `dismissed` is set only when `meeting_id.is_none()`, so a
  recurring event you dismissed but later recorded still shows (you recorded it).
- **Key:** the EventKit event id as the agenda exposes it (`DayAgendaItem.id` for an unrecorded
  calendar row, or the synthetic `evt-<hash>`); dismissing hides all occurrences sharing that id,
  which matches "ignore this thing on my calendar" (the recurring-id caveat is acceptable here).

## Acceptance criteria

- Tie back to the Definition of Done in `/CLAUDE.md`.
- A calendar event can be dismissed and stays hidden across restarts, with a way to reveal/undo.
- Recurring occurrences dismiss the occurrence by default.

## Risks / open questions

- **Recurring events:** dismiss-occurrence vs dismiss-series. Default to the occurrence id, but
  recurring occurrences can share an EventKit id (the `specs/0019` WS6.3 caveat) — verify the id is
  occurrence-unique before relying on it as the dismissal key; if not, key on `(id, start_time)`.
- **Stale rows:** dismissed ids for long-past events accumulate; a periodic prune is optional.

## Verification

- Rust: `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal && cargo
  clippy && cargo test --features metal`.
- Frontend: `cd frontend && pnpm lint && pnpm test`.
- Manual: hide a non-meeting event → it disappears; restart → still hidden; "show hidden" → undo.
