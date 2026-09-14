# 0007 — UX redesign: a Granola-grade interface for back-to-back meetings

- **Status:** Approved-for-build 2026-06-24 — decisions locked (see "Decisions locked" below);
  building incrementally in slices. (Originally: analysis + redesign proposal for review.)
- **Owner agent(s):** spec-architect (this doc); implementation later by frontend-engineer
  (+ rust-core-engineer for shortcut/tray + meeting-list query work)
- **Roadmap phase:** Phase 5 (Polish) — but several quick wins should jump ahead; sets up
  `specs/0008` (calendar / Zoom auto-detection)

> This is a design/analysis deliverable. It audits the *actual* current UI (grounded in the
> code read below), critiques it against the Granola bar for an all-day-Zoom user, and proposes
> a concrete redesign with wireframes and a prioritized plan. It does not change code.

---

## Context / Problem

Vinyl's flagship flow (live notepad + notes-aware summary, `specs/0003`) shipped, but in
Brian's words *"the usability and craft of the interface is nowhere near Granola quality."*
The product inherited meetily's UI wholesale and bolted the notepad onto it. The result is a
recording-centric tool, not a meeting **operating surface**.

The target user lives in **back-to-back Zoom meetings all day**. For them the interface is
crossed dozens of times per day under time pressure: a meeting is *starting now*, they need to
be recording + taking notes in seconds, then get out, and later find one meeting among many.
Every extra click, every ambiguous state, every "where did my notes go" is a tax paid all day.

This spec measures the current app against that user and proposes a redesign.

---

## Current state — audit (grounded in the code)

### Global shell / IA
- **Shell** (`frontend/src/app/layout.tsx`): a left `Sidebar` + `MainContent` that offsets by
  the sidebar width (`components/MainContent/index.tsx`: `ml-16`/`ml-64` + a stray `pl-8`).
  Routes are only three: `/` (record), `/meeting-details?id=…` (post-meeting), `/settings`.
  An orphaned `/notes/[id]` path is still referenced in the sidebar router (`Sidebar/index.tsx`
  line ~582) but was retired in the 0003 pivot — dead branch.
