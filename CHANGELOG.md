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

### Added

- **A transcript you can actually read.** Consecutive lines from one speaker now form a
  single block under one name, with each line's timestamp on the right. The same meeting
  takes a fraction of the scrolling, and the speaker names are still one click from being
  corrected.
- **Speakers appear while Nixon is still identifying them**, instead of all at once at the
  end. The numbered speakers show up as soon as they are worked out, and the real names land
  as each voice is recognised — and the button now says what it is doing rather than sitting
  on 100%.
- **Copy an Ask AI answer as plain prose.** The Copy button on an answer leaves the meeting
  references behind, so it pastes cleanly into a message or a doc.
- **Ask AI history opens where it sits.** Click a past question and its answer expands
  underneath, with its own Copy and **Ask again** — no more losing the question you were
  about to ask.
- **Faces on Today.** Meetings on your day show who is in them and mark the one being
  recorded, the way All Meetings already did.
- **A summary tells you when it is preliminary.** A summary written before Nixon has
  finished identifying who spoke now says so above it — "Preliminary summary — speakers will
  be added when available" — until the version with names replaces it.
- **Delete a recorded meeting from Today.** Every meeting on Agenda, List and Week now has
  a ⋯ menu: recorded ones offer **Delete meeting**, the same as All Meetings.
- **Move your recordings when you change the recordings folder.** Pick a new folder in
  Settings → Recording and Nixon shows how many meetings and how much space will move, then
  moves every recording there — including ones left in folders you used before — so they
  all live in one place. You can watch its progress or stop it, and it is safe to quit
  mid-move: the next launch finishes the job without losing or duplicating a recording.

### Changed

- **Recordings stay on this Mac's own drive.** A USB stick, SD card, disk image or network
  drive can't be chosen as the recordings folder, so a drive that goes away can't break a
  recording.
- **The transcript's pause button says what it will do.** The control on the recording header
  used to be labelled with the mode you were already in — "Live" — so pressing it stopped the
  transcript. It now reads **Pause transcript** while running and **Resume transcript** while
  paused, and the state it used to carry is on the transport rail with everything else.
- **Nothing claims to be listening when it isn't.** Pause the transcript and the transcript
  area says so — and says that audio is still recording, which is the thing you actually want
  to know. The rail says "Transcript paused" on every screen.
- **A meeting tells you when it is being processed.** Stopping a meeting whose transcript was
  paused at any point queues a full re-transcribe and a fresh summary; the meeting now shows
  that it is queued, re-transcribing or summarizing, instead of showing an old transcript and
  saying nothing for minutes.
- **The meter in the footer follows the whole meeting.** It used to go dead whenever your
  mic was muted in Zoom, even though the other side of the call was still being recorded.
  Muting your mic now only changes the words next to it.
- **A tighter recording screen.** The meter bridge is smaller and carries its channel labels
  inside the meters, the duplicated status line is gone, and the three controls read
  Participants, Live, Template — all the same size.
- **Participants fold up.** A meeting with four or more people opens with a row of faces and
  a summary instead of three rows of names; one click shows everyone.
- **Today's views read Agenda, List, Week**, and the week view now has the same ⋯ menu as the
  other two, so you can hide or edit a meeting from any of them.
- **Ask AI's box is laid out around the question.** The question spans the full width, with
  Ask and Save under it on the left and the filters on the right. Answers now default to the
  last 7 days rather than all time, and the person filter says what it does — it picks the
  *meetings* someone was in.
- **Copy and Open folder moved into the transcript's ⋯ menu**, so the row shows the actions
  worth taking while a transcript is being worked on.
- **Settings shows your save location as `~/Movies/…`**, without your account name.
- **Ending a meeting is quiet.** Stopping a recording no longer stacks up "saved",
  "generating summary", "speakers identified" and "summary ready" pop-ups. Nixon takes you to
  the meeting and the tape rewinds while it works; you only hear from it if something needs
  your attention.
- **Your notes are the context for a summary.** The "Add context for AI summary" box at the
  bottom of the transcript is gone; the notes you keep on a meeting already go into every
  summary, and they are somewhere you can find them.
- **Calmer meeting tabs.** Enhance and Regenerate Summary are ordinary buttons again, the
  edit and reassign controls on a transcript line disappear as soon as the pointer leaves,
  and a summary's status sits above it instead of under pages of text.

