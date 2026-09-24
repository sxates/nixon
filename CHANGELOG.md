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

_(nothing yet)_

## [0.11.0] - 2026-09-24

### Added

- **A new menu bar icon.** Nixon's menu bar icon is now a single tape reel, the one from the
  app icon. While a meeting records, it turns and shows a red light in its corner; on
  hold it stops and the light blinks amber, as the HOLD key does in the app. It now matches
  the other menu bar icons' colour instead of staying black.
- **Nixon remembers where you left its window.** Its size and position carry over when you
  quit and reopen it, and across updates.

### Changed

- **Low Power Mode also keeps the animations still.** On battery with Low Power Mode on,
  the tape reels, the VU needles and the blinking lights stop moving while you record. The
  lights stay lit and every reading still updates, and Nixon uses noticeably less CPU during
  a meeting.

### Fixed

- Turning off **Label speakers live while recording** now stops it in the meeting you're
  recording, not just from the next one. The setting now notes that it uses significant CPU.

### Internal

- specs/0077 calm motion: `CalmMotionProvider` sets `html[data-calm-motion]` (globals.css stops
  `[data-hub]`, `.animate-hold-blink`, `.animate-pulse`); VuMeter takes its reduced-motion path;
  `power::calm_motion()` holds the tray reel. Measured: the reels and VU each cost ~12% of a core in
  WindowServer while visible.
- Tray icon: `src/tray_reel.rs` + `scripts/tray-icons/render.mjs`. All reel frames are template
  images set with `set_icon_with_as_template` — plain `set_icon` cleared the template flag, so the
  old icon rendered black on a menu bar tinted white. The REC light is a `CALayer` on the status
  button, since a template can't hold colour (HOLD blinks it with a `CABasicAnimation`).
- Window state: `tauri-plugin-window-state` (size, position, maximized), skipped under the dev
  control channel so screenshot runs don't overwrite the saved size; saved explicitly before the
  updater's `app.restart()`, which bypasses `RunEvent::Exit` on the main thread.
- specs/0076: a process-wide `LIVE_PASSES_ENABLED` flag, checked by the live diarization pass
  loop each tick and set by `api_set_live_diarization_enabled`.

## [0.10.0] - 2026-09-24

### Added

- **Google Calendar tells you when it needs you.** If your Google sign-in expires or is
  revoked, Today and Settings → Calendar show a **Reconnect** prompt, and **Sync now** says
  what actually happened — how many changes it picked up, that a sync is still running, or
  that nothing synced and why. Each calendar also shows when it last synced.
- **VU meters with depth.** The recording meters sit in a thin bezel with a backlit face,
  and fill the header instead of floating in it.

### Changed

- **A narrow window gives the page more room.** Below about 900 pixels wide the sidebar
  folds to its icons (open it and it floats over the page) and the margins tighten. The VU
  meters step aside when the recording header runs out of room.
- **Recording opens on the recording screen.** Pressing REC goes straight to "Listening for
  speech…", and stopping shows "Saving the recording…" until your meeting opens.
- **The live transcript follows along from the first line**, and picks up following again
  whenever you scroll back to the bottom.
- **The meeting being recorded is marked once on Today**, with the turning reels, in
  Agenda, List and Week alike.
- **Meetings you add yourself show Prep until they're about to start**, like calendar
  meetings. The Record button appears five minutes before; to record earlier, choose
  **Record now** from the meeting's menu.

### Fixed

- The "starting now" notification arrives at the start of a calendar meeting even when
  Nixon is in the background.
- Google Calendar picks up meetings added or moved since the last sync, and keeps syncing
  after the Mac sleeps.
- A meeting moved to another day leaves Today.
- Nixon no longer offers the recording in progress as an interrupted meeting to recover or
  delete.
- A long status in the sidebar queue no longer runs into its count.

### Internal

- specs/0075: `SyncOutcome` from `sync_all`; oauth2 token refresh and code exchange bounded
  at 15s (its reqwest client had no timeout, so a half-open socket could hold `SYNC_LOCK`
  until restart); 120s pass cap; bounded startup probe; enrichment on its own lock;
  `singleEvents` on incremental requests; `google-calendar-synced` event;
  `notif_deliver.deliverAtMs` (UNTimeIntervalNotificationTrigger) + `notif_cancel_pending`;
  `api_get_upcoming_meetings.includeStartedWithinMs`; `api_get_day_agenda` returns
  `{ items, calendarSource }`; recovery waits for the first recording-state reply.