- **Sidebar** (`components/Sidebar/index.tsx`, `Sidebar/SidebarProvider.tsx`): defaults to
  **collapsed** (`isCollapsed = true`). Contains a logo, a search box ("Search meeting
  content..."), a "Home" row, a single hard-coded **"Meeting Notes"** folder listing every
  meeting as a flat file list, plus footer buttons (record / import / settings / Info / a
  hard-coded `v0.4.0`). Each meeting renders only `meeting.title` (an icon + the title string),
  with hover-revealed edit/delete. There is **no date, no time, no duration, no source, no
  summary preview** on a meeting row.

### The "home" screen (`frontend/src/app/page.tsx`)
- Home **is the recording screen**. There is **no dashboard / meeting list / "today" view**.
  Landing on `/` you get a 50/50 split: live transcript (left, `_components/TranscriptPanel`)
  and live notepad (right, `_components/NotepadPanel`), with a floating pill of recording
  controls fixed at `bottom-12` (`RecordingControls.tsx`).
- Before recording, both panes are essentially empty; the transcript pane shows a
  `PermissionWarning`. There is no "you have nothing recording yet, here's your day / recent
  meetings / press X to start" state — just two blank editors.

### Starting a recording (friction path)
- Start is **manual** and routed several ways: the floating mic button, the sidebar footer
  button, the collapsed-sidebar mic icon (`Sidebar/index.tsx`), or the macOS **tray** menu
  ("Start Recording", `src-tauri/src/tray.rs:334`). Sidebar start uses a `sessionStorage`
  `autoStartRecording` flag + a `start-recording-from-sidebar` window event
  (`SidebarProvider.tsx`, `useRecordingStart.ts`) — three near-duplicate start code paths.
- **There are no keyboard shortcuts anywhere.** Grep shows no `global_shortcut` plugin usage
  and no `accelerator` on any tray/menu item (`src-tauri/src/tray.rs`). The only key handlers
  in the whole frontend are inline Enter/Escape in the edit-title dialog and the BlockNote
  editor internals. For an all-day-Zoom user this is the single biggest miss.
- On start, the meeting is auto-named `Meeting DD_MM_YY_HH_MM_SS`
  (`useRecordingStart.ts:generateMeetingTitle`) — a machine string, not a human title.
- Start can be blocked late by a model-readiness check (`checkParakeetReady`) that fires a
  toast/modal — friction discovered at the worst moment (meeting starting).

### During the meeting
- 50/50 transcript|notepad. Reasonable concept, but: the notepad has no formatting affordances
  surfaced, no timestamp anchoring, no "/" command hinting beyond raw BlockNote. The transcript
  pane has only Copy + (Whisper-only) Language buttons; **no live elapsed timer, no audio level
  meter, no speaker labels** (diarization is Phase 3), no "jump to live".
- **Notes-persistence is fragile by construction** (documented in 0003): during recording the
  meeting has no DB row (`currentMeeting.id === 'intro-call'` placeholder), so notes live in an
  in-memory draft store (`NotesDraftContext`) and are flushed only on the stop→navigate
  transition. Any crash mid-meeting loses notes. There *is* a transcript-recovery dialog
  (`TranscriptRecovery`) but it's transcript-only.

### Stopping → summary (`useRecordingStop` → `/meeting-details?source=recording`)
- On stop the app navigates to `meeting-details`. Summary auto-generates if `source=recording`
  AND the user's auto-summary toggle is on AND a model is configured
  (`meeting-details/page.tsx:setupAutoGeneration`) — otherwise the user lands on an empty state
  with a "Generate Summary" button (`EmptyStateSummary.tsx`). There's a lot of conditional
  branching (gemma3:1b fallback, DB-as-source-of-truth checks) before anything appears.
- During generation: a generic centered spinner ("Generating AI Summary…"), no streaming, no
  progress, no time estimate. Summary status is **polled every 5 s** (`SidebarProvider`
  `startSummaryPolling`, up to ~16 min) rather than event-driven.

### Post-meeting view (`meeting-details/page-content.tsx`)
- Layout (post-0003 pivot): summary as the main pane (`SummaryPanel`) + a right rail with
  **Transcript / My Notes** tabs (`Tabs` from `ui/tabs`; notes via `NotepadPanel meetingId=…`).
  This is the strongest screen in the app.
- But the header is hollow: the `EditableTitle` is **commented out** (`SummaryPanel.tsx:259`),
  so on this screen you **cannot see or edit the meeting title** — it only lives in the sidebar.
  No date, no duration, no participants, no source. Two dense button-groups
  (`SummaryGeneratorButtonGroup` + `SummaryUpdaterButtonGroup`) appear only when a summary
  exists, jumping the layout. A `summaryResponse` legacy panel is `fixed bottom-0` over the
  whole window (`SummaryPanel.tsx:369`) — latent overlap bug.
- "Find in summary" is a button that `console.log`s a TODO (`SummaryPanel.tsx:298`).

### Finding a past meeting
- Only via the sidebar's flat "Meeting Notes" list (titles only) or the search box, which calls
  `api_search_transcripts` (transcript `LIKE`, per ROADMAP Phase 4 — no FTS yet) and inlines a
  yellow "Match:" snippet under matching rows. There is **no list/table view, no grouping by
  date, no filters, no sort, no at-a-glance metadata**. With dozens of meetings named
  `Meeting 23_06_26_…`, this list is unusable at scale.

### Onboarding (`components/onboarding/`)
- A 4-step flow (Welcome → Setup/DB → model download → permissions, macOS). Functional and
  reasonably clean; not the priority. Note it still says "Meetily" in comments
  (`OnboardingFlow.tsx`) — covered by `specs/0006`.

### What's meetily's vs ours
- **Inherited from meetily:** the entire shell, sidebar, recording screen layout, summary
  panel + button-groups, templates, model settings, transcript recovery, tray, onboarding.
- **Ours (added):** the live `NotepadPanel`, the `NotesDraftContext` plumbing, the right-rail
  **My Notes** tab on meeting-details, and notes-aware summary generation (`specs/0003`).
  Everything else is upstream UI we have not yet made our own.

---

## Critique against the Granola bar (for the all-day-Zoom user)

Granola's craft for this user reduces to: **glanceable, keyboard-first, low-friction, calm.**
Where Vinyl falls short:

1. **No "command-key to record."** The defining power-user action — start capturing the meeting
   that just started — requires reaching for the mouse and clicking a pill/sidebar/tray.
   Granola users hit a shortcut. We register *zero* shortcuts. (Highest-impact gap.)
2. **No home that respects the user's day.** `/` is two blank editors. There's no "here are
   today's / recent meetings, here's the one you're about to join." The user's mental model is
   *meetings*, the app's model is *a recorder*.
3. **Meetings aren't glanceable.** Rows are bare title strings (`Meeting 23_06_26_14_…`), no
   date/time/duration/preview. Granola shows a scannable, date-grouped list with human titles
   and a one-line gist. Finding "the pricing call from this morning" is effectively impossible
   here without remembering the timestamp.
4. **Auto-naming is machine-first.** `Meeting DD_MM_YY_HH_MM_SS` is the opposite of Granola's
   human titles (derived from calendar event / first line of notes / summary). Renaming is a
   buried hover-affordance in the sidebar.
5. **Title/identity invisible where it matters.** On the post-meeting screen the title field is
   commented out — you literally can't see what meeting you're looking at without the sidebar.
6. **State & feedback are coarse.** Spinners instead of progress/streaming; 5-second polling
   instead of events; late-binding model-readiness errors; layout that jumps as button-groups
   appear. Granola feels instantaneous and stable; this feels like it's thinking.
7. **Information density & hierarchy are off.** Centered button-groups, full-width pills, a
   `w-2/3 max-w-[750px]` centered column inside each pane, lots of empty gutters. It reads like
   a demo, not a dense daily tool. Typography is the default `Source_Sans_3` with little
   typographic hierarchy between title/section/body.
8. **Notes durability is a trust problem.** "I took notes and they vanished" (the original 0003
   bug) is the cardinal sin for this user. The in-memory-during-recording design still risks
   loss on crash.
9. **Empty states are generic.** `EmptyStateSummary` ("No Summary Generated Yet") and the blank
   recording screen don't teach the next action or reflect the user's context.
10. **Keyboard navigation of the list is absent.** No j/k, no ⌘K palette, no type-to-filter.

---

## Highest-impact pain points (ranked for this user)

1. **Manual, mouse-bound, slow start** — no global shortcut; meeting starts before you're
   recording.
2. **No glanceable meeting list / home** — can't see the day; can't find past meetings.
3. **Machine-named, un-scannable meetings** — titles + missing metadata make the archive noise.
4. **Notes durability / discoverability** — in-memory-until-stop risk; notes hidden behind a
   right-rail tab.
5. **Weak feedback during stop→summary** — spinners + polling + late errors.
6. **Identity invisible post-meeting** — no title/date/duration header.
7. **No keyboard-first operation** anywhere in the app.

---

## Goals

- **Seconds to capture:** a global shortcut and a one-keystroke start that works whether or not
  the window is focused, with a human-editable title from the first moment.
- **A glanceable home** organized around *meetings and the day*, not the recorder.
- **A scannable archive:** date-grouped list/table with human titles + metadata + one-line gist,
  fast filter, keyboard navigation, and a ⌘K command palette.
- **Trustworthy notes:** persist from keystroke one; notes always visible and recoverable.
- **Calm, dense, hierarchical craft:** real typographic hierarchy, stable layouts, progress not
  spinners, event-driven status.
- **Keyboard-first throughout.** Mouse optional.
- Leave clean **hooks for calendar/Zoom auto-detection** (`specs/0008`) without designing it here.

## Non-goals

- Designing the calendar/Zoom integration itself (`specs/0008`) — only leave seams.
- Diarization UI (Phase 3) — reserve space for speaker labels; don't build them.
- Full-text search engine work (FTS5, Phase 4) — design the search/filter UI; backend later.
- Rebrand/data-dir (`specs/0002`) and de-meetily strings (`specs/0006`) — tracked separately.
- A visual design system / theming overhaul beyond the tokens needed for hierarchy.

---

## Approach (recommended)

**Re-anchor the app on "meetings + the day," make capture keyboard-instant, and make every
screen glanceable** — done as an *incremental* migration on the existing Tauri/React shell, not
a rewrite. Reuse what's good (the meeting-details two-pane, the BlockNote editor, the notes-aware
summary, templates) and replace what's recorder-centric (the blank home, the bare sidebar list,
the missing header).