### Fixed

- **Meetings in a recordings folder you used before keep working.** After you change where
  recordings are saved, deleting one of your earlier meetings removes its recording too,
  and an interrupted recording in the old folder is still offered to resume.
- **A meeting that ends with the transcript running now processes itself.** If you paused the
  transcript partway through a meeting and resumed it, stopping used to tell you to process
  the audio by hand — and the **Process now** button did nothing when you tried. Both went
  through the same broken check, which could never succeed for any meeting. Stopping now
  transcribes and summarizes on its own, the way a meeting you never paused always did.
- **A summary generated in the background now appears without reopening the meeting.** The
  meeting page read the summary once when you opened it, so one produced afterwards by
  background processing sat in the database unseen until you navigated away and back.
- **A meeting that was being processed no longer gets abandoned halfway.** Nixon could end
  up with two processing runs for the same meeting; the second saw the first's work already
  underway, took that for a failure, and gave up — leaving the meeting transcribed but never
  summarized. It now recognises its own work in progress and waits for it.
- **A meeting's length is recorded correctly.** The stored duration was taken after the
  recording's clocks had already been cleared, so it fell back to the moment speech last
  stopped — a meeting with quiet at the end, or a paused transcript, came out shorter than it
  was (63 seconds recorded as a 67-second meeting; 69 for a 115-second one).
- **Hiding a meeting on Today now actually hides it.** Clearing something off your day —
  lunch, a hold, anything you are not recording — no longer sends you a reminder to prep for
  it or to join it, and no longer spends a summary working out what it was about.
- **Only the alert that matters waits for you.** "A meeting is starting" stays on screen
  until you deal with it; every other Nixon banner now clears itself after a few seconds.
  Set Nixon to **Alerts** in System Settings → Notifications and Settings will tell you the
  rest.
- **A finished summary replaces "writing your summary".** When a meeting was summarized by
  background processing while its page was open, the page could keep saying it was still
  writing — over a summary that had already finished. It now shows the summary as soon as
  it is ready.
- **A meeting is summarized once when it ends.** The meeting page and background processing
  could both start a summary of the same meeting, doing the work twice and renaming the
  meeting twice. The page now leaves a meeting to background processing once it has it.
- **No "Welcome to Nixon!" flash when a meeting ends.** The recording screen emptied its
  transcript a moment before the meeting page replaced it, briefly showing the screen for a
  brand-new meeting on the way out.
- **Faces on every meeting on Today.** A meeting recorded without a calendar invite now
  shows the people named on it, the way All Meetings does.
- **The meeting you are recording is marked in the Week view** too, and appears on Today
  as soon as recording starts.
- **A meeting you added in Nixon says whether it was recorded.** Once its time had passed
  it read "Recorded" whether you had recorded it or not, and it kept offering a Record
  button long after it was over. Record now shows until the meeting ends.

### Internal

- Spec 0073 W1: a per-meeting folder lease (`audio/folder_lease.rs`) is now the one
  exclusion primitive for meeting folders. The recording saver, retranscription,
  diarization, the retention sweep, the transcript save's `folder_path` write-back, meeting
  delete and interrupted-recording discard take it and re-read `folder_path` after acquiring; a stale `folder_path` from the frontend can
  no longer overwrite a live one. "Under the current recordings root" is replaced by an
  ownership check (`audio/meeting_folder.rs`) and `known_recording_roots()`, which the
  delete, `fs_guard`, recovery, reconcile and diarization-fallback scans all use. Removed
  the unused `StreamManagerType` enum.
- Spec 0073 W2: the recordings mover (`audio/recordings_move/`). It plans a move of every
  meeting folder into one target, then moves each folder under its folder lease: a rename on
  one volume, a synced copy plus verify and staging rename across volumes. A journal
  (`recordings-move.json`) and an ordering where rows change only after a verified copy make
  every crash point recoverable at the next launch. Emptied old roots are removed with a
  non-recursive `remove_dir`. A startup gather collects leftovers, but the first launch
  only asks (`recordings-gather-needed`). A debug build never moves from, counts in or
  removes the production recordings folders. Commands: `api_plan_recordings_move`,
  `api_change_recordings_folder`, `api_gather_recordings`, `api_cancel_recordings_move`,
  `api_recordings_move_status`, `api_recordings_gather_state`.
