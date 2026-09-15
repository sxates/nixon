# Changelog

All notable changes to **Nixon** are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/); versions follow [SemVer](https://semver.org/).

Nixon is a hard fork of [meetily](https://github.com/Zackriya-Solutions/meeting-minutes)
**v0.4.0** (forked 2026-06-05, MIT — see `LICENSE.md`). It was developed privately as
"Vinyl" through twenty releases before the rename; that history is not tracked in this
repository. Nixon's own version line starts at **0.1.0** and will reach 1.0.0 once the
redesign has been through real use. Git tags are plain `vX.Y.Z`.

> Keep this file updated as part of each change: add an entry under the right heading in
> _Unreleased_ (Added / Changed / Fixed / Removed / Deprecated / Security).

## [Unreleased]

_(nothing yet)_

## [0.2.0] - 2026-09-14

### Changed

- **Canvas feedback pass (owner, 2026-09-15).** Record header: the PEAK / MIC GATE lamps
  are gone (the rail carries both states), the template / mode / participants controls sit
  under the meeting title, and the back control is the same unboxed chevron as meeting
  details. Meeting details: the typed reel-label card is removed — the engraved identity
  line now carries the reel number, date, time, length, source and voice count — and the
  back button hangs in the left gutter at wide widths so the title aligns with the content
  below. Speaker names in the channel strip and people in the participants row are plain
  text with their edit / remove affordances on hover only. The record red is a hotter lamp
  colour in both themes; red text uses a new `record-ink` shade that keeps AA contrast.

- **Settings pages speak one visual language.** Every tab (General, Recordings,
  Transcription, Summary, Templates, Beta) is now built from a shared set of primitives
  in `frontend/src/components/ui/settings.tsx` — an engraved caps section header with a
  one-line purpose, a `rounded-[3px]` group card of ruled rows, and one row shape for
  every setting: name, description, control on the right. This replaces the three
  vocabularies that had grown side by side (the ruled row, the old shadcn
  `p-6 shadow-sm` card with an `h3 text-lg font-semibold`, and ad-hoc bare labels), the
  `u-section-label` engraved style misused as a row label, and the odd-styled
  "Loading…" placeholder boxes. Sections within each tab are ordered the way a new user
  meets them — what is captured, how it behaves, where it is stored, destructive
  cleanup last — and the Transcription tab's emoji (⚡/🏠/☁️ in the model picker) are
  gone; the options now read "Parakeet (recommended)" and "Local Whisper".

## [0.1.0] - 2026-09-14

First release under the Nixon name. Everything that shipped in the private Vinyl line
(0.2.0 – 1.20.0: on-device transcription, diarization with owner-mic attribution, deferred
processing, Zoom mute gate, Google Calendar, summaries and action items, voiceprints) is
carried forward unchanged; the entries below are what is new relative to Vinyl 1.20.0.

### Changed

- **Rebrand: Vinyl → Nixon** (specs/0057). Product name, window/tray/notification titles,
  onboarding and settings copy, script names (`dev-nixon.sh`, `upgrade-nixon.sh`), DMG/app
  bundle names, and the Google OAuth build variables (`NIXON_GOOGLE_CLIENT_ID` /
  `NIXON_GOOGLE_CLIENT_SECRET` in the gitignored `.env.google`). The bundle identifier
  `ai.vinyl.app` is unchanged, so existing data and permissions carry over untouched.
- **Recordings folder**: fresh installs use `~/Movies/nixon-recordings/`. An install that has
  `~/Movies/meetily-recordings/` keeps writing there for as long as that folder exists.
- **Two-channel VU on the record page** (specs/0057 §3.2). The single mixed needle is now
  **CH1 MIC** and **CH2 SYS**, each reading its own clean pre-mix channel, so you can see
  at a glance whether it is you or the far side that is hot or silent. The backend
  `recording-level` event gains `mic` / `sys` `{ rms, peak }` objects alongside the
  unchanged mixed pair (the rail ladder still reads the mix); the PEAK lamp latches on
  whichever channel clips. The live meter feed moved out of `audio/pipeline.rs` into a
  unit-tested `audio/live_meter.rs`.
- **VU calibration**: 0 VU now sits at -18 dBFS (EBU R68 alignment) on both the needles
  and the rail ladder. The mic path is loudness-normalised to -23 LUFS, so the previous
  0 VU = 0 dBFS face parked CH1 MIC at the -20 stop during normal speech.
- New app icon: two-reel silhouette on charcoal with an amber REC lamp.
- **Theme: "Faceplate" (light) / "Deck" (dark)** replaces Warm Editorial. Follows the
  macOS appearance by default; Settings → General → Appearance overrides it. Fonts are now
  Archivo / Archivo Narrow / IBM Plex Sans / IBM Plex Mono / Courier Prime.
- **Transport rail** (specs/0057 Phase C): one persistent bottom rail on every screen carries
  REC / HOLD / STOP, the reels, the tape counter, a level ladder, and the single **Queue** for
  deferred processing and background AI. The floating recording pill, the global recording bar,
  the sidebar AI-activity row and the backlog pill are gone.
- **VU meter** with real needle ballistics on the Record screen, plus PEAK and MIC GATE lamps
  (the Zoom mute gate is finally visible). The spectrometer strip is retired.
- **Channel strip**: the speaker list on a meeting shows CH numbers, talk time and share of
  talk; CH 1 is always you.
- **Sidebar**: an icon rail with a collapse/expand toggle, an engraved "NIXON" wordmark, and
  a 1.5px amber index bar tracking the active screen — the settings puck is gone.
- **Meeting details**: the reel label (a typed paper card — reel ordinal, date, time,
  length, source, voices, deck, tags) sits beside the title; the transcript body renders as
  typewriter-on-paper; the channel strip is the meeting's identity, shown above the tabs.
- **Today** and **Meetings**: both read as a transport tape log — Today as a timeline of
  transport-style blocks, Meetings (list view) with reel tags and tape-counter durations on
  every row.
- **People**, **Settings**, **Onboarding**: re-skinned with ruled rows, engraved tags, and
  position markers, matching the rest of the machine-panel language.
- **Tray**: plain text labels (no emoji) and monochrome template icons that reflect
  recording state, so the menu bar icon itself communicates idle/recording/paused.
- **Geometry pass ("nothing is a pill")**: chips, badges, capsule buttons, tags, and
  oversized cards across the app move from `rounded-full` / `rounded-xl` / `rounded-2xl` to
  the machine-panel's sharp `rounded-[3px]` (or `rounded-[2px]` for tiny tags). Circles are
  now reserved for things that are actually round — avatars, status dots/lamps, spinners,
  progress tracks, and icon pucks. The Action Items status filter is now the shared
  `SegmentedControl` instead of a hand-rolled pill row.

### Fixed

- **Theme no longer freezes at the server value after hydration.** The provider reads the
  stored preference in a layout effect instead of at state init, so Deck users no longer
  get light-skinned toasts and a mis-checked Appearance radio until a reload.
- **Tray → Start recording** routed to Today, where nothing consumed the start; it now lands
  on the recorder, and the tray no longer flips to "Starting…" before the recorder has
  agreed to start (a missing STT model used to strand it there).

- New recordings are written to the folder shown in Settings (the persisted preference), not
  to a folder guessed from disk state; this closes the "Access denied … outside the app's
  allowed data directories" path after a folder rename or machine migration.
- The mic-permission probe (`getUserMedia`) no longer runs against Nixon's own recording
  session — it uses a private, isolated stream so it can't interfere with an in-progress
  capture.
- People and Meetings-list pages fetch their full data set once instead of re-issuing the
  same "list everything" query per row/card.
- The channel strip's talk-time percentages keep the last good value across a transient
  zero-duration tick instead of flashing to 0%/NaN.
- A stale transcription-error event listener that outlived its owning component is torn
  down on unmount.
- The 8-slot speaker/avatar color palette is deterministic and collision-checked (was
  silently degrading to fewer distinguishable colors past 8 speakers).
- Settings → Appearance: the theme swatches are keyboard-navigable with the arrow keys
  (were mouse-only).
- The BlockNote summary editor's dark-mode override is scoped to `.summary-doc
  .bn-container` — it no longer bleeds into unrelated BlockNote instances.
- The sidebar's walnut cheek strip no longer gets painted over by a nav row's hover
  background (`z-10`).
- The reel label card's shadow softened to `shadow-[0_1px_2px_rgba(40,30,20,0.08)]` to
  match the mockup, and it is marked `aria-hidden` (it duplicates the accessible identity
  line above it, so it no longer double-announces to screen readers).
- The tray now logs a warning instead of silently swallowing a failed icon update.

### Removed

- The `/design-preview` prototype page and the unmounted `RecordingStatusBar` component.