Rationale over alternatives:
- *Full rewrite* — rejected; ~44k LOC of working audio/STT/summary plumbing and a shipped 0003
  flow. Risk/effort dwarfs the craft payoff. We change surfaces, not foundations.
- *Pure CSS polish pass* — rejected as insufficient; the IA itself (home = recorder, no list
  view, no shortcuts) is the problem, not just spacing.
- The chosen path lets Brian ship the **quick wins** (shortcut, human titles, list metadata,
  header) in days while the **larger redesign** (home dashboard, command palette, recording HUD)
  lands behind them.

---

## Design

### Information architecture (target)

```
Vinyl
├─ Home / "Today"              ← NEW default route; meeting list + day + quick-start
│   ├─ (later) Up next from calendar      ← hook for specs/0008
│   ├─ In progress (if recording)
│   └─ Recent meetings (date-grouped, scannable)
├─ Meeting view  /meeting?id=…  ← unify live + post-meeting (one screen, two modes)
│   ├─ mode: live   → Notes | Transcript (notes primary), recording HUD
│   └─ mode: review → Summary | Transcript | Notes, identity header
├─ Search / Command palette (⌘K)  ← overlay, available everywhere
└─ Settings
```

Key IA moves:
- **Home becomes a meeting list/dashboard**, not the recorder. Recording happens *into* a
  meeting view.