- Spec 0073 W3: the mover's UI. `SaveLocationRow` plans → asks (`MoveRecordingsDialog`,
  Move recordings / Cancel only) → `api_change_recordings_folder`, and shows progress, Stop
  and the "still in another folder" line from `useRecordingsMove`, which pulls
  `api_recordings_move_status` / `api_recordings_gather_state` on mount because the startup
  events fire before any listener. `RecordingsMoveWatcher` (AppShell) raises the finish toast
  on any route and asks the first-launch gather question. Continue-meeting now always
  resolves its folder from the meeting row. New shots: `settings-recordings-moving`,
  `settings-recordings-gather` (`move=` mock param, fictional paths only).
- Spec 0071 covers the follow-up from an owner recording on 2026-09-21: three of the four
  reported faults were the app working as designed and failing to say so (the chip's label,
  the "Listening" indicator, and a silent 3min19s `'process-now'` repass after stop). Root
  causes established from the recording's own files.
- The VAD now splits a speech run longer than 30s — Whisper's own encoder window — into
  contiguous pieces at emission rather than handing over one oversized unit; an offline
  re-pass had produced a single 108.7-second segment. Split at emission, not by cutting
  mid-run, because `SpeechEnd` carries its own samples alongside the parallel accumulation
  and reconciling the two is how 0046 and 0051 both got scrambled clocks. `vad_split.rs` is a
  new module because `vad.rs` was at the size cap.
- `useDeferredBacklog.enqueueMeeting` read `folder_path` off `api_get_meeting`, whose
  `MeetingDetails` has no such field — so it returned `no-folder-path` for every meeting,
  ever. It now asks `api_get_meeting_metadata`, which carries it (and costs less, since it
  does not serialize every transcript). The periodic refresh path was unaffected because
  `api_list_deferred_meetings` is camelCase and matches its TS type — which is also the real
  reason a stop took 3min19s to summarize: a failed handoff, then the refresh eventually
  noticing the marker, not retranscription cost. The hook had no tests; it has five now,
  built on the real IPC payload shapes.
- Log files were capped at the plugin's default 40 KB with `KeepOne`, which discards on
  rotation — smaller than one recording produces, so two investigations found the session
  they wanted already gone. Now 8 MB, keeping the last three.
