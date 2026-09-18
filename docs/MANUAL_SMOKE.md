# Manual smoke checklist

The parts of Nixon that CI **cannot** cover — real microphone + system-audio capture
(Core Audio tap), audio-capture permission, and live Zoom/Meet/Teams behavior. Run this
before a release (`./release.sh`) and after any change to the recording lifecycle, audio
pipeline, or calendar/Zoom integration.

Automated layers (run first; they gate PRs via `.github/workflows/ci-checks.yml`):
- `cd frontend/src-tauri && cargo test --features metal` (lifecycle/diarization/STT fixtures)
- `cd frontend && pnpm lint && pnpm test` (frontend unit/hook tests)

Test against the **bundled `Nixon.app`** (or `Dev Nixon` from `./dev-nixon.sh`), not the bare
`cargo run` binary — the ad-hoc-signed dev binary keeps losing its Audio-Capture grant, so
the system-audio tap silently returns silence (see `CLAUDE.md` / ADR-0004).

---

## 0. Permissions (first launch on a clean identifier)
- [ ] Microphone permission prompt appears and, once granted, the mic level meter moves.
- [ ] Audio-capture permission prompt appears; after granting + relaunch, **system audio**
      is captured (not just mic).

## 0a. Demo profile (specs/0059)
- [ ] Run `./dev-nixon.sh --demo`. It seeds the DEBUG profile and launches: **Today** shows
      activity, **Meetings** lists **5** meetings and **People** lists **7**. First run
      takes about **4 minutes** (`say`/ffmpeg audio synthesis); the recording folders are
      named `nixon-demo-<id>` and are stable, so re-running `--demo` reuses the cached
      audio and finishes in a few seconds (or use `--no-audio` for fast iteration when
      audio doesn't matter).
- [ ] Meeting 01 ("Product sync — Q4 firmware") already has a summary, action items, notes,
      and playable audio.
- [ ] Meeting 05 ("Vendor call — enclosure supplier") has no summary yet and offers
      "Identify speakers".
- [ ] Run `./dev-nixon.sh --onboarding`. Both simulated model downloads complete in about
      **11 s** each, the Continue button enables once they finish, and completing onboarding
      lands on the main app (not a stuck download step).

## 0b. Screenshots (specs/0060)
- [ ] `cd frontend && pnpm shots` renders every manifest entry, in both themes, into
      `docs/screenshots/headless/` — none come back blank/empty.
- [ ] `pnpm shots:diff` prints `0 changed` on a clean tree.
- [ ] From a terminal with Screen Recording permission, with `./dev-nixon.sh --demo`
      running, `pnpm shots:real` renders every manifest entry — including `record-live`,
      with the level meters actually moving — and none show the "DEV" badge.
- [ ] `?onboardingStep=3` shows the model-download cards actually in progress (not already
      complete, not still queued).

## 1. Core record → review loop (DoD #4)
- [ ] Press **REC** on the transport rail from Today. It navigates to /record and starts; the
      live transcript streams in.
- [ ] While recording, the rail's level ladder responds to real audio.
- [ ] Press **STOP** on the rail. It finishes and lands on the meeting with the full transcript.
- [ ] Generate a summary; it completes and renders.
- [ ] Reopen the meeting later — transcript, summary, and notes are all still there.
- [ ] "Open folder" opens **this** meeting's recording folder (matches its audio).

## 2. Transport rail — persistent across screens (specs/0057 Phase C)
- [ ] The rail is visible and identical on **Today**, **Meetings** and **Settings** (and on
      every other screen); no page content hides behind it and no page scrolls under it.
- [ ] Idle: REC lit, HOLD/STOP disabled, counter dim at `00:00:00`, reels still.
- [ ] Right after pressing REC the **"Starting"** phase is visible (REC lit, HOLD/STOP still
      disabled, counter dim) before it flips to recording.
- [ ] Recording: counter ticks `hh:mm:ss`, reels spin, ladder moves, HOLD and STOP enabled.
- [ ] Navigate to **Meetings** while recording and press **HOLD** there — the counter freezes,
      the reels stop, the rail reads paused. Press HOLD again to resume; the counter continues
      (does not restart).
- [ ] Press **STOP** from a screen other than /record — the recording wraps up and the rail
      returns to idle.
- [ ] Toasts/alerts stack **above** the rail, never underneath it.
- [ ] Everything above still reads correctly in **both themes** (Faceplate light / Deck dark).
- [ ] While recording, navigate to **Meetings**, then click the rail's meeting title — it
      returns to /record. On /record the title is not clickable.
- [ ] Rename the meeting in the record header; the rail's title updates immediately, and so
      does the meetings list in the sidebar.

## 3. VU meters and lamps on /record (specs/0057 §3.2)
- [ ] Two needle meters, **CH1 MIC** and **CH2 SYS**. Speak into the mic with nothing
      playing — only CH1 swings, with real ballistics (fast rise, slow fall), CH2 rests.
      Normal speech should read around **-5 VU** with peaks near 0 (0 VU = -18 dBFS); if
      CH1 barely leaves the -20 stop, the calibration is off — report it.
- [ ] Play audio (a video, a Zoom test call) with your mic muted at the OS — only CH2
      swings. Both together — both move independently; the rail ladder shows the mix.
- [ ] Clap once — the **PEAK** lamp lights and latches briefly, then clears (it latches
      when either channel clips, not just the mix).
- [ ] Mute yourself in **Zoom** — the **MIC GATE** lamp lights while muted and clears on
      unmute (this is the specs/0049 mute gate, now visible).
- [ ] There is no spectrometer strip and no separate recording-controls block; the Record
      header is the control panel.
- [ ] No key glyph touches the lamp bar above it; the legend is centred under the glyph.
- [ ] A lit **REC** key is a red key with cream ink in both themes; a lit **HOLD** is visibly
      amber on Faceplate (light), not brown. REC while on HOLD is dimmed, not red.
- [ ] On the **Deck** (dark) theme, a lit **HOLD** key's glyph and "HOLD" legend are dark ink
      on the amber face, not near-white — no headless screenshot catches this (`record-live`
      is `real_only`), so it needs an eyes-on check.

## 4. Queue panel (deferred processing + background AI)
- [ ] With queued work, the rail's **Queue** shows a non-zero count and its lamp lights.
- [ ] Open the Queue panel — rows show stage text for each deferred meeting and background
      LLM task.
- [ ] **Stop** on a running row cancels it.
- [ ] **Process all** starts the deferred backlog.
- [ ] **Clear finished** removes completed rows.
- [ ] Force a failure (e.g. stop Ollama), then **Retry** on the failed row re-runs it, and
      **Dismiss failures** clears the failed rows from the panel.
- [ ] Closing and reopening the panel keeps the counts consistent with what is actually running.

## 5. Channel strip on a meeting (specs/0057 §3.5)
- [ ] Open a meeting → **Transcript** tab. The speaker list is a channel strip: **CH 1** is
      always you, then CH 2, CH 3, … with talk time and share of talk per channel.
- [ ] The shares are computed over the **full** meeting (not just the first page of
      transcripts) and sum to ~100%.
- [ ] From a channel's name cell: **rename** a speaker, **merge** two speakers, and **assign**
      a speaker to an attendee — each updates the strip and the transcript rows.

## 6. Live-transcript review while recording (WS1.2)
- [ ] While recording, scroll **up** into earlier transcript. New incoming segments must
      **not** yank the view back to the bottom.
- [ ] Scroll back to the bottom — live following resumes.

## 7. Consecutive recordings — no cross-contamination (WS6.7) ⚠️ data-integrity
- [ ] Record meeting A, stop, let it save.
- [ ] Immediately record meeting B, stop.
- [ ] A and B are **two separate meetings**. A shows only A's transcript; B only B's
      (no interwoven text). Each "Open folder" shows its own audio. Attendees don't bleed across.

## 8. Empty / abandoned recording (WS6.1) ⚠️ data-integrity
- [ ] Start a recording and stop it almost immediately (no speech). An empty ad-hoc meeting may
      be discarded — that's fine.
- [ ] But a recording that **captured audio** (even if transcripts are sparse) is **kept**, and
      a meeting started from a calendar event is **never** silently deleted.

## 9. Calendar → Join & Record (WS6.3) ⚠️ data-integrity
- [ ] From a calendar event with a Zoom link, click "Join & Record".
- [ ] The in-progress recording shows the **event title** and its **attendees** (not a generic
      `Meeting <date-time>` with no participants).
- [ ] It links to the calendar event (same logical meeting), and joining the same event twice
      does not create a duplicate.

## 10. Zoom auto-detect / auto-stop (WS6.2)
- [ ] Joining a Zoom call (without clicking record) surfaces the "Zoom meeting detected" prompt.
- [ ] When the Zoom call ends, Nixon wraps up the recording **and** the transport rail returns
      to idle (counter dim, reels stopped, HOLD/STOP disabled).

## 11. Recording state on Home (WS6.4)
- [ ] While a recording is in progress, Home shows it as recording; "happening now" calendar
      rows do **not** still offer "Join & Record".

## 12. Navigating while recording (WS6.5)
- [ ] While recording meeting A, open a **previous** meeting B from the sidebar. It shows B
      (does not redirect you to A). The rail keeps counting A the whole time.

## 13. Speaker identification (WS2.5)
- [ ] Run "Identify speakers" on a meeting. The progress indicator advances past 0% to 100%
      (it must not sit stuck at 0%).

## 14. Sidebar collapse / expand (specs/0057 Task 3)
- [ ] Collapse the sidebar to the icon rail — nav still works (icons only, tooltips on
      hover), and the walnut cheek strip stays visible along the left edge (not painted over
      by a nav row's hover highlight).
- [ ] Expand it back — labels return, the engraved "NIXON" wordmark and the amber index bar
      on the active row are both visible.
- [ ] The collapsed/expanded state survives navigating between screens and a relaunch.

## 15. Reel label + typed transcript (specs/0057 Task 4)
- [ ] Open a meeting. The **reel label** (the paper card beside the title) shows the reel
      ordinal (`REEL 0412`-style, or the blank `REEL ——` handle for an unnumbered meeting),
      date, time, length, source, voices, and deck.
- [ ] Deleting an earlier meeting **renumbers** later reels (ordinals are derived, not
      stored) — confirm this reads as expected rather than as a bug.
- [ ] The transcript body reads as typewriter-on-paper (monospace ink on the paper
      background), not the old chat-bubble styling.

## 16. Today / Meetings tape-log visuals (specs/0057 Task 6)
- [ ] **Today**: the timeline reads as a transport log — each entry a compact block with a
      spine/hover treatment, not a card list.
- [ ] **Meetings** (list view): every row shows a reel tag and a tape-counter-style duration.
      Below the `sm` breakpoint the row still keeps its 5 essential tracks (no silent
      truncation).
- [ ] **Meetings** (month view): month chips render correctly and switching List ⇄ Month
      keeps the current filter/search intact.

## 17. Tray icon states (specs/0057 Task 8)
- [ ] Idle: the tray icon and its menu labels read as plain text (no emoji); the icon's tape
      bar is visible at the menu-bar's actual size (not just in a zoomed screenshot).
- [ ] Start a recording: the tray icon switches to its recording state; stop and confirm it
      returns to idle. Pause (HOLD) shows a distinct paused state.
- [ ] Menu items match the current transport state (no stale "Start recording" while already
      recording, etc.).
- [ ] From any page other than the recorder, choose the tray's **Start recording** — the app
      lands on the recorder and a recording actually starts (it used to route to Today, where
      nothing consumed the start). With no STT model installed the tray must NOT stick on
      "Starting…".

## 18. Onboarding — fresh profile walk (specs/0057 Task 7)
- [ ] Launch with a brand-new identifier (or after clearing the app's data directory) and
      walk the full onboarding flow start to finish: welcome → permissions → model
      download/setup → completion.
- [ ] The download-progress and setup-overview steps render correctly (these re-skinned
      steps have no automated test coverage) — progress advances, errors (if forced) surface
      legibly, and the flow does not get stuck.
- [ ] Permission rows show the correct granted/ungranted state as each permission is
      actually granted mid-flow.

## 19. Settings — ruled rows and Appearance (specs/0057 Task 7)
- [ ] Settings sections render as ruled rows (a hairline between rows), not card grids.
- [ ] Quit and relaunch with **Deck** stored: toasts (trigger any) render dark and Settings →
      Appearance shows Deck selected on the very first paint (no light toast skin, no
      "System" radio checked). Same check after a hard load via the tray's Settings item.
- [ ] Settings → General → Appearance: the theme swatches are reachable and selectable with
      **arrow keys**, not just the mouse.
- [ ] Both themes (**Faceplate** light / **Deck** dark) render every Settings section
      correctly — no low-contrast or clipped text in either.

## 20. Both themes, every screen (specs/0057 Plan 3)
- [ ] Switch between **Faceplate** (light) and **Deck** (dark) and spot-check Today,
      Meetings, a meeting's details (reel label + channel strip + transcript), People,
      Settings, and Onboarding in both — no leftover raw-palette colors, no illegible text,
      no theme-locked component (the reel label's paper look, the walnut sidebar cheek, and
      ruled Settings rows should all still look intentional in dark mode).

---

## 21. In-app update (specs/0058) — run against a throwaway patch release
- [ ] Install the DMG of the version under test; launch; Settings > About shows
      "Not checked yet" then within ~30 s "Up to date · checked just now".
- [ ] Publish a throwaway patch release (`./release.sh patch`, notes "updater smoke").
- [ ] Within ~20 s of relaunching the older build, the sidebar footer shows
      "Downloading X.Y.Z" with a percentage, then "Nixon X.Y.Z ready · Restart"; the tray
      menu gains "Restart to update to X.Y.Z".
- [ ] Start a recording: the Restart key is disabled ("Finish the recording first") and
      the tray item disappears. Stop: both return.
- [ ] Click Restart: the app relaunches, About shows X.Y.Z, meetings and settings are
      intact, `<app-data-dir>/updates/` no longer needs the old tarball (may be deleted).
- [ ] Settings > General > "Download updates automatically" off → no row appears for a
      further release, but About's "Check for updates" still finds and downloads it.

## 22. Speaker corrections and transcript editing (specs/0061 W4-W6)
- [ ] With `./dev-nixon.sh --demo` running, open **Meeting 05** ("Vendor call — enclosure
      supplier") — it's seeded with an "Unknown" speaker and no owner ("You") track at all.
      Reassign its "Unknown" line to **You**: "You" appears as an option even though this
      meeting never diarized an owner, and the reassignment sticks.
- [ ] On any meeting, click a transcript line's pencil, change its text, and save — the
      line shows an `edited` mark and the new wording is findable via search.
- [ ] In Meeting 05's channel strip, click a speaker's row — the transcript filters to
      that speaker and jumps to their first line. Click the same row again to clear the
      filter.

## Release builds (WS8)
- [ ] A production build (`./build-gpu.sh` / `./upgrade-nixon.sh`) launches as **Nixon** —
      window title, menu bar, tray and notifications all say Nixon, and the sidebar shows no
      **"DEV"** badge (that badge belongs to `./dev-nixon.sh` only).

## Notes
Record anything that fails here as a numbered spec item (and, where possible, add a regression
test at the cheapest layer — Rust `tests/` or a Vitest hook test — per `specs/0023`).
