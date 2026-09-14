# 0014 — App-flow redesign rollout (App.dc.html)

- **Status:** In progress
- **Owner agent(s):** frontend-engineer
- **Roadmap phase:** Phase 2 (UX)

## Context / Problem
The Vinyl UI Claude Design project shipped a connected prototype — `templates/app/App.dc.html`,
the *"Vinyl App (flow)"* template — that redesigns the whole app: sidebar, Home dashboard,
live recording (transcript + notes), AI summary, a floating recording bar, and a permissions
modal. It uses the **Warm Editorial** look (Newsreader serif headings, `--brand`/`--record`/
`--chart-*` tokens) that is already wired in `globals.css`.

The comp was first ported verbatim as a self-contained, full-screen prototype route at
`/design-preview` (`frontend/src/app/design-preview/page.tsx`) using the real tokens but mock
data. This spec covers rolling that design into the **real, Tauri-wired app**, screen by screen.

The in-repo source of truth for the *visual* target is `/design-preview`; the source of truth
for *behavior/data* is the existing wired screens.

## Goals
- Land the redesign on the screens where it can be done **safely without live GUI verification**
  (this environment has no WindowServer; we verify via lint/tsc/build only).
- Preserve every piece of existing functionality and Tauri wiring — no regressions to the
  record → transcript → summary smoke path.
- Keep `/design-preview` as the living reference until the real screens fully match it.

## Non-goals
- Blind-rewriting the deeply-wired real-time screens (live `/record`, the BlockNote summary
  editor) in a way that relocates functional controls — deferred to a supervised session.
- Re-theming — tokens/fonts already exist; this is layout/structure only.
- Any backend/Tauri/migration changes. Frontend-only.

## Approach
Roll out in waves, lowest-risk-first, composing existing wired components rather than replacing
their logic. Each screen is gated on `pnpm lint` + `tsc` + `next build`, then `/code-review`.

**Wave 1 — landing now (this branch, autonomous):**
- **Home `/`** — rebuild `frontend/src/app/page.tsx` to the comp's dashboard layout
  (time-based greeting + day summary, search affordance, Record CTA; "Happening now" /
  "Later today" / "Recent recordings"). Drive the agenda sections from the **existing real
  calendar data** that `Calendar/DayAgenda` already loads (`now`/`upcoming`/`past` phases,
  Join-&-record / Summarize / Return-to-recording actions, calendar-connect prompt). Drive
  "Recent recordings" from `api_get_meetings` (existing `DashboardMeeting` shape + grouping).
  Reuse existing handlers (`handleRecord` autostart flag, `handleOpen` navigation). No
  fabricated calendar data — sections that have no real source are omitted/empty-stated.
- **Floating recording bar** — restyle `components/GlobalRecordingBar.tsx` toward the comp's
  dark pill (blinking dot, title + state·elapsed, mini waveform, pause / return / stop) while
  preserving ALL of its wiring: `useRecordingState`, the `sessionInFlight`/`isFinalizing`
  gating, pause/resume via `recordingService`, and the stop-via-`/record` flow
  (`stopRecordingOnLoad` + `stop-recording-from-global-bar`).

**Wave 2 — staged for a supervised session (documented, not landed blind):**
- Live `/record` header/controls redesign (relocates start/stop/pause; must be watched while
  actually recording — and `RecordingControls`/`StatusOverlays`/recovery dialog must keep working).
- Summary `/meeting-details` single-column doc layout + Summary/Transcript/Notes tabs (wraps the
  BlockNote summary editor + generate/regenerate/export/language controls — must be eyeballed).
- Sidebar restructure (the comp drops the meeting list — a functional regression to resolve;
  shared navigation chrome, high blast radius).
- Permissions pre-record modal wired to `trigger_microphone_permission` /
  `trigger_system_audio_permission_command` (net-new flow; needs real permission testing).

## Design
### Data model
No changes.

### Tauri IPC
No new commands/events. Reuses: `api_get_meetings`, `Calendar/DayAgenda`'s existing agenda
loaders + actions, `recordingService` pause/resume.

### UI
- `frontend/src/app/page.tsx` — rebuilt (Wave 1).
- `frontend/src/components/Calendar/DayAgenda.tsx` — may be restyled toward the comp **without
  changing its data/actions** (optional; conservative path is to keep it as-is and adopt only
  the comp header + recent-recordings list).
- `frontend/src/components/GlobalRecordingBar.tsx` — restyled (Wave 1).
- `frontend/src/app/design-preview/page.tsx` — kept as the reference; temporary sidebar entry
  point + "Exit preview" hatch.

## Tasks
1. [ ] Home `/` rebuilt on real data (frontend-engineer).
2. [ ] Floating recording bar restyled, wiring preserved (frontend-engineer).
3. [ ] `/code-review` clean; lint + tsc + `next build` green.
4. [ ] Wave 2 screens implemented in a supervised session (deferred).

## Acceptance criteria
- `pnpm lint`, `tsc --noEmit`, and `next build` are clean.
- Home loads real meetings + real day agenda; Record CTA still auto-starts; opening a meeting
  still navigates to `/meeting-details`; calendar-connect prompt still appears when unauthorized.
- The floating bar still appears only while a session is in flight and not on `/record`, and
  pause/resume/stop still work via the existing flows.
- `/design-preview` still renders and is reachable + exitable.

## Risks / open questions
- **No live GUI verification in this environment** — Wave 1 is restricted to changes provable
  by lint/tsc/build + careful reading; Wave 2 is explicitly deferred for this reason.
- DayAgenda is large (653 lines) with subtle EventKit status-propagation handling — restyling it
  is optional and must not touch that logic.
- Greeting has no real user-name source — use a time-of-day greeting without a name.

## Verification
`cd frontend && pnpm lint && npx tsc --noEmit -p tsconfig.json && pnpm build`. Manual smoke
(tomorrow, on device): Home renders agenda + recordings; Record auto-starts; floating bar
pause/resume/stop; `/design-preview` reachable from sidebar and exitable.
