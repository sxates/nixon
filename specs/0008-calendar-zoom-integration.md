# 0008 — Calendar + Zoom integration (alert · one-click join+record · auto end-detect)

- **Status:** Done — shipped in 0.2.0 (calendar + Zoom integration).
- **Owner agent(s):** rust-core-engineer (lead) + frontend-engineer + audio-engineer (detection signal)
- **Roadmap phase:** Phase 5 (Polish — "calendar / auto Zoom-meeting detection"), but P1 below is
  small enough to pull forward.

## Context / Problem
The target user lives in back-to-back Zoom meetings all day. Today Vinyl requires the user to
manually click record at the top of every call, and to remember to go process the meeting after.
For someone with 6–10 meetings/day this is high-friction and easy to forget — they end up with
no transcript/notes for calls they didn't manually capture.

We want Vinyl to behave like a personal meeting concierge:
1. **Alert** when it's time to start a meeting (from the calendar).
2. **One button** that *joins the Zoom call and starts recording* together.
3. **Auto-detect meeting end** → kick off the notes-aware summary (`specs/0003`).
4. **Catch manually-started Zoom calls** Vinyl didn't launch, and offer/auto-start recording so
   nothing is missed.

**The constraint that shapes everything (`CLAUDE.md`):** Vinyl is local-first and privacy-first.
Meeting audio/transcripts/notes never leave the machine; the only sanctioned outbound traffic is
to a *user-chosen* LLM. A calendar integration must not quietly become a reason to ship the user's
schedule to a cloud service. This pushes us hard toward **on-device** signals.

### Grounding (verified in this repo)
- **Notifications already exist and already model meetings.** `notifications/types.rs` defines
  `NotificationType::MeetingReminder(u64)` + a `meeting_reminder(minutes_until, title)` builder;
  `notifications/manager.rs` has `show_meeting_reminder(...)` gated on
  `notification_preferences.show_meeting_reminders` and a `meeting_reminder_minutes: Vec<u64>`
  setting. **There is no calendar source feeding it** — that's exactly the gap. We reuse this path.
- **Recording start/stop is well-factored.** Frontend: `hooks/useRecordingStart.ts`
  (`recordingService.startRecordingWithDevices(mic, system, title)`; already supports an
  `autoStartRecording` sessionStorage flag and a `start-recording-from-sidebar` window event) and
  `hooks/useRecordingStop.ts` (drives transcription-wait → save → **notes-aware summary** auto-gen).
  Stop is exposed on `window.handleRecordingStop(callApi)` for Rust callbacks.
- **We already capture system audio via a Core Audio process tap** (`audio/capture/core_audio.rs`,
  `audio/capture/system.rs`) — *no BlackHole*, requires Screen-Recording permission (already
  requested). This is a meeting-activity signal we already have rights to.
- **Backend wiring pattern:** `lib.rs` `run()` `.setup(...)` spawns background tasks via
  `tauri::async_runtime::spawn` and manages shared state via `.manage(...)`; the
  `NotificationManager` is initialized in such a spawned task. A calendar/Zoom monitor fits the
  same mold.
- **macOS config:** `tauri.conf.json` has `macOS.entitlements = "entitlements.plist"`,
  `macOSPrivateApi: true`, and `tauri-plugin-single-instance` is present (required so a
  `zoommtg://`/our-own deep link re-focuses the running app instead of launching a second one).
  No deep-link / EventKit / calendar deps yet.

## Goals
- On-device alert (notification) ahead of each calendar meeting that has a video-call link.
- A meeting "tray"/list that, per upcoming Zoom meeting, offers **Join + Record** (one click:
  open the Zoom call *and* start recording with the meeting's title).
- **End detection** that reliably fires the existing stop→save→notes-aware-summary flow when the
  Zoom meeting actually ends (window closed, call left), not just when audio briefly goes quiet.
- **Manual-call detection:** when the user joins a Zoom meeting Vinyl didn't launch, prompt (or
  auto-start, per setting) recording, titled from the calendar event if one matches the time.
- Keep the whole thing **off by default / opt-in**, and make the calendar source **local**.

## Non-goals
- Zoom *content* APIs (server-to-server OAuth, meeting SDK, in-client active-speaker) — out of
  scope; they're cloud/account-coupled and unnecessary for record+transcribe (we capture audio at
  the OS level). Diarization is `specs/0003`/Phase 3, not here.