- **Unify the live and post-meeting screens** into one `/meeting?id=…` with a `mode`. Today
  they're two routes (`/` and `/meeting-details`) with duplicated panels (two `TranscriptPanel`,
  two `NotepadPanel` usages) — collapsing them removes the notes-loss seam and the duplicate
  code, and means the title/identity header exists in both modes.
- **Retire** the dead `/notes/[id]` branch.

### Key screens + wireframes

#### 1. Home / Today (new default)

```
┌───────────────────────────────────────────────────────────────────────────┐
│  Vinyl                                          [⌘K Search…]      [● Record]│  ← Record = ⌘⇧R, also global
├───────────────┬───────────────────────────────────────────────────────────┤
│  ◉ Today      │  Today · Tuesday, Jun 23                                    │
│  ▸ This week  │  ┌─────────────────────────────────────────────────────┐   │
│  ▸ Earlier    │  │ ● Recording · Pricing sync          12:58 · 04:12 ⏺ │   │ ← in-progress card
│               │  └─────────────────────────────────────────────────────┘   │
│  ⚙ Settings   │  ┌─────────────────────────────────────────────────────┐   │
│               │  │ Design review            10:00–10:45 · 45m · Zoom    │   │
│               │  │ "Agreed to ship the new editor behind a flag…"       │   │ ← one-line gist
│               │  └─────────────────────────────────────────────────────┘   │
│               │  ┌─────────────────────────────────────────────────────┐   │
│               │  │ 1:1 with Sam             09:00–09:30 · 28m           │   │
│               │  │ "Q3 goals; Sam to draft OKRs by Fri"                 │   │
│               │  └─────────────────────────────────────────────────────┘   │
│               │                                                             │
│               │  (later: "Up next — Standup at 1:00, [Record this] ")  ◄── specs/0008 hook
└───────────────┴───────────────────────────────────────────────────────────┘
   j/k or ↑/↓ move · Enter open · ⌘⇧R record · / filter · ⌘K palette
```

- Cards show **human title, date/time, duration, source badge, one-line gist** (first
  summary key-point or first note line). Grouped by Today / This week / Earlier.
- The in-progress card is always pinned at top while recording; click/Enter returns to the live
  meeting view (so you never "lose" your meeting between screens).
- The "Up next" slot is a **placeholder hook** for calendar (`specs/0008`); ships empty/hidden.

#### 2. Meeting view — live mode (`/meeting?id=…`, recording)

```
┌───────────────────────────────────────────────────────────────────────────┐
│  ‹ Today    Pricing sync ✎              ● REC 04:12  ▮▮▯ ▮  [⏸] [■ Stop]    │  ← editable title + HUD
├───────────────────────────────────┬───────────────────────────────────────┤
│  NOTES  (primary, focused)         │  TRANSCRIPT            [Copy] [live ▾] │
│                                    │                                       │
│  - ask about volume discount       │  10:01  …so on the enterprise tier…   │
│  - they want SSO ←                 │  10:01  we'd need SSO before signing  │
│  |                                 │  10:02  let's target end of quarter   │
│                                    │  ▌ (live, auto-scrolling)             │
│  "/" for commands · autosaved ✓    │  (speaker labels reserved — Phase 3)  │
└───────────────────────────────────┴───────────────────────────────────────┘
   Tab toggles focus · ⌘S not needed (autosave) · Esc → Today · ■ = ⌘⇧R again
```

