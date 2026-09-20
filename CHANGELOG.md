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
>
> **Those sections are user-facing and are published verbatim** as the GitHub release body
> AND as the `notes` the in-app updater shows in its update dialog. Anything a person using
> Nixon would not notice — CI gates, build tooling, refactors, dead-code removal — goes under
> a trailing `### Internal` heading instead, which `release.sh` strips from both.

## [Unreleased]

### Changed

- **Nixon no longer asks you to pick AI models.** It chooses the transcription engine and
  the summary model to suit your Mac, and Settings shows what is in use rather than a menu
  of names only an enthusiast could rank. Each one still has a **Change** next to it if you
  want the full list back — including the cloud providers — and a choice you have already
  made stays on screen instead of hiding behind that link.
- **Settings is reorganised so each tab is one subject.** General, Audio, Calendar,
  Recording, Transcription, Summary, About. Notably: **Audio** shows each input's
  permission *and* its device on the same row, so you are not checking two tabs to find out
  why the microphone is quiet; speaker labels and voiceprints moved to **Transcription**,
  beside the engine they describe; your calendar source, Google connection and email
  addresses are together on **Calendar**; and templates are on **Summary**, which is what
  they shape.
- **Fewer settings that did nothing.** The transcription-language picker is gone for the
  default engine, which only ever detects the language automatically; the summary-language
  picker is gone from both Settings and the meeting page, where Auto already follows the
  transcript; and the **AI Model** entry is gone from a meeting's "…" menu, since the model
  is chosen for you and lives in Settings.
- The **Recording** tab is tidier: the pause-mic-in-Zoom switch sits with the other
  recording switches, the audio-retention dropdown matches every other dropdown in the app,
  and a signpost pointing at another tab is gone.

### Fixed

- **On the collapsed sidebar, the update lamp now opens a panel instead of restarting the
  app.** Clicking that amber dot — which sits right above the queue's own amber dot, with no
  label to tell them apart — used to relaunch Nixon and install the update on the spot. It
  refused mid-recording, but clicking a light to find out what it means should never be the
  same gesture as "restart now". It opens a flyout describing the update, with Restart as a
  separate, labelled button inside it.
- A person's name no longer wraps onto a second line on their page while the rest of the
  row sits empty.

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- `tests/transcription_wer.rs` scores both transcription engines against the Zoom VTTs in
  the eval corpus — the repo had no WER measurement at all, only DER for diarization. Both
  engines are fed 30-second windows, matching how the app actually calls them; an earlier
  version handed them five-minute buffers and produced a ~100% WER for Whisper that would
  have argued for deleting the better engine. Parakeet 17.6% vs Whisper medium-q5_0 20.6%;
  against Whisper's actual default (large-v3-turbo) it is 17.6% vs 14.7%, with Whisper also
  faster on this hardware — the default stays Parakeet on size, streaming design and the
  fact that every figure comes from one M3 Ultra (specs/0067).
- `patchRecordingPreferences` read-modify-writes the recording-preferences store. Every
  caller used to save the whole object from state loaded at mount, which was safe until
  devices and recording behaviour ended up on different tabs (specs/0067).
- A saved summary no longer needs a `transcript_chunks` row to be readable, which is why
  every `--demo` meeting showed "No Summary Generated Yet" (specs/0066).
- README requirements now state measured hardware expectations: peak memory per summary
  model, model download sizes, ~250 MB per recorded hour, and that a base M1 summarises an
  hour-long meeting in about a minute (specs/0066).

## [0.6.0] - 2026-09-19

### Changed

- **A summary is now written automatically when a recording stops.** This was off by
  default and had to be found in Settings first. If you turned it off deliberately, it
  stays off.
- **Import audio is no longer a beta feature.** It is an ordinary part of the app: the
  sidebar button has lost its BETA tag, and there is no switch to turn it on first. The
  Beta section of Settings is gone with it, since that was the only feature in it.
- **Two settings are removed.** *Expected number of speakers* is no longer needed — Nixon
  sizes a meeting from the audio itself, and a number set here used to override that for
  good. *System Audio Backend* offered a second capture path that needed BlackHole and
  reverted itself on the next launch; system audio always uses the Core Audio tap now.
- The **"connect your calendar" prompts** name Google Calendar as well as the Mac's, and
  their Connect button takes you straight to Settings → Calendar. Previously they named
  only macOS Calendar, and Connect asked for calendar permission on the spot — which, if
  the answer was no, could only tell you to go and find a setting for yourself.
- **UI enhancements throughout the app.** The sidebar (Home is now **Today**, and the queue
  lives there rather than in the footer), the transport rail (the timer and level meter hold
  still while the meeting title changes, and REC/HOLD/STOP sit together at the right), and
  the meeting page (a full-width header, a proper participants grid, speakers ordered by
  share of talk, and a shorter Summary toolbar). Plus smaller touches: REC stays readable
  while a recording is on hold, release notes in Settings → About render as formatted text,
  and the deck rewinds while a summary is being written.

### Fixed