- Auto-*joining* a meeting with no user action (we open Zoom on a click; we don't silently dial in).
- Teams / Google Meet / Webex first-class support in v1 (the architecture is provider-pluggable;
  v1 ships **Zoom** detection + **generic calendar-link** alerting. Meet/Teams join links still get
  *alerts* via the calendar; only the process-level detection is Zoom-specific in v1).
- Writing to the calendar (creating events). Read-only.
- Windows/Linux Zoom detection (macOS-only, like the rest of the audio stack).

## Approach (decisions up front)

### Calendar source: **macOS EventKit, not Google Calendar API.** ✅ recommendation
Answering Brian's explicit question — **No, we do not need the Google Calendar API.** EventKit
(Apple's calendar framework) reads **all accounts the user has already added to macOS Calendar** —
iCloud, **Google**, Exchange, CalDAV, subscribed — through one local API, with one OS permission
prompt, and **zero network calls from Vinyl**. That is the privacy-correct, lower-effort choice and
it directly serves the all-day-Zoom user who already has their work Google/Exchange calendar in
macOS.

| | **EventKit (recommended)** | Google Calendar API |
|---|---|---|
| Privacy | 100% on-device; no Vinyl network egress | OAuth + HTTPS to Google; tokens stored; egress of schedule |
| Account coverage | Google + Exchange + iCloud + CalDAV (whatever's in Calendar.app) | Google only; per-account OAuth |
| Setup for user | one macOS permission prompt | OAuth consent screen, Google Cloud project, client secret, refresh tokens |
| Our effort | one permission + read predicate | OAuth client, token refresh, secret management, Google verification review |
| Privacy-ethos fit | matches `CLAUDE.md` | contradicts "nothing leaves the machine" by default |
| Zoom link extraction | parse `location`/`notes`/`url` of `EKEvent` (see below) | structured `conferenceData.entryPoints` (cleaner data) |

The one Google-API advantage is *structured* conference data (`conferenceData.entryPoints[].uri`)
vs. EventKit where we regex the Zoom URL out of `location`/`url`/`notes`. That's a parsing nicety,
not worth the privacy/effort cost. **If** a user has a Google calendar *not* mirrored into macOS
Calendar, the fix is "add it to Calendar.app," not "build a cloud OAuth client." We can keep a
pluggable `CalendarSource` trait so a Google source could be added later behind an explicit opt-in,
but it is **not** in scope.

Rust binding: **`objc2-event-kit`** (v0.3.x, part of the maintained `madsmtm/objc2` family;
provides `EKEventStore`, `requestFullAccessToEvents`/access handlers, event predicates, and
`EKEvent` `startDate/endDate/title/location/notes/url`). Requires the macOS 14+
`requestFullAccessToEvents` authorization and an `NSCalendarsFullAccessUsageDescription` (older:
`NSCalendarsUsageDescription`) string in the Info.plist/entitlements. Read-only access.

### Zoom detection: **primary = `CptHost` process presence; fallback/confirmation = our own
system-audio activity.** ✅ recommendation
The most reliable, permission-free signal that a Zoom *meeting* (not just the app) is live on macOS
is the helper process **`CptHost`**, which Zoom spawns only while in a meeting and tears down on
leave (widely used in MDM/"don't be a jerk" meeting-status scripts). Detected with a plain process
scan — **no extra permission** (we can poll the process list from Rust; no Accessibility, no SDK).

- **Primary — `CptHost` present ⇒ in a Zoom meeting; `CptHost` gone ⇒ meeting ended.** This gives
  us *both* manual-start detection (capability 4) and end detection (capability 3) from one signal.
- **Confirmation/fallback — Vinyl's existing system-audio tap.** We already capture and VAD system
  audio (`audio/pipeline.rs`, `audio/vad.rs`). Sustained system-audio activity is corroborating
  evidence (and the only signal if Zoom renames `CptHost` in a future version). We use it to avoid
  false "ended" on a brief process blip, and as a generic "a call is happening" backstop for other
  apps. We do **not** rely on it *alone* to declare end (music/videos would false-positive).
- **Launch/join — `zoommtg://` URL scheme.** `open "zoommtg://zoom.us/join?action=join&confno=<id>&pwd=<hash>"`
  launches the desktop client straight into the meeting. We extract `confno`/`pwd` from the
  calendar event's Zoom URL. (`zoomus://` is the legacy alias.)

Rejected as primary: **Accessibility API** window inspection (needs the heavyweight Accessibility
permission and is brittle across Zoom UI changes); **Zoom Meeting SDK / local API** (account/cloud
coupling, large dep, overkill — we capture audio at the OS layer already); **menu-bar scraping**
(Accessibility again). We note Accessibility as a *possible* future enhancement for richer state
(e.g., mute status) but it's out of scope.

> Robustness note: `CptHost` is an undocumented implementation detail. We isolate it behind a
> `ZoomDetector` so the process name is a single constant, and corroborate with audio so a rename
> degrades to "audio-based detection" rather than total failure. We add a settings escape hatch
> (disable auto-detect) and log the detector decision for debugging.

## Design

### Component map (where each piece lives)
- **Rust — new module `frontend/src-tauri/src/meeting_concierge/`** (a background monitor + IPC):
  - `calendar/` — `CalendarSource` trait + `eventkit.rs` (EventKit reader, `objc2-event-kit`),
    `event.rs` (`CalendarEvent { id, title, start, end, join_url, provider }`), `zoom_link.rs`
    (regex extraction of Zoom/Meet/Teams URLs from `location`/`url`/`notes`).
  - `detector/` — `ZoomDetector` (poll `CptHost`), `audio_signal.rs` (reads the existing system-audio
    activity state), and a combined `MeetingDetector` that fuses the two.
  - `monitor.rs` — the background loop (one `tauri::async_runtime::spawn` task started in
    `lib.rs::run().setup`), holding the state machine (below); emits Tauri events + drives the
    notification manager.
  - `state.rs` — the `ConciergeState` enum + transitions; managed via `.manage(...)`.
  - `commands.rs` — the Tauri commands (below), registered in `lib.rs`.
- **Frontend — `frontend/src/`:**
  - `components/MeetingConcierge/` — an "Upcoming meetings" panel/tray (next-meeting card with a
    **Join + Record** button) + a toast/banner for "Zoom meeting detected — record?".
  - `hooks/useMeetingConcierge.ts` — subscribes to concierge events; calls join+record.
  - Reuse `useRecordingStart` (sessionStorage `autoStartRecording` / `start-recording-from-sidebar`)
    and `window.handleRecordingStop` for auto-stop. Reuse existing notification settings UI; add a
    "Calendar & auto-record" settings section.

### Meeting lifecycle / state machine
The monitor runs a single state machine per "current meeting context." Calendar events arm
*alerts*; the **detector** (process + audio) is the source of truth for *recording* lifecycle, so we
behave correctly even with no calendar entry (capability 4) and never depend on calendar accuracy
for start/stop.

```
                         calendar: event within ALERT_LEAD (e.g. 1–5 min)
            ┌──────────────────────────────────────────────────────────────┐
            │                                                                ▼
   ┌────────────────┐   user clicks "Join+Record"        ┌───────────────────────────┐
   │      IDLE       │ ─────────────────────────────────► │  JOINING (open zoommtg://) │
   │ (poll cal+proc) │                                    └─────────────┬─────────────┘
   └───────┬────────┘                                                   │ CptHost appears
           │  CptHost appears  (manual Zoom start, no Vinyl join)        │ (confirm w/ audio)
           │  ───────────────► PROMPT_RECORD ──user yes / auto──┐        │
           │                        │ user no                   ▼        ▼
           │                        └───────────────► ┌────────────────────────────┐
           │                                          │   RECORDING  (start_rec)    │
           │                                          │  title = matched cal event  │
           │                                          └─────────────┬──────────────┘
           │                                                        │ END signal:
           │                                                        │  CptHost gone for
           │                                                        │  END_GRACE (e.g. 20s)
           │                                                        │  AND system audio idle
           │                                                        ▼
           │                                          ┌────────────────────────────┐
           │                                          │  ENDING → stop+save+        │
           │                                          │  notes-aware summary (0003) │
           │                                          └─────────────┬──────────────┘
           └──────────────────────────────────────────────────────►│ back to IDLE
                                                                     ▼
                                                                   IDLE
```

Key rules:
- **Alert (cap. 1):** for each calendar event with a join URL, when `now` enters
  `[start - lead]`, fire `show_meeting_reminder(minutes_until, title)` (existing path) **once**
  (dedupe by event id + lead bucket). Lead minutes come from existing `meeting_reminder_minutes`.
- **Join+Record (cap. 2):** open `zoommtg://...` then set `autoStartRecording` + meeting title and
  trigger start (reuse `useRecordingStart`). Title = calendar event title.
- **Start detection (cap. 4):** `CptHost` appears while `IDLE` and Vinyl isn't already recording ⇒
  `PROMPT_RECORD` (toast) or auto-start (setting `auto_record_detected_meetings`). Title resolved by
  matching current time against calendar events (±N min); else timestamp title.
- **End detection (cap. 3):** declared only when `CptHost` is **absent for `END_GRACE`** *and*
  system audio is idle — debounced to survive process blips / screen-share helper churn. On end →
  call `window.handleRecordingStop(true)`, which already runs transcription-wait → save →
  **notes-aware summary** (`specs/0003`). No new summary trigger needed.
- **Audio-only fallback:** if `CptHost` never appears but Vinyl is recording (e.g. user hit record
  manually on a non-Zoom call), end detection falls back to sustained audio-idle + the existing
  manual stop; we do not force-stop on audio alone.

### Data model
Mostly **stateless** — calendar events are read live from EventKit; recording/notes persistence is
unchanged (`meetings`, `meeting_notes`). We do **not** copy the calendar into SQLite (privacy +
freshness). Optional, deferred: a tiny `calendar_event_id TEXT` column on `meetings` (new
forward-only migration) to link a recording back to its calendar event for later "meeting history"
features — **not required for v1**, list it as a P2 nice-to-have. Alert-dedupe + "already handled"
flags live in in-memory monitor state (reset on relaunch is acceptable).

### Tauri IPC (register in `frontend/src-tauri/src/lib.rs`)
Commands (frontend → Rust):
- `concierge_request_calendar_access() -> bool` — trigger EventKit authorization prompt; report
  grant.
- `concierge_get_upcoming_meetings(window_minutes) -> Vec<CalendarEvent>` — read EventKit, filter to
  events with a join URL, return next N.
- `concierge_join_and_record(event_id)` — resolve event, `open` the `zoommtg://` URL, then signal
  the frontend to start recording with the event title (or do the open in Rust and emit a
  `concierge://start-recording` event the frontend's `useMeetingConcierge` consumes).
- `concierge_get_settings()/concierge_set_settings(...)` — enable/disable, alert lead, auto-record
  detected meetings, end-grace seconds. (Could fold lead/enable into existing notification settings;
  keep auto-record + grace here.)
- `concierge_get_state() -> ConciergeState` — for UI/debug.

Events (Rust → frontend):
- `concierge-meeting-detected` `{ provider, matched_event?: CalendarEvent }` → drive the
  "record this call?" prompt / auto-start.
- `concierge-meeting-ended` → frontend calls `window.handleRecordingStop(true)`.
- `concierge-upcoming-changed` → refresh the upcoming panel.
- (Alerts go through the existing notification system, not a new event.)

### Permissions / deps / config
- **New macOS permission:** Calendar (EventKit) — add `NSCalendarsFullAccessUsageDescription`
  (and legacy `NSCalendarsUsageDescription`) to the Info.plist / `entitlements.plist`. One-time
  prompt; new bundle id `com.vinyl.dev` means a fresh prompt.
- **No new permission for Zoom detection** (process scan) or end detection (we already hold
  Screen-Recording for the audio tap).
- **New crates:** `objc2-event-kit` (+ the `objc2`/`objc2-foundation` it pulls). Process scan can
  use `sysinfo` (already common) or a thin `pgrep`-equivalent; prefer `sysinfo` to avoid shelling
  out. URL launch: `tauri-plugin-shell`'s `open`/`std::process::Command open` or
  `tauri-plugin-opener`.
- **Deep-link (only needed for the *reverse* — Vinyl handling a `vinyl://` link):** not required for
  v1; `zoommtg://` is handled by Zoom, we just `open` it. `tauri-plugin-single-instance` already
  present covers re-focus.

### UI
- **Upcoming meetings panel** (in the sidebar or a top banner on `app/page.tsx`): next meeting card
  with countdown + **Join + Record**; list of the day's meetings with join links.
- **Detection toast/banner:** "Zoom meeting detected — start recording?" with
  Record / Ignore / Always (writes the auto-record setting).
- **Settings:** a "Calendar & meetings" section — Connect Calendar (permission), enable concierge,
  alert lead minutes (reuse), auto-record detected meetings, end-grace seconds.

## Tasks (ordered, phased — see Phasing)
**P1 — manual-Zoom detect + auto-record + auto-end (no calendar; highest value/effort ratio)**
1. [ ] **audio-engineer** — expose a cheap "system-audio active" read from the existing pipeline
   (`audio/pipeline.rs`/`vad.rs`) for the detector to query (no new capture).
2. [ ] **rust-core-engineer** — `meeting_concierge/detector/` (`ZoomDetector` via `sysinfo` on
   `CptHost`, fused with the audio signal); `monitor.rs` state machine (IDLE↔PROMPT_RECORD↔RECORDING
   ↔ENDING) sans calendar; spawn in `lib.rs::run().setup`; emit `concierge-meeting-detected` /
   `concierge-meeting-ended`; `concierge_get_state`, settings commands.
3. [ ] **frontend-engineer** — `useMeetingConcierge` + detection toast; wire detected→start via
   existing `autoStartRecording`/`start-recording-from-sidebar`; wire ended→`window.handleRecordingStop`.
   Settings section (enable + auto-record + grace).

**P2 — calendar alerts (read-only EventKit)**
4. [ ] **rust-core-engineer** — `calendar/eventkit.rs` (`objc2-event-kit`), `CalendarSource` trait,
   `zoom_link.rs` URL extraction; `concierge_request_calendar_access`,
   `concierge_get_upcoming_meetings`; Info.plist/entitlements usage strings.
5. [ ] **rust-core-engineer** — feed upcoming events into `monitor.rs`: alert scheduling via
   `show_meeting_reminder` (dedupe), and title-matching for detected meetings.
6. [ ] **frontend-engineer** — Upcoming meetings panel + "Connect Calendar" flow; titles on
   detected recordings.

**P3 — one-click Join + Record**
7. [ ] **rust-core-engineer** — `concierge_join_and_record` (`open zoommtg://...` from extracted
   confno/pwd) + JOINING state.
8. [ ] **frontend-engineer** — **Join + Record** button on the next-meeting card; confirm the join
   transitions JOINING→RECORDING when `CptHost` appears.

**Cross-cutting**
9. [ ] **spec-architect** — ADR `docs/decisions/ADR-0004-calendar-via-eventkit.md` (EventKit over
   Google API; `CptHost` detection contract + fallback). Write alongside P2.

## Effort estimates (rough)
- P1: ~2–3 days (detector + state machine are the meat; reuses all recording plumbing).
- P2: ~3–4 days (EventKit binding + auth + URL parsing + alert dedupe; first time touching `objc2`).
- P3: ~1 day (mostly the URL-open + one state transition + a button).

## Acceptance criteria (testable; tie to Definition of Done in `/CLAUDE.md`)
- **Cap. 1:** With a Google/Exchange calendar in macOS Calendar and an event with a Zoom link
  starting in ≤ lead minutes, Vinyl shows a meeting reminder notification once.
- **Cap. 2:** Clicking **Join + Record** opens the Zoom client into the meeting *and* Vinyl starts
  recording within a few seconds, titled with the event name.
- **Cap. 3:** Leaving/ending the Zoom call (CptHost exits) auto-stops recording and produces the
  notes-aware summary (`specs/0003`) without manual action; a brief process blip does **not**
  trigger a false stop (END_GRACE honored).
- **Cap. 4:** Joining a Zoom meeting Vinyl didn't launch produces a "record this call?" prompt
  (or auto-records if enabled), titled from a matching calendar event when present.
- **Privacy:** with the feature on, Vinyl makes **no** outbound network calls for calendar/Zoom
  (verify with a network monitor); calendar data stays on device.
- **Gate:** `cargo check` + `cargo clippy` clean (`src-tauri`); `pnpm lint` clean; app launches via
  `./clean_run.sh`; record→transcript→summary smoke still works.

## Risks / open questions
- **`CptHost` is undocumented** and could be renamed/removed by Zoom. *Mitigation:* single constant,
  audio corroboration, settings off-switch, logged decisions. **Open:** confirm `CptHost` on the
  user's current Zoom build (and note it isn't present for *phone-only* Zoom or browser-tab Zoom).
- **Browser-based Zoom (no desktop client)** won't spawn `CptHost`; only the audio fallback would
  fire. **Open:** is desktop-client-only acceptable for v1? (Likely yes for this user.)
- **EventKit authorization UX** on macOS 14+ uses `requestFullAccessToEvents`; the `objc2-event-kit`
  async-handler ergonomics from Rust need a quick spike. **Open:** confirm the crate exposes the
  full-access call (older bindings only had the deprecated `requestAccessToEntityType`).
- **Zoom URL extraction variance** — links appear in `location`, `url`, or buried in `notes`
  ("Join Zoom Meeting … https://…/j/<id>?pwd=<hash>"). Need a robust regex + map web URL → `confno`
  (`/j/<id>` → `confno`, `?pwd=` → `pwd`). **Open:** confirm passcode handling (some links use a
  hashed `pwd` that `zoommtg://` accepts; others need the numeric passcode).
- **End-detection grace tuning** — screen-share / breakout transitions may briefly churn helpers;
  END_GRACE + audio must be tuned on real calls. Default ~20s, configurable.
- **DND interaction** — back-to-back-meeting users often run Focus/DND; the existing notification
  manager suppresses non-critical alerts under DND. **Open:** should meeting-start alerts be
  High vs Critical so they survive DND? (Lean: keep High, respect DND, surface in-app banner too.)
- **Double-record guard** — Join+Record then `CptHost`-detect must not start two recordings; the
  state machine + existing `isRecording` guards cover it, but test explicitly.

## Verification
- **P1:** Start a Zoom call manually (no Vinyl) → confirm detect prompt/auto-record; leave the call
  → confirm auto-stop + summary. Toggle Zoom mute / brief screen-share to confirm no false stop.
- **P2:** Add a test event with a Zoom link in macOS Calendar (via a Google account) a few minutes
  out → confirm the reminder fires and the upcoming panel lists it. Run a packet capture to confirm
  no calendar egress.
- **P3:** Click **Join + Record** on the next-meeting card → Zoom opens into the meeting and Vinyl
  records with the event title.
- Run `/check` (cargo check/clippy, pnpm lint, `./clean_run.sh`, record→transcript→summary smoke).

## Sources
- CptHost / Webex / Teams / GoTo meeting-state detection on macOS — brunerd, "Respecting Focus and
  Meeting Status in Your Mac scripts": https://www.brunerd.com/blog/2022/03/07/respecting-focus-and-meeting-status-in-your-mac-scripts-aka-dont-be-a-jerk/
- Zoom meeting monitoring via CptHost — Home Assistant community thread:
  https://community.home-assistant.io/t/zoom-meeting-monitoring-to-ha/246336
- Prior art (ScreenCaptureKit + whisper.cpp local meeting transcription) — meetink:
  https://github.com/sservaes/meetink
- `zoommtg://` URL scheme to launch/join — Zoom Developer Forum:
  https://devforum.zoom.us/t/open-meeting-join-url-in-app-if-installed-else-join-from-browser/30842
  and gist: https://gist.github.com/brycemcd/04092405cbc663ee7ea48b933e40844e
- EventKit reads iCloud/Google/Exchange/CalDAV accounts; Apple docs:
  https://developer.apple.com/documentation/EventKit/accessing-calendar-using-eventkit-and-eventkitui
  and `EKEventStore`: https://developer.apple.com/documentation/eventkit/ekeventstore
- Rust EventKit bindings — `objc2-event-kit`: https://docs.rs/objc2-event-kit/ ;
  alt CLI prior art `eventkit-rs`: https://crates.io/crates/eventkit-rs/0.1.0
- Google Calendar `conferenceData.entryPoints` (the structured alternative we're declining):
  https://developers.google.com/workspace/calendar/api/v3/reference/events
```