- The `[fe:backlog]` instrumentation paid for itself immediately: it showed the app
  remounting ~9s after a stop (recovery/onboarding/model-config burst at 04:37:35), which
  hands the backlog a fresh controller whose one-drain-at-a-time guard is a per-instance ref
  starting at false. `AUTOSTART_DEBOUNCE_MS` (5s) then fired its mount refresh at 04:37:40 and
  started a second drain over the first — and the first controller's JavaScript had died with
  the old React tree, so the survivor was the only thing that could finish the job.
  `start_retranscription_command` now tracks WHICH meeting holds the guard: the same meeting
  gets `already_running: true` (keep waiting — the listener was registered before the call, so
  the in-flight pass's completion event settles it), a different meeting is still refused.
- `RecordingState::stop_recording` now captures the duration accounting before anything
  clears it. `cleanup()` (via `stop_streams_only`) wipes `recording_start` and runs BEFORE
  `save_recording_only` reads it, so the save got `None` and `recording_saver.rs:1299` fell
  back to the last transcript segment's `audio_end_time`. That is the whole root cause of the
  wrong durations, and it is now covered by tests that reproduce the stop path's ordering.
  `recording_duration.rs` is a new module because `recording_state.rs` hit the size cap.
- `api_log_frontend` + `lib/app-log.ts`: the frontend can write into the app log file. The
  deferred-backlog drain sequences retranscribe → diarize → summarize entirely in TypeScript,
  so none of its decisions were recorded anywhere — when a meeting retranscribed and then
  never summarized, the log showed the Rust work succeeding and then nothing. Every drain step
  and every wait result is now logged, as is `start_retranscription_command`'s
  already-in-progress refusal, which was the one silent exit from the pipeline: the caller's
  invoke rejects, the wait reports 'error', and the meeting is abandoned undiarized and
  unsummarized while the work runs fine under whoever holds the guard.
- A recording logs every term behind its stored duration (elapsed / pauses / active). One
  recording stored 68.85s for 114.6s of ffmpeg-measured audio; that is **not fixed** — the
  mute gate is exonerated, `pause_recording` has only the HOLD caller, and the session's log
  had been truncated, so this is the evidence the next occurrence needs.
- Specs 0070 covers the four functional items of this batch (progressive speaker reveal,
  dismissed events reaching the notifier and the prep-brief generator, notification lifetime,
  the dev recordings root). The other eleven were presentation and were built without a spec.
- The debug build writes recordings to `nixon-recordings-dev` instead of inheriting the
  production probe's legacy folder; `fs_guard` allow-lists the release root in debug builds
  so pre-existing dev recordings stay readable.
- Removed two committed screenshots that leaked the maintainer's own path and email address;
  `docs/screenshots/README.md` now names the review step that missed them.
- Three shared Today components (`AgendaRowMenu`, `AgendaAttendees`, `RecordingBadge`) so the
  three views cannot drift apart again. First tests for `WeekView` and the Ask AI page.

## [0.8.0] - 2026-09-20
- Screenshots: a `today-week` shot, so the Week view is covered (its parity regressions went
  unseen for a release without one), and the sidebar's DEV row is hidden whole in shot
  mode — hiding only the badge left an empty row that lifted Settings and Queue.

### Added

- **Join and start recording in one click.** When a meeting starts, Nixon sends an alert
  carrying **Join & Record**: one press opens the call *and* starts a recording filed
  against that meeting, without switching to Nixon to press record. When the meeting has no
  link to join it offers plain **Record**, and it stays quiet while you are already
  recording.
- **A heads-up five minutes before**, carrying **Prep** — it opens that meeting's Prep tab,
  because five minutes out what you want is to remember what the meeting is for, not to
  join it yet.
- **Record straight from the "meeting detected" alert** when Nixon spots a call you might
  want to capture, instead of only bringing Nixon forward.
- **A banner when a recording starts and stops**, while Nixon is behind another window —
  the one clear sign that the recording is running.
- **Notification settings** in Settings → General: whether macOS is letting Nixon notify
  you, a way to grant it, a route into System Settings, and a **Send a test** button. macOS
  will ask your permission the first time Nixon has something to tell you.
- **Add a meeting to your day inside Nixon.** A call that is not on your calendar — or that
  is not on a calendar at all — can be put on Today with a title, a time and an optional
  join link. Nixon opens it as soon as you add it, so you can start writing prep straight
  away, and you can change its time or title later from the meeting itself. Record it
  whenever you are ready — there is no waiting for the clock to catch up — and the prep you
  wrote comes with it: it becomes that meeting's recording rather than a second, empty one.
- **A List view for Today**, alongside Day and Week. If you have not connected a calendar,
  Nixon opens on it — your day as a simple list of what you have added and what you have
  recorded, instead of an empty hour grid.
- **Restarting to update asks first**, and says what it is about to do: which version you
  are on, which one you are getting, and that Nixon will close and reopen.
- **Nixon tells you when it has updated.** The first time you open the new version it shows
  what changed, once.

### Changed

- **The sidebar keeps its layout when you open it.** The icons stay exactly where they are,
  at the size the collapsed rail shows them; expanding widens the panel and reveals the
  labels. The reel mark stays put, beside the NIXON wordmark.
- **Import audio lives in the All meetings header**, where an imported recording lands.
  Dropping an audio file onto the window works from anywhere, as it always has.
- **The update indicator is a restart symbol**, so it reads at a glance as "relaunch to
  update" rather than as another status lamp.
- **Today's header is one button — Add meeting.** Recording stays where it always is: the
  transport rail's REC key, and ⌘⇧R from anywhere.

### Fixed

- **Today recognises Google Calendar as your calendar.** If Google is your source, Today
  treats you as connected instead of asking you to connect one — and the prompt can be
  dismissed for good.

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- Replaces the previous notification path with a native
  `UNUserNotificationCenter` layer (`notifications/macos/`, objc2-user-notifications 0.3):
  capability gate, authorization, delivery, and a `define_class!` delegate that emits
  `notification-action`. The bundle gate matters — `currentNotificationCenter` raises an
  uncaught `NSInternalInconsistencyException` outside an `.app`, so `tauri dev` would abort;
  `bundle_gate_holds` covers it and was sabotage-verified.
- Deletes the module it replaces (`manager`/`settings`/`system`/`types`/`commands`,
  ~1,500 lines, 14 registered commands of which the frontend called two) and drops
  `tauri-plugin-notification` and `@tauri-apps/plugin-notification`. Net −413 lines.
- macOS 26 renders one notification action as a button and hides two or more behind an
  "Options" menu, whatever the notification style; `NSUserNotificationAlertStyle` is
  ignored. Hence one action per category, and two alerts carrying different ones. Measured
  with a throwaway app under a fresh bundle id, not assumed.

## [0.7.0] - 2026-09-19

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

- **Clicking the collapsed sidebar's update lamp opens a panel describing the update**, with
  Restart as its own labelled button inside it — checking what the light means is never the
  same click as installing the update.
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
  fact that every figure comes from one M3 Ultra.
- `patchRecordingPreferences` read-modify-writes the recording-preferences store. Every
  caller used to save the whole object from state loaded at mount, which was safe until
  devices and recording behaviour ended up on different tabs.
- A saved summary no longer needs a `transcript_chunks` row to be readable, which is why
  every `--demo` meeting showed "No Summary Generated Yet".
- README requirements now state measured hardware expectations: peak memory per summary
  model, model download sizes, ~250 MB per recorded hour, and that a base M1 summarises an
  hour-long meeting in about a minute.

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
  their Connect button takes you straight to Settings → Calendar to finish connecting.
- **UI enhancements throughout the app.** The sidebar (Home is now **Today**, and the queue
  lives there rather than in the footer), the transport rail (the timer and level meter hold
  still while the meeting title changes, and REC/HOLD/STOP sit together at the right), and
  the meeting page (a full-width header, a proper participants grid, speakers ordered by
  share of talk, and a shorter Summary toolbar). Plus smaller touches: REC stays readable
  while a recording is on hold, release notes in Settings → About render as formatted text,
  and the deck rewinds while a summary is being written.

### Fixed

- **Retry in the Queue now tells you what happened.** If a retry can't run — because that
  work is already in progress, or the task has already been cleared — you get a clear
  message instead of the row silently vanishing.
- **A speaker Nixon already recognizes is named automatically**, instead of offering a
  "Looks like …" button to click. That covers a well-trained voiceprint and a voice
  recognized across several previous meetings, and it applies whenever you open the meeting
  — including older ones you're revisiting, not just the moment it was first analyzed. A name
  applied this way is renamed like any other.
- **Prep notes now follow a meeting that gets rescheduled** — moving a meeting to another day
  keeps its notes, its brief, and its link to previous occurrences. For a macOS Calendar
  meeting the notes move once the old time is confirmed empty, so a recurring series' other
  occurrences keep their own.

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- Tauri copies `templates/*.json` into `target/<profile>/templates/` and never prunes, so a
  template deleted from source kept being served by dev builds — the Psychiatric Session
  template was still listed as hidden two specs after its removal. `build.rs` now deletes
  staged templates the source dir no longer has.
- `tests/import_audio.rs` runs a real file through the whole import path (model-gated,
  skips without a Whisper model). Writing it found that `get_or_init_whisper` never did the
  "or init" half, so import failed where live transcription self-healed.
- The screenshot mock now rejects debug-only commands (`dev_get_flags` and friends) instead
  of resolving everything, so shots show the release build's UI.
- A saved summary no longer has to have a `transcript_chunks` row to be readable.
  `get_summary_data_for_meeting` joined that table, which is written only by the summary
  *generation* path — so a summary that arrived any other way was invisible to the app
  holding it. Every meeting in the demo dataset was in that state, which is why `--demo`
  showed five meetings with no summaries. No real meeting is affected (a generated summary
  always writes chunks; checked against a production profile), so this is dev-visible only.
- The README's real-window screenshots are regenerated for the release: the debug-only
  Developer tab is hidden during captures (`data-dev-only` + the existing `data-shot`
  mechanism), the meeting routes wait long enough for their content, and the stale
  TapeCounter ignore rect — left pointing at the old rail layout — moved to
  where the counter actually is.

## [0.5.0] - 2026-09-18

### Added

- While a recording is running, the transport rail's title is a link back to it — click it
  from any screen to land on the recording.
- **The Queue panel now shows what's running in the background** — diarization, automatic
  summaries, and post-recording transcription — so you can see the work in progress and
  check back on it later.

### Changed

- **The Prep tab is reordered and simplified**: your prep notes are now at the **top**,
  above the brief and the carried-over items, so the one part of the tab you write is the
  first thing you reach. The brief renders flat, with its sources on a single muted "From"
  line, and "Link previous meeting…" / "Regenerate" are icon buttons with tooltips instead
  of two labelled buttons.
- Carried-over open items in the Prep tab can be **checked off in place**, instead of being
  read-only. A checked item stays visible, struck through, until you leave the tab, and the
  Prep count badge updates as you go.
- The duplicate queue popover on Today is removed — its "Processing…" button now opens the
  transport rail's Queue panel instead.

### Fixed

- Renaming a meeting from the record header now updates the transport rail and the meetings
  list immediately.
- The REC/HOLD/STOP keys have more breathing room above the lamp bar, and a lit key now
  lights its whole face — REC is cream on red, HOLD is dark on amber — so "armed" is
  unmistakable in both themes.
- **Failed items in the Queue show a "Failed" label with a real Retry button** (or Dismiss,
  for tasks that can't be retried) — covering deferred backlog meetings and failed
  background AI work like summaries, extractions, prep briefs, and diarization.
- A background summary run (the post-diarization re-summarize, or a summary retried from
  the Queue) that generates successfully but fails to save now shows as "Failed" in the
  Queue instead of being silently logged.
- A summary you cancel is recorded as skipped rather than as a success.
- The automatic summary that runs after a recording stops now appears in the Queue while it
  works, so it stays visible if you navigate away from the meeting — and if it fails, it can
  be retried from there.

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- The file-size gate (`scripts/check-file-size.sh`) no longer freezes each large legacy file
  at its exact line count. The 800-line cap on new files is unchanged, but the grandfathered
  files now share a single shrinking *excess* budget, so a small justified addition is
  possible without a refactor first, and splitting a large file now reduces the budget rather
  than merely being permitted. `scripts/file-size-allowlist.txt` is replaced by
  `scripts/file-size-tracked.txt` plus `scripts/file-size-budget.txt`.
- The orphaned prep-only retry path (`api_llm_activity_retry` and its unused frontend half)
  is deleted; `llm_activity::retry::retry_task` is the only retry path.

## [0.4.0] - 2026-09-18

### Added

- **Editable transcript text.** Click a line's pencil to correct it inline — a saved edit
  is marked and stays searchable, and if the save fails, your edit stays in the editor
  instead of being lost. Re-transcribing warns you how many manually-edited lines it's
  about to replace, or shows a general warning if that count isn't available.
- **Click a speaker's row in a meeting's channel strip** to filter the transcript to that
  speaker and jump to their first line — click again to clear the filter. If Nixon has
  consolidated several detected voices into one person, the filter covers all of them, not
  just one.
- A "Change…" folder picker next to "Open folder" in Recording settings, so where new
  recordings are saved can be changed without leaving the app — existing meetings keep
  their own already-saved folder.

### Changed

- Reassigning a transcript line to "You" now works even in a meeting that never diarized
  an owner track — "You" always appears as a reassignment option. A non-owner speaker left
  with no lines after a correction or merge, and no stored voiceprint, is removed instead
  of lingering in the speaker panel.
- The speaker legend's channel strip fits six rows before it needs to scroll.
- "Summarize automatically when a meeting ends" is a single toggle under Summary —
  Recording settings points to it instead of duplicating it. Live speaker labels in
  Recording settings are described as provisional numbered placeholders that get real
  names once the recording ends.
- The Speakers section on a meeting page is boxed like Participants, with its title inside
  the box.

### Fixed

- A meeting pinned to a summary template that no longer exists — a built-in removed in an
  update, or a deleted custom template — now falls back to the default template instead of
  failing to generate a summary.

### Removed

- The "Test Mic" audio-level monitor and the "File format" row are gone from Recording
  settings — neither did anything you could act on.
- The Psychiatric Session built-in summary template. Meetings already pinned to it fall
  back to the default template.

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- Dev-only fixture seeding (`--demo`), onboarding harness (`--onboarding`) and a Developer
  section in Settings › Beta; debug builds only.
- Screenshot pipeline: `pnpm shots` (headless, both themes), `pnpm shots:diff` (contact
  sheet), `pnpm shots:real` (real window via a debug-only control listener); README
  screenshots.
- The headless screenshot mock is generated from the fixture dataset (`pnpm shots:mock`).
- A `--demo` profile is now fully synthetic. The fixture seeder re-seeded meetings and
  people but left the Google Calendar cache alone, so a connected account's real events,
  attendee names and attendee photos rendered straight through it — and into the first
  real screenshot run. The demo seed now also clears the cached events, attendee photos,
  dismissed events and briefs (and resets the sync tokens so the next non-demo launch
  does a full re-sync), and calendar sync is suppressed at source while the demo dataset
  is active. The connected account is kept, so no re-authentication is needed.
- `pnpm shots:real` can capture the live-recording screen again: the debug control
  listener started recordings through the explicit-devices path with no devices, which
  could only ever fail with "No audio streams could be created". It now uses the same
  default-device resolution a recording started from the UI uses.

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

- **Refined the record and meeting-details screens.** The PEAK / MIC GATE lamps are gone
  from the record header — the transport rail already carries both states — the template,
  mode, and participants controls sit under the meeting title, and the back control is the
  same plain chevron used on meeting details. On a meeting's detail page, the reel card is
  gone in favor of a single identity line carrying the reel number, date, time, length,
  source, and voice count, and at wide widths the back button sits in the left gutter so
  the title lines up with the content below it. Speaker names in the channel strip and
  people in the participants row show their edit/remove controls only on hover. The record
  red is a brighter lamp color in both themes, with a new red text shade for better
  contrast.

- **Every Settings tab now looks and behaves the same way** — one section-header style,
  one card style, one row layout for every setting (name, description, control on the
  right), ordered the way you'd meet them: what's captured, how it behaves, where it's
  stored, with destructive options last. Loading states show a proper placeholder instead
  of an oddly-styled box, and the Transcription tab's model picker drops its emoji icons;
  options now read "Parakeet (recommended)" and "Local Whisper".

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- Settings pages are now built from a shared set of primitives in
  `frontend/src/components/ui/settings.tsx`, replacing three vocabularies that had grown
  side by side (the ruled row, the old shadcn `p-6 shadow-sm` card, and ad-hoc bare
  labels) and the `u-section-label` engraved style misused as a row label. Canvas feedback
  pass incorporated design review feedback from 2026-09-15.

## [0.1.0] - 2026-09-14

First release under the Nixon name. Everything that shipped in the private Vinyl line
(0.2.0 – 1.20.0: on-device transcription, diarization with owner-mic attribution, deferred
processing, Zoom mute gate, Google Calendar, summaries and action items, voiceprints) is
carried forward unchanged; the entries below are what is new relative to Vinyl 1.20.0.

### Changed

- **Rebrand: Vinyl → Nixon.** New product name throughout — window, tray, and notification
  titles, onboarding and settings copy, and the app icon. Your existing data and
  permissions carry over untouched.
- **Recordings folder**: fresh installs use `~/Movies/nixon-recordings/`. An install that has
  `~/Movies/meetily-recordings/` keeps writing there for as long as that folder exists.
- **Two-channel VU on the record page.** The single mixed needle is now **CH1 MIC** and
  **CH2 SYS**, each reading its own channel, so you can see at a glance whether it's you or
  the far side that's hot or silent. The PEAK lamp latches on whichever channel clips.
- **VU calibration.** The needles and rail ladder are recalibrated so normal speech reads
  near the middle of the meter instead of low and quiet-looking.
- New app icon: two-reel silhouette on charcoal with an amber REC lamp.
- **Theme: "Faceplate" (light) / "Deck" (dark)** replaces Warm Editorial. Follows the
  macOS appearance by default; Settings → General → Appearance overrides it. Fonts are now
  Archivo / Archivo Narrow / IBM Plex Sans / IBM Plex Mono / Courier Prime.
- **Transport rail.** One persistent bottom rail on every screen carries REC / HOLD / STOP,
  the reels, the tape counter, a level ladder, and a single **Queue** for deferred
  processing and background AI — replacing the floating recording pill, the global
  recording bar, the sidebar AI-activity row, and the backlog pill.
- **VU meter** with real needle ballistics on the Record screen, plus PEAK and MIC GATE
  lamps, so the Zoom mute gate is visible at a glance. The spectrometer strip is retired.
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
- **Geometry pass ("nothing is a pill").** Chips, badges, buttons, tags, and cards across
  the app move from fully-rounded to the machine-panel's sharp corners. Circles are now
  reserved for things that are actually round — avatars, status dots/lamps, spinners,
  progress tracks, and icon pucks.

### Fixed

- **Theme now applies immediately**, without a reload — Deck users no longer briefly see
  light-skinned toasts or a mis-checked Appearance radio after opening the app.
- **Tray → Start recording now lands on the recorder**, instead of just opening Today. The
  tray no longer shows "Starting…" if a missing transcription model stops the recording
  from beginning.
- **New recordings always go to the folder shown in Settings.** This fixes an "Access
  denied" error that could show up after renaming the recordings folder or moving to a new
  machine.
- Checking microphone permission no longer risks interfering with a recording already in
  progress.
- People and the meetings list load faster — each page fetches its data once instead of
  re-querying per row.
- The channel strip's talk-time percentages keep the last good value across a transient
  zero-duration tick instead of flashing to 0%.
- The 8-slot speaker/avatar color palette is deterministic and collision-checked, so
  reused colors past 8 speakers are consistent instead of degrading unpredictably.
- Settings → Appearance: the theme swatches are keyboard-navigable with the arrow keys.
- A dark-mode style leak that could affect other note editors elsewhere in the app is
  fixed.
- The sidebar's walnut cheek strip no longer gets painted over by a nav row's hover
  highlight.
- The reel label card's shadow is softer, and screen readers no longer announce it twice
  (it duplicates the identity line above it).

### Internal

_Not shown in release notes or the in-app updater (see `release.sh`)._

- Rebrand touched script names (`dev-nixon.sh`, `upgrade-nixon.sh`), DMG/app bundle names,
  and the Google OAuth build variables (`NIXON_GOOGLE_CLIENT_ID` /
  `NIXON_GOOGLE_CLIENT_SECRET` in the gitignored `.env.google`). The bundle identifier
  `ai.vinyl.app` is unchanged.
- The backend `recording-level` event gains `mic` / `sys` `{ rms, peak }` objects alongside
  the unchanged mixed pair (the rail ladder still reads the mix). The live meter feed moved
  out of `audio/pipeline.rs` into a unit-tested `audio/live_meter.rs`.
- 0 VU now sits at -18 dBFS (EBU R68 alignment) on both the needles and the rail ladder; the
  mic path is loudness-normalised to -23 LUFS. The previous 0 VU = 0 dBFS calibration
  parked CH1 MIC at the -20 stop during normal speech.
- Geometry pass: `rounded-full` / `rounded-xl` / `rounded-2xl` become `rounded-[3px]`
  (`rounded-[2px]` for tiny tags) throughout. The Action Items status filter now uses the
  shared `SegmentedControl` instead of a hand-rolled pill row.
- The theme provider reads the stored preference in a layout effect instead of at state
  init, fixing the freeze-at-server-value bug after hydration.
- The mic-permission probe (`getUserMedia`) now uses a private, isolated stream instead of
  running against Nixon's own recording session.
- A stale transcription-error event listener that outlived its owning component is torn
  down on unmount.
- The BlockNote summary editor's dark-mode override is scoped to `.summary-doc
  .bn-container` so it no longer bleeds into unrelated BlockNote instances.
- The reel label card's shadow is `shadow-[0_1px_2px_rgba(40,30,20,0.08)]` to match the
  mockup, and it is marked `aria-hidden` (it duplicates the accessible identity line above
  it).
- The tray now logs a warning instead of silently swallowing a failed icon update.
- The `/design-preview` prototype page and the unmounted `RecordingStatusBar` component are
  deleted.