- **Notes are primary** (larger pane, focused on entry) — matches Granola and our 0003 thesis;
  transcript is the reference rail.
- **Recording HUD** in the header: elapsed timer, live level meter (`AudioLevelMeter` exists),
  pause/stop. Replaces the floating pill (which collides with content and the sidebar offset).
- **Title is editable inline from second one** (no machine name shown to the user).
- **Autosave indicator** ("autosaved ✓") makes durability visible — addresses the trust gap.
  (Implementation should persist to a real meeting row at *start*, not stop — see Risks.)

#### 3. Meeting view — review mode (after stop)

```
┌───────────────────────────────────────────────────────────────────────────┐
│  ‹ Today   Pricing sync ✎     Jun 23 · 12:58 · 14m · Zoom   [Copy][Share▾] │  ← identity header (real, not commented out)
├───────────────────────────────────────────────────┬───────────────────────┤
│  SUMMARY                            [Template ▾]    │  [Transcript][Notes]  │
│  ▌ generating… ███████░░ 70%  (streaming in)       │                       │
│                                                     │  10:01 …enterprise…   │
│  ## Decisions                                       │  10:01 …need SSO…     │
│  - Ship behind a flag                               │                       │
│  ## Action items                                    │  (Notes tab = your    │
│  - Sam: draft pricing doc (Fri)                     │   raw notes, editable)│
│                                                     │                       │
│  [Regenerate ▾]                                     │                       │
└───────────────────────────────────────────────────┴───────────────────────┘
```

- Keeps the strong two-pane from 0003 but **adds the identity header** (title/date/duration/
  source) and **uncomments the editable title**.
- **Streaming + progress bar** instead of a blank spinner; status event-driven, not 5 s polling.
- Consistent, non-jumping toolbar (template / copy / regenerate) that's present in all states.

#### 4. Command palette (⌘K) — overlay, everywhere

```
┌──────────────────────────────────────────────┐
│ ⌘K  > pric|                                   │
├──────────────────────────────────────────────┤
│ ▶ Start recording                      ⌘⇧R    │
│ ── Meetings ─────────────────────────────────│
│  Pricing sync           today · 12:58         │
│  Q2 pricing review      Jun 11                │
│ ── Actions ──────────────────────────────────│
│  New blank note                               │
│  Open settings                          ⌘,    │
└──────────────────────────────────────────────┘
```

- One surface to start recording, jump to any meeting, and run actions — the keyboard-first
  backbone for the all-day user. Reuses `api_get_meetings` now; upgrades to FTS in Phase 4.

### Core flows (target)

- **Capture:** global shortcut (works unfocused) → recording starts immediately into a new
  meeting view in live mode, focus in the notes pane, title editable, persisted at start.
  No model-readiness surprise (pre-warm + a calm inline banner if not ready, not a blocking
  modal at start). Compare today: mouse → pill/sidebar/tray → blank editors → notes in memory.
- **Review/find:** stop → review mode with streaming summary → back to Today (Esc) → scannable
  list → ⌘K or j/k to the meeting → done.

### Data / IPC implications (no schema change required for the redesign core)

- **Reuse** `api_get_meetings` but extend the returned shape to include `created_at`,
  `duration` (derivable from transcript span / audio), and a `gist` (first summary key-point or
  first note line) so the list/cards are glanceable without N extra calls. The `meetings` table,
  `meeting_notes`, and `summary_processes` all already exist; this is a query/serialization
  change in `database/repositories/` + the `api_get_meetings` command, not a migration.
- **Human auto-title:** keep `Meeting …timestamp` only as an internal fallback; derive a display
  title from the calendar event (`specs/0008`) or the first note/summary line. No schema change
  (`title` column already exists).
- **Global shortcut + tray accelerators:** add the Tauri `global-shortcut` plugin and register
  ⌘⇧R (toggle record) in `src-tauri/src/lib.rs`/`tray.rs`; reuse the existing
  `request-recording-toggle` event already wired in `layout.tsx`. **Needs Brian's pick of the
  default chord** (see open questions).
- **Status via events, not polling:** the summary path already emits status; the frontend should
  `listen` instead of `startSummaryPolling`'s 5 s loop (`SidebarProvider`). Backend events exist.
- **Persist meeting at start, not stop:** remove the `intro-call` placeholder window by creating
  the meeting row when recording starts, so notes/transcript autosave to a real id throughout
  (eliminates the `NotesDraftContext` flush dance and the crash-loses-notes risk). This is the
  one change with backend lifecycle impact — flag for Brian.