- Screenshots: the Today shot is the Agenda view, `next dev`'s error overlay is hidden in
  shots, and narrow-window and lapsed-Google shots were added.
- README: macOS 14 is the supported floor; Google Calendar's test-mode 7-day sign-in noted.

## [0.9.0] - 2026-09-23

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
- **Keeping audio for less time asks first.** Choose a shorter time in Settings → Recording
  and Nixon shows how many meetings and how much space will be cleared, then clears it right
  away when you confirm, and tells you what was freed.
- **Prep briefings show in the queue while they wait, run and finish.** Every brief Nixon
  prepares ahead of your meetings, or that you ask for by opening Prep or pressing
  Regenerate, appears as a row the moment it is planned. **Retry** on a failed one redoes
  just that one.
- **The call-detected alert now always arrives as a macOS notification, even with Nixon in
  front.** "Record this meeting?" shows in Nixon and as a notification at the same time, and
  answering it in either place clears the other. If macOS notifications are off for Nixon,
  the prompt says so and **Enable** takes you to the setting.
- **Nixon offers to record Teams and Google Meet calls too, not just Zoom.** When a Teams
  call starts, or a browser call starts while a meeting on your calendar is under way, you
  get the same "Record this meeting?" prompt, named for the app. A browser using your
  microphone with no meeting on your calendar (or one you hid from Today) is left alone.
  Only a Zoom call ending stops a recording; muting in Teams or Meet never does.
- **"Starting now" alerts with Google Calendar alone.** If Google Calendar is your only
  connected calendar, you now get the five-minute and "starting now — Join & Record" alerts
  too.

### Changed

- **Recordings take about a sixth of the disk space.** Audio you keep is stored compressed
  once a meeting has been processed.
- **"Immediately" is now "Once processed".** Audio is deleted after the meeting is
  transcribed and its speakers are identified, never before. If identifying speakers fails,
  the meeting page says so and the audio is kept for 7 days so you can retry.
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
- **"Auto-detect Zoom meetings" is now "Detect meetings"** in Settings → Recording. Your
  choice carries over.

### Fixed

- **Once processed now removes the separate microphone and system-audio tracks too**,
  including those kept from past meetings.
- **Meetings in a recordings folder you used before keep working.** After you change where
  recordings are saved, deleting one of your earlier meetings removes its recording too,
  and an interrupted recording in the old folder is still offered to resume.
- **A meeting whose transcript you paused processes itself when you stop.** It is
  transcribed and summarized on its own, like any other meeting, and **Process now** runs
  it by hand whenever you want.
- **Background summaries appear on the open meeting page** the moment they are ready, in
  place of "writing your summary".
- **Every meeting finishes processing**, transcribed and summarized, even when processing
  was started for it twice.
- **Meeting lengths include quiet endings and paused stretches.**
- **Hiding a meeting on Today also silences it**: no prep reminder, no join alert, and no
  summary spent working out what it was.
- **Only the alert that matters waits for you.** "A meeting is starting" stays on screen
  until you deal with it; every other Nixon banner clears itself after a few seconds.
  Set Nixon to **Alerts** in System Settings → Notifications and Settings will tell you the
  rest.
- **A meeting is summarized, and renamed, once when it ends.**
- **Ending a meeting goes straight to the meeting page**, without a "Welcome to Nixon!"
  flash on the way.
- **Faces on every meeting on Today.** A meeting recorded without a calendar invite shows
  the people named on it, the way All Meetings does.
- **The meeting you are recording is marked in the Week view** too, and appears on Today
  as soon as recording starts.
- **A meeting you added in Nixon says whether it was recorded**, and offers Record only
  until it ends.

### Internal

- Branch-review fixes: meeting detection reads the calendar and the browser window title
  only while detection is on and Nixon isn't recording (a confirmed Meet call is held, and
  Zoom's end still stops a recording). The abandoned-recording guard counts `.opus` channels
  as audio. Row recovery after a same-volume rename matches a folder stored through a
  symlink. `decode_stats` reaps ffmpeg on a read error. A refused folder change removes
  every folder it created, and the gather never creates a missing folder the owner chose
  (it reports it as blocked). `save_transcript` no longer recreates a missing folder.
  ADR-0013 documents the folder lease.