- **Retry in the Queue now works, or tells you why it can't.** Clicking it on a failed item
  could quietly do nothing: a retry the app refused — because that work was already running,
  or the task had already been cleared — was thrown away without a word, and a refused
  speaker-identification retry even reported success while the row vanished. Every refusal
  now says what happened.
- **A speaker Nixon already recognizes is named automatically**, instead of offering a
  "Looks like …" button for you to click. That covers a well-trained voiceprint and a voice
  it has recognized across several previous meetings, and it now happens when you open an
  older meeting too — not only at the moment the meeting was first analyzed, which is why
  confident matches used to keep asking forever. A name applied this way is renamed like any
  other.
- **Prep notes now follow a meeting that gets rescheduled.** Moving a meeting to another day
  used to leave your prep behind on a meeting that had quietly disappeared, and the new time
  showed up as a blank one; the prep now moves with the meeting, keeping its notes, its brief
  and its link to previous occurrences. For a macOS Calendar meeting the notes move once the
  old time is confirmed empty, so a recurring series' other occurrences always keep their own.

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- Tauri copies `templates/*.json` into `target/<profile>/templates/` and never prunes, so a
  template deleted from source kept being served by dev builds — the Psychiatric Session
  template was still listed as hidden two specs after its removal. `build.rs` now deletes
  staged templates the source dir no longer has (specs/0066).
- `tests/import_audio.rs` runs a real file through the whole import path (model-gated,
  skips without a Whisper model). Writing it found that `get_or_init_whisper` never did the
  "or init" half, so import failed where live transcription self-healed (specs/0066).
- The screenshot mock now rejects debug-only commands (`dev_get_flags` and friends) instead
  of resolving everything, so shots show the release build's UI (specs/0066).
- A saved summary no longer has to have a `transcript_chunks` row to be readable.
  `get_summary_data_for_meeting` joined that table, which is written only by the summary
  *generation* path — so a summary that arrived any other way was invisible to the app
  holding it. Every meeting in the demo dataset was in that state, which is why `--demo`
  showed five meetings with no summaries. No real meeting is affected (a generated summary
  always writes chunks; checked against a production profile), so this is dev-visible only
  (specs/0066).
- The README's real-window screenshots are regenerated for the release: the debug-only
  Developer tab is hidden during captures (`data-dev-only` + the existing `data-shot`
  mechanism), the meeting routes wait long enough for their content, and the stale
  TapeCounter ignore rect — left pointing at the old rail layout by specs/0064 — moved to
  where the counter actually is (specs/0066).

## [0.5.0] - 2026-09-18

### Added

- While a recording is running, the transport rail's title is a link back to it — click it
  from any screen to land on the recording (specs/0063 W1).
- Diarization passes, automatic background summary runs (the post-diarization re-summarize,
  or a summary retried from the Queue), and post-stop transcription status now appear in
  the Queue panel while they run (specs/0063 W3).

### Changed

- The Prep tab is reordered and simplified (specs/0063 W4): your prep notes are now at the
  **top**, above the brief and the carried-over items, so the one part of the tab you write
  is the first thing you reach. The brief renders flat — no card around it — with its
  sources on a single muted "From" line, and "Link previous meeting…" / "Regenerate" are now
  icon buttons with tooltips instead of two labelled buttons crowding the heading.
- Carried-over open items in the Prep tab can be **checked off in place**, instead of being
  read-only. A checked item stays visible, struck through, until you leave the tab, and the
  Prep count badge updates as you go (specs/0063 W4).
- The duplicate queue popover on Today is removed. Its "Processing…" button now opens the
  transport rail's Queue panel instead (specs/0063 W3).

### Fixed

- Renaming a meeting from the record header now updates the transport rail and the meetings
  list immediately, instead of leaving the old name in the footer for the rest of the session
  (specs/0063 W1).
- The REC/HOLD/STOP keys no longer sit so tight that the glyph touches the lamp bar above it,
  and a lit key now lights its whole face — REC is cream on red, HOLD is dark on amber — so
  "armed" is unmistakable in both themes. Previously only a 4px bar lit, and on the light
  theme it lit in a brown dark enough to read as unlit (specs/0063 W2).
- The Queue's "Retry" stage label was misleading — it was plain text, not a button. The label
  now reads "Failed", and every failed row shows a real Retry button (or Dismiss for
  non-retryable tasks) that works. Failed deferred backlog meetings and failed background AI
  work (summaries, extractions, prep briefs, diarization) can now be retried from the Queue
  (specs/0063 W3).
- A background summary run (the post-diarization re-summarize, or a summary retried from
  the Queue) that generates successfully but then fails to save now shows as "Failed" in
  the Queue instead of being silently logged (specs/0063 W3).