### UI component impact (frontend/src/)

- **New:** `app/(home)/page.tsx` meeting-list/dashboard; `components/MeetingList/*` (card/row,
  date grouping); `components/CommandPalette/*` (⌘K); `components/RecordingHUD/*` (header timer +
  level + controls); a `components/meeting/MeetingHeader` (identity header, replacing the
  commented-out `EditableTitle`).
- **Refactor/merge:** unify `app/page.tsx` (live) and `app/meeting-details/*` into a single
  `app/meeting/*` with a `mode`; collapse the duplicate `TranscriptPanel`/`NotepadPanel` usages.
- **Replace:** the recorder-centric `Sidebar` list with a slimmer nav (Today / week / Earlier /
  Settings); the floating `RecordingControls` pill with the in-header HUD.
- **Tune:** typography/spacing tokens in `globals.css` for real hierarchy; remove the per-pane
  `w-2/3 max-w-[750px]` centering in favor of denser, full-width content with comfortable
  measure on text blocks only.
- **Fix:** uncomment + wire `SummaryPanel` title; remove the `fixed bottom-0` legacy
  `summaryResponse` panel; implement or remove the "Find in summary" TODO.

---

## Prioritization

Effort: **S** ≈ <1 day, **M** ≈ 1–3 days, **L** ≈ multi-day. Impact for the all-day-Zoom user.

### Quick wins — high craft-per-effort

| # | Change | Effort | Impact | Notes |
|---|---|---|---|---|
| Q1 | **Global record shortcut (⌘⇧R)** + tray accelerators | S–M | ★★★★★ | Tauri `global-shortcut`; reuse `request-recording-toggle`. Brian picks chord. |
| Q2 | **Meeting list metadata + date grouping** (date/time/duration/gist on rows) | M | ★★★★★ | Extend `api_get_meetings` shape; render scannable rows. Unblocks "find it later." |
| Q3 | **Human display titles** (derive from first note/summary line; hide machine name) | S | ★★★★ | No schema change; fallback to timestamp internally. |
| Q4 | **Identity header on meeting-details** (uncomment title; add date/duration/source) | S | ★★★★ | `SummaryPanel.tsx:259` already stubbed. |
| Q5 | **Autosave indicator + persist-at-start** (visible "saved ✓"; create row on start) | M | ★★★★ | Kills the notes-loss trust gap; removes `intro-call` placeholder dance. |
| Q6 | **Recording HUD: elapsed timer + level meter** in header (drop floating pill) | S–M | ★★★ | `AudioLevelMeter` exists; fixes pill/sidebar collision. |
| Q7 | **Event-driven summary status + streaming/progress** (retire 5 s polling) | M | ★★★ | Backend events exist; replaces blank spinner. |
| Q8 | **Typographic hierarchy + spacing pass** (titles/sections/body; denser gutters) | S–M | ★★★ | Tokens in `globals.css`; biggest "feels crafted" lever per hour. |
| Q9 | **Remove dead/legacy UI** (`/notes/[id]` branch, `fixed bottom-0` summary panel, Find-TODO, hard-coded `v0.4.0`) | S | ★★ | Reduces confusion/jank. |

### Larger redesign

| # | Change | Effort | Impact | Notes |
|---|---|---|---|---|
| L1 | **Home / Today dashboard** (new default route; in-progress + recent, day-grouped) | L | ★★★★★ | Re-anchors IA on meetings; needs Q2/Q3 first. |
| L2 | **⌘K command palette** (start record / jump to meeting / actions) | M–L | ★★★★ | Keyboard-first backbone; reuse meetings query, FTS later. |
| L3 | **Unify live + post-meeting into one `/meeting` view with modes** | L | ★★★★ | Removes notes-loss seam + duplicate panels; enables header in both modes. |
| L4 | **Keyboard navigation everywhere** (j/k list nav, Tab focus toggle, Esc, ⌘,) | M | ★★★ | Layered onto L1/L3. |
| L5 | **Slim, meeting-first sidebar/nav** (Today/week/Earlier; retire recorder-centric list) | M | ★★★ | Pairs with L1. |
| L6 | **Calendar/Zoom hooks wired** ("Up next", "Record this", auto-title from event) | M | — | Design only here; build in `specs/0008`. |