- Spec 0074 W4: the queue panel renders prep-brief queue rows (`LlmActivityView.queued` →
  Waiting, successful `prepBrief` history → Done, capped at 5) alongside the existing backlog
  rows; **Clear finished** now also calls `api_llm_activity_clear_finished`. Carried fix
  (Ruling 11): `NotificationPermissionRow` now treats a missing/failed capability check as
  unsupported instead of crashing on `.supported`, fixing 4 unhandled `pnpm test` rejections
  from unrelated settings fixtures that stub every `invoke` call.
- Spec 0072 W3: the stop path calls `api_finish_audio_processing` instead of diarizing
  inline; the meeting page reads `api_meeting_audio_status` and follows
  `meeting-audio-state-changed`. The demo fixtures seed one meeting per `audio_state`, and
  the screenshot mock answers the lifecycle commands (new `retention=` URL param).
- Spec 0072 W2: compression is on (`lifecycle::COMPRESSION_ENABLED`). Every channel reader
  resolves `.wav` then `.opus` (`audio/channel_files.rs`: diarization, owner turns,
  retranscription channel tags, the recordings-root fallback scan); `.opus` decodes through
  ffmpeg at 16 kHz mono. A resume scopes compressed channels (`system_seg00.opus`) and joins
  channel segments by decoding. `find_audio_file` (now `audio/meeting_audio.rs`) never picks
  a channel file and mixes a channels-only folder. The mix is written at 64 kbps
  (`MIX_AAC_BITRATE`). Capture always saves the mix whatever the retention setting. The
  backfill skips a meeting whose compression failed until the next launch. A unit test
  checks the bundled ffmpeg has `libopus`.
- Spec 0072 W1: the audio lifecycle (`audio/lifecycle/`). A migration adds
  `meetings.audio_state` (NULL/processed/failed/purged) and `speakers_identified_at`, and
  backfills `processed` for every meeting not awaiting transcription. Rust writes the state
  where processing finishes: diarization's terminal outcome (`launch.rs`), the backlog
  clearing `defer`, retranscription, import, and a resume (which resets it). One pure
  `disposition()` decides keep/compress/delete for the post-processing hook, the startup +
  hourly sweep, "apply now" and the dry-run preview. The sweep and the channel compressor
  (Opus 24k, verified by decoding, both channels or neither) take the folder lease with
  `try_acquire` and re-read the folder, skipping a meeting the mover or a recording holds
  (0073 W4). The retention preference is now `audio_retention`, derived once from
  `auto_save`/`retention_days`. The backlog predicate is one shared SQL fragment and now
  lets a transcribed silent meeting go. `audio/retention.rs` is gone (its daily sweep
  returned early whenever no day count was set). Compression is wired but switched off
  (`COMPRESSION_ENABLED`) until W2 teaches the channel readers `.opus`. New commands:
  `api_finish_audio_processing`, `api_preview_audio_retention`,
  `api_apply_audio_retention_now`, `api_meeting_audio_status`; event
  `meeting-audio-state-changed`.
- Spec 0072 W0: codec round-trips for the eval harnesses (`tests/eval_codec/`).
  `NIXON_EVAL_CODEC=opus<k>|flac` scores diarization after a channel re-encode and
  `NIXON_WER_CODEC=aac<k>` scores transcription after a mix re-encode. The gate chose Opus
  24 kbps for the channels (macro DER 7.93% vs 8.07% baseline, every pin held) and AAC
  64 kbps for the mix (Parakeet pooled WER 10.6%, vs 10.7% at today's 192 kbps). Numbers are
  in ADR-0014.
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
- `notif_remove` command (capability-gated like every UNUserNotificationCenter call) backs
  the detected-call prompt's cross-dismiss; `ZoomAutoDetect` is now `MeetingAutoDetect` and
  reads an optional `platform` from the still-`zoom-meeting-*` events (specs/0074 W5).
- Meeting detection moved from `zoom/monitor.rs` to `meeting_detect/` (specs/0074 W6): a pure
  `classify` over Core Audio's client-process list (pid, bundle id, running-input) plus the
  Zoom helpers, a calendar corroboration probe, and an Accessibility window-title check that
  runs only when Accessibility is already granted. Events renamed to `meeting-detected` /
  `meeting-ended` with a `platform` field. The Teams and browser signals are provisional
  until the `meeting_detect_spike` diagnostic (an ignored test) is run during real calls.

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