- A summary you cancel is now recorded as skipped rather than as a success (specs/0063 W3).
- The automatic summary that runs after a recording stops now appears in the Queue while it
  works, so it stays visible if you navigate away from the meeting — and if it fails, it can
  be retried from there. Previously it was only ever visible as inline progress on the meeting
  page, and vanished the moment you left (specs/0063 W3).

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- The file-size gate (`scripts/check-file-size.sh`) no longer freezes each large legacy file
  at its exact line count. The 800-line cap on new files is unchanged, but the grandfathered
  files now share a single shrinking *excess* budget, so a small justified addition is
  possible without a refactor first, and splitting a large file now reduces the budget rather
  than merely being permitted. `scripts/file-size-allowlist.txt` is replaced by
  `scripts/file-size-tracked.txt` plus `scripts/file-size-budget.txt` (specs/0065).
- The orphaned prep-only retry path (`api_llm_activity_retry` and its unused frontend half)
  is deleted; `llm_activity::retry::retry_task` is the only retry path.

## [0.4.0] - 2026-09-18

### Added

- Dev-only fixture seeding (`--demo`), onboarding harness (`--onboarding`) and a Developer
  section in Settings › Beta; debug builds only (specs/0059).
- Screenshot pipeline: `pnpm shots` (headless, both themes), `pnpm shots:diff` (contact
  sheet), `pnpm shots:real` (real window via a debug-only control listener); README
  screenshots (specs/0060).
- Editable transcript text: click a line's pencil to correct it inline. A saved edit is
  marked `edited` and stays searchable (the correction re-indexes for full-text search); a
  failed save keeps the editor open with what you typed instead of discarding it.
  Re-transcribing now warns how many manually-edited lines it will replace, or — if that
  count can't be fetched — shows a generic warning rather than silently reading as "no
  edits at risk" (specs/0061 W5).
- Click a speaker's row in a meeting's channel strip to filter the transcript to that
  speaker and jump to their first line; click the row again to clear the filter. When
  diarization over-splits one person into several speaker keys later assigned to the same
  person, the legend shows them as one consolidated row — clicking it filters and jumps
  across every one of those underlying keys, not just the row's primary key (specs/0061
  W4).
- A "Change…" folder picker next to "Open folder" in Recording settings, so where new
  recordings are saved can be changed without leaving the app; existing meetings keep
  their own already-saved folder (specs/0061 W6).

### Changed

- The headless screenshot mock is generated from the fixture dataset (`pnpm shots:mock`).
- Reassigning a transcript line to "You" now works even in a meeting that never diarized
  an owner track — the owner's speaker row is created on demand, and "You" always appears
  as a reassignment option. A non-owner speaker left with zero lines after a correction or
  merge, and with no stored voiceprint, is pruned instead of lingering in the speaker panel
  (specs/0061 W4).
- The speaker legend's channel strip fits six rows before it needs to scroll (specs/0061
  W4).
- "Summarize automatically when a meeting ends" is now a single toggle, under Summary;
  Recording settings previously duplicated it as a second switch and now just points to
  the one under Summary. A few Recording settings rows also read more honestly about what
  they do — e.g. live speaker labels are described as provisional numbered placeholders
  that get real names once the recording ends, not live names (specs/0061 W6).
- The Speakers section on a meeting page is boxed like Participants, with its title inside
  the box (specs/0064 item 4).

### Fixed

- A `--demo` profile is now fully synthetic. The fixture seeder re-seeded meetings and
  people but left the Google Calendar cache alone, so a connected account's real events,
  attendee names and attendee photos rendered straight through it — and into the first
  real screenshot run. The demo seed now also clears the cached events, attendee photos,
  dismissed events and briefs (and resets the sync tokens so the next non-demo launch
  does a full re-sync), and calendar sync is suppressed at source while the demo dataset
  is active. The connected account is kept, so no re-authentication is needed
  (specs/0059, specs/0060).
- `pnpm shots:real` can capture the live-recording screen again: the debug control
  listener started recordings through the explicit-devices path with no devices, which
  could only ever fail with "No audio streams could be created". It now uses the same
  default-device resolution a recording started from the UI uses (specs/0060).

- A meeting pinned to a summary template id that no longer resolves — a built-in removed in
  an app update, or a deleted custom template — now falls back to the default template
  (with a logged warning) instead of having its summary generation marked failed
  (specs/0061 W6).

### Removed

- The dead "Test Mic" audio-level monitor and the "File format" row from Recording
  settings — neither did anything a user could act on (specs/0061 W6).
- The Psychiatric Session built-in summary template (specs/0061 W6). Meetings already
  pinned to it fall back to the default template at generation time — see Fixed, above,
  for the general-case guarantee this also relies on.

## [0.3.1] - 2026-09-16

### Changed

- **Updater verification release.** No functional changes; this release exists to exercise
  the in-app update path introduced in 0.3.0 (background download, restart to update).

## [0.3.0] - 2026-09-16

### Added

- **In-app updates.** Nixon checks GitHub for new versions about every six hours,
  downloads them in the background, and shows a "ready · Restart" row in the sidebar
  (and a tray item) once the update is verified. Restarting is always your choice and
  is never offered mid-recording. "Check for updates" lives in Settings > About; the
  background check can be turned off under Settings > General. Copies older than this
  release need one last manual install from the release page.

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