**Suggested path:** ship Q1→Q9 as a "craft sprint" (instantly better daily ergonomics with low
risk), then L1+L2+L3 as the "Granola-grade" redesign, then L4–L6.

### Needs Brian's decision
- **Record shortcut chord** (⌘⇧R? ⌥Space? something that won't clash with Zoom's own hotkeys).
- **Persist-at-start** (Q5/L3): OK to create a real meeting row when recording starts? It's the
  cleanest fix but touches the recording lifecycle and the recovery flow.
- **Default landing route** becoming Today (L1) vs. keeping the recorder as `/`.
- **How aggressive** on the typography/density pass before it risks the upstream-merge surface.

---

## Acceptance criteria (for the eventual implementation, not this doc)

This spec is "done" when Brian has reviewed it and chosen a path. The *implementation* specs it
spawns must each, per `/CLAUDE.md` Definition of Done:
- `cargo check` + `clippy` clean; `pnpm lint`/`tsc` clean for touched files; app launches via
  `./clean_run.sh`; record → live transcript → notes → summary smoke still works.
- Measurable UX targets to design implementation specs against:
  - **Time-to-recording** from "meeting starting" to "capturing + cursor in notes" ≤ **2 s**
    via shortcut, window unfocused.
  - A user can **find a specific past meeting** by date/title gist in ≤ **2 actions** (⌘K or
    scan the day-grouped list) without remembering a timestamp.
  - **Zero notes loss** across a forced crash mid-meeting.
  - No layout jump between "no summary / generating / summary" states.

## Risks / open questions

- **Upstream merge surface:** large UI refactors (L1/L3/L5) diverge hard from meetily, raising
  cherry-pick cost for their audio fixes. Mitigate by keeping changes in *our* `components/` and
  routes, leaving the audio/STT/summary Rust core untouched.
- **Persist-at-start lifecycle:** creating a meeting row on start interacts with transcript
  recovery and "discard short/empty recordings." Needs care (auto-delete empty meetings on a
  too-short stop).
- **Shortcut conflicts:** global chords can clash with Zoom/Meet/OS; make it user-configurable.
- **Scope creep into Phase 3/4:** reserve space for diarization + FTS but don't build them here.
- **Open questions** for Brian: the four "Needs Brian's decision" items above, plus — do we want
  a true light/dark theme as part of the craft pass, or hold it? And should "Today" show
  *calendar* events before `specs/0008` lands, or stay recordings-only until then?

## Decisions locked (2026-06-24, Brian)
- **Scope:** go straight to the larger redesign (craft polish folded in), not a separate sprint.
- **Home / landing screen:** a **meeting-list/dashboard** (date-grouped, metadata + 1-line gist,
  with a Record action), replacing the recorder-as-home.
- Defaults adopted from the recommendations: global record shortcut **⌘⇧R**; **persist meeting at
  recording start** (removes the `intro-call` placeholder + `NotesDraftContext` flush + crash-loss
  risk); ⌘K command palette.
- Build incrementally in reviewable slices (UI look/feel needs Brian's eyes per slice). Order:
  **(1) dashboard home + landing route → (2) persist-at-start lifecycle → (3) post-meeting review
  view + identity header → (4) ⌘⇧R + tray accelerators → (5) ⌘K palette → (6) IA/nav cleanup +
  typography/density + dead-UI removal.** Each slice: build → verify (lint/tsc, launch Dev Vinyl)
  → commit → show Brian.

## Verification (of the design, before code)

- Walk the three wireframed screens against the four UX targets above with Brian.
- Confirm each quick win maps to a named file/command (done in Design/Prioritization).
- Spin out the agreed subset into implementation specs (`0009+`), each with its own DoD.

---

### Which sub-agents implement what (when greenlit)
- **frontend-engineer:** L1 home, L2 palette, L3 unified meeting view, HUD, header, typography,
  dead-UI removal, keyboard nav (Q4/Q6/Q8/Q9, L1–L5 UI).
- **rust-core-engineer:** global-shortcut + tray accelerators (Q1), `api_get_meetings` shape +
  gist/duration query (Q2/Q3), event-driven summary status (Q7), persist-at-start lifecycle (Q5).
- **llm-pipeline-engineer:** human auto-title heuristic / gist extraction (Q3), summary
  streaming UX copy.
- **spec-architect:** decompose the greenlit path into `0009+` implementation specs; ADR for
  "Home = Today (IA re-anchor)" and for the global-shortcut dependency if adopted.
