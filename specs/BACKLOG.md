# Backlog

**Raw intake.** Any loose idea lands here first with a date and enough context to pick it
up cold. When an item is prioritized it gets **promoted to [`ROADMAP.md`](../ROADMAP.md)**
(Now/Next/Later/Someday), and when it's picked up for build it graduates to a numbered spec
(`/spec` — see [`INDEX.md`](INDEX.md)). Don't plan here; plan in the roadmap. Shipped items
move to the [Resolved log](#resolved-log) below. Newest at top.

Entry format: title, **Reported:** date + source, a few lines of context with file/spec
pointers, open questions.

---

## Live items

### `clean_run.sh` (Definition-of-Done gate #3) launches against PRODUCTION data

**Reported:** 2026-08-18 (found while smoke-testing 0052)

`clean_run.sh` ends with `pnpm run tauri dev`. In `frontend/package.json` that resolves to
`"tauri": "tauri"` — the **raw** Tauri CLI, which uses the base `tauri.conf.json` and
therefore the production identifier `ai.vinyl.app`. The isolated path is
`"tauri:dev": "node scripts/tauri-auto.js dev"`, which merges `tauri.dev.conf.json` to get
`ai.vinyl.app.debug`. `dev-gpu.sh:170` correctly calls `pnpm run tauri:dev`; only
`clean_run.sh` is wrong.

Confirmed from the startup log:
`app_lib::database::manager INFO Tauri DB path: .../ai.vinyl.app/meeting_minutes.sqlite`

**Why it matters:** CLAUDE.md lists "the app still launches via `./clean_run.sh`" as DoD #3,
so following the documented gate reads and writes the user's real meetings — defeating the
ADR-0004 isolation guarantee. It also applies the *branch's* migrations to the production DB.
That is not cosmetic: `database/manager.rs:54` runs `sqlx::migrate!` without
`set_ignore_missing`, and sqlx-core 0.8.6 (`migrate/migrator.rs:40`, default `ignore_missing:
false` at :51) returns `MigrateError::VersionMissing` when the DB carries an applied migration
the binary lacks. So after running the gate on a feature branch, the *released* `Vinyl.app`
fails to initialize its database until that branch ships. This happened for real on
2026-08-18 with 0052's `20260818000000`; it was reverted by hand.

**Fix:** change `clean_run.sh` to `pnpm run tauri:dev`. Consider also making `"tauri"` itself
refuse a bare `dev` subcommand so the raw CLI can't be reached by accident.


### 0041 code-review follow-ups (valid, deferred)

**Reported:** 2026-07-08 (the 0041 `/code-review` pass — confirmed correctness findings were
fixed on the branch; these are the verified cleanup/efficiency/architecture leftovers).

- **Dedup:** `LinkMeetingPicker` hand-rolls a filterable list dialog — use the existing cmdk
  `CommandDialog`/`CommandInput`/`CommandItem` (`components/ui/command.tsx`) for keyboard nav
  parity with ⌘K. Its `api_get_meetings` loader is the ~6th inline copy (also
  `meetings/page.tsx`, `CommandPalette`, `SidebarProvider`, `MeetingIdentityHeader`) — extract
  a shared `useMeetings()` + `DashboardMeeting` type (same story as the `usePeople()` item).
  The iCalUID collapse exists twice (`google/sync.rs::dedup_rows_by_ical_uid` keyed
  organizer-first vs `eventkit.rs::dedup_by_external_id` first-seen) — share one generic
  `(uid, starts_at)` collapse so the dismiss-key invariant can't drift; ideally enforce at
  `build_agenda`, the layer every reader crosses. `VirtualizedTranscriptView` has two
  near-copies of the optimistic set→write→revert overlay (`reassignSpanTo` /
  `reassignSegmentOptimistic`) — one `applyOptimistic(ids, patch, write)` helper.
- **Efficiency:** the prep matcher's title arm (`LOWER(TRIM(title))`) is unindexable — add an
  expression index if prep opens ever feel slow at scale. Speaker-correction reconcile
  refetches the whole loaded transcript window per correction (multi-thousand rows on long
  meetings) — patch-only or page-scoped reconcile. Retranscribe through the new in-place
  `refetch` shows no loading state and can leave a longer rewrite's tail behind
  lazy-scroll-load — acceptable today, revisit with the notes/retranscribe UX.
- **Architecture (watch, don't build yet):** the WS2 re-summary trigger is hard-wired to
  diarization completion; when "Transcribe now" rewrites need the same staleness handling,
  generalize to a `summary_inputs_changed(meeting_id)` invalidation instead of a second
  bespoke trigger. `ScheduledRecordControl` decides continue-vs-start from threaded props,
  mirroring backend adoption rules in JSX — more weight for the ROADMAP's conditional
  "backend-owned recording-session state machine" item.
- **Pre-existing, surfaced by the review:** the calendar-connect convergence loop (optimistic
  authorized + bounded retry to defeat EventKit's post-grant lag) died with the Today-view
  switch in 0036 — first connect can flicker empty until the next poll/focus; consider
  re-homing into `ConnectCalendarNudge.onConnected`. A `MIN_ITEM_PX`-floored meeting in the
  last minutes of the visible window paints ~25px past the timeline's bottom gridline into
  the scroll padding (cosmetic, pre-dates 0041, slightly improved by it).

### Diarization: raw clustering threshold + MERGE_FLOOR are the real accuracy levers

**Reported:** 2026-07-08 (the 0041 ground-truth eval sweep — measured findings, feeds
`specs/0039`). The eval harness (`tests/diarization_tuning.rs`, `eval_ground_truth_sweep`,
3 local Zoom recordings with speaker-labeled VTTs) showed the consolidation floor is a
**second-order knob** (≤1.2pp DER swing); two upstream parameters dominate:

1. **`DEFAULT_AUTO_THRESHOLD = 0.8`** (`sherpa.rs:161`, unchanged since 0011) produces
   impure raw clusters — on the 4-speaker file the dominant raw cluster is only ~55% pure
   (701s mixing 3 voices), and no post-hoc consolidation can repair that. Measured
   centroid sims on real audio: same-speaker median 0.51, distinct-speaker median 0.07
   with a tail to 0.62. Re-tuning this threshold against the harness is the
   highest-leverage accuracy work available. Sweep knob already in the harness.
2. **`MERGE_FLOOR = 0.30` lets the AtMost cap fuse real speakers:** distinct-voice pairs
   reach cos 0.62, so capping to the attendee count can yield the right cluster *count*
   with 98% of speech in one cluster (measured on the 4-speaker file).
3. Context: **AtMost(true n) seeding dominates everything** (macro DER 47.1% vs 55.0%
   Auto) — calendar-linked meetings are structurally better; anything that increases
   calendar linkage (0041 WS4 manual series links) indirectly improves diarization.

Baselines to beat (pinned in the regression gate): 43.6/41.1/56.5% per-file DER.

### Diarization regression: dominant speaker absorbs everyone (0039 WS1 consolidation)

**Reported:** 2026-07-08 (owner dogfooding v1.8.0). **Regression from a shipped change —
candidate for immediate fix; feeds `specs/0039`.**

Speaker ID got *worse* in 1.8: the first participant who talks a lot tends to get assigned
across the whole meeting. Prime suspect is the 0039 WS1 drift-consolidation pass (commit
`61ad921`): `CONSOLIDATE_FLOOR = 0.50` (`diarization/sherpa.rs:222`) with an **unbounded**
merge cascade (`consolidate_clusters` → `merge_closest_pairs(..., |_| true)`,
`sherpa.rs:932-968`). Same-gender/similar-timbre distinct voices commonly sit at cos 0.5–0.7,
and each merge duration-weights the surviving centroid toward the longest-talking cluster —
a snowball that pulls everyone into the dominant speaker. `specs/0039` line 474 predicted
exactly this failure mode of a too-low floor. It runs *before* the `AtMost` cap
(`sherpa.rs:1025-1040`), so the calendar/attendee seed can't undo an over-merge. Fix levers:
raise the floor, bound the cascade (respect the attendee-seeded count / cap merges per pass),
or require more than mutual-closest-≥-floor to fuse. Rule-out during diagnosis: confirm the
affected meetings didn't resolve `Fixed(1)` via `resolve_speaker_count`.

### Auto-summary races diarization → speakerless summaries + unresolved action-item owners

**Reported:** 2026-07-08 (owner dogfooding v1.8.0 — "summaries and action items are a lot
more useful after speakers are identified; what's the best way to auto-summarize?").

The two post-stop jobs are uncoupled and race: `useRecordingStop.ts:469-480` fires
`api_diarize_meeting` fire-and-forget, and the meeting-details page auto-generates the
summary as soon as it loads with `source==='recording'` (`meeting-details/page.tsx:148+`,
gate in `lib/auto-summary.ts:44-54`). The summary *does* use speaker labels — but only if
they're already in the DB (`summary/service.rs:427-456` prefixes `Name: text` when
`any_speaker`); diarization on a long call takes minutes, so the summary usually wins the
race and runs unattributed. The 0034 action-item extractor works over the summary text, so
assignees collapse to `assignee_raw`/unresolved — the exact "action items less useful"
symptom. Nothing ever re-runs the summary: `diarization-complete` is emitted
(`diarization/pipeline.rs:885`) but its only listeners refresh lists, and manual speaker
corrections don't re-summarize either.

Options (owner to pick a direction): (a) gate auto-summary on `diarization-complete` with a
timeout fallback; (b) generate immediately, then auto-regenerate once diarization lands —
action items are already regeneration-safe by design (0034); (c) hybrid: quick summary now,
badge "updating with speakers…", refresh when done. Open questions: also offer re-summarize
after manual speaker corrections? Double-LLM-run cost on cloud providers; local default makes
(b)/(c) cheap. Related: the "seed offline diarization from live labels" item below would
shrink the window.

### Record from a meeting detail page: doesn't open the call + mints a duplicate meeting

**Reported:** 2026-07-08 (owner dogfooding v1.8.0; the control shipped in v1.8.0 via commit
`2bfda08`, out-of-band under the 0038 label — no spec WS covers it).

Two defects in `ScheduledRecordControl`:
1. **Call never launches:** it calls `joinAndRecord` *without* `zoomUrl`
   (`ScheduledRecordControl.tsx:79-87`), and `joinAndRecord` only opens the meeting client
   when `event.zoomUrl` is set (`lib/calendar.ts:377-403`). The home-page path passes it
   (`DayAgenda.tsx:213-231`); the detail page never does — recording starts, Zoom doesn't.
2. **Duplicate row on retry after a false start:** once the first attempt promoted the
   scheduled row to `origin='recorded'` and wrote any transcript, both dedupe paths decline —
   `find_scheduled_for_occurrence` no longer matches (not scheduled) and
   `find_adoptable_calendar_meeting` excludes rows with transcripts by design
   (`repositories/meeting.rs:110-130, 238-257`) — so `api_create_meeting` falls through to a
   fresh INSERT (`api/api.rs:1296`). The right answer for "record this occurrence again" is
   probably the 0037 resume/continue path: on an already-recorded occurrence the control
   should offer *continue recording* (same `meeting_id`, new segment), not a fresh create.

Secondary: the detail path passes `startsAt = meeting.created_at` (`page-content.tsx:386`);
if unparseable it falls back to `Utc::now()` (`api.rs:1206-1219`), which can shift the
occurrence-day match.

### Prep briefs never find "previous meetings" (+ no manual way to associate)

**Reported:** 2026-07-08 (owner dogfooding v1.8.0 — never seen a prior-meeting summary in
prep, even with a same-title meeting the previous day; asked for manual recur/relate marking).

`find_prior_series_occurrences` (`repositories/meeting.rs:158-202`) matches by
`calendar_series_key` **exclusively** whenever the upcoming event has one — the
normalized-title fallback is an `else if`, shadowed for every calendar event with a series
key. A prior row only matches if it's `origin='recorded'`, has content, AND carries the
*identical* stamped series key — ad-hoc recordings (no calendar link → NULL key), one-off
events, and same-title-different-series meetings never match. On a miss the brief is skipped
entirely (`status='none'`, `aggregation/prep_jobs.rs:108-167`), so the owner sees nothing
rather than a degraded brief. There is **no manual association UI** — `set_calendar_series_key`
isn't even exposed as a Tauri command.

Fix directions: make title matching additive (union of series-key + normalized-title, maybe
attendee-overlap-scored) rather than a shadowed fallback; a manual "mark as related / link to
series" affordance on the meeting page (the owner explicitly asked for this — also covers
renamed recurring meetings); surface *why* prep is empty ("no prior occurrences found —
link one?") instead of silence. Spec home: extends `specs/0036`.

### Duplicate calendar events on Home + no way to dismiss them from the timeline

**Reported:** 2026-07-08 (owner dogfooding v1.8.0 — some meetings appear twice; the
duplicate can't be deleted).

Duplication: the same invite on two synced Google calendars produces two cached rows keyed
`gcal:{calendar_id}/{event_id}` with the same `ical_uid` — which is stored but **never used
for dedup** (`calendar/google/sync.rs:817-918`; `repositories/google_calendar.rs:206-222` is
a plain SELECT across all cached calendars; `build_agenda` emits one bubble per event row,
`calendar/day_agenda.rs:175-295`). With 30+ calendars auto-synced (next item) cross-calendar
duplicate invites are common. "Can't delete": the 0026 dismiss commands exist
(`day_agenda.rs:398-431`) but the only UI that calls them is the **unmounted**
`Calendar/DayAgenda.tsx` — the live Today timeline (`app/page.tsx` `TimelineBlock`) has no
hide/dismiss affordance at all. Fix: dedup reads by `ical_uid` (the dismiss key is already
`ext:{iCalUID}@{starts_at}`, so dismissal semantics survive), and add a dismiss control to
the timeline block. Note the EventKit path has the same no-dedup shape (`eventkit.rs:299-461`).

### Google Calendar connect: 30+ calendars all sync by default

**Reported:** 2026-07-08 (owner dogfooding v1.8.0).

`sync_calendar_list` upserts every `calendarList` entry on connect
(`calendar/google/sync.rs:576-626`) and `selected` defaults to `1` (migration
`20260704000000_add_google_calendar.sql:25`; `upsert_sync_state` never sets it), so all ~30
visible calendars immediately sync. Owner rule: **if more than 5 calendars, default all to
unselected and let the user pick** (sensible refinement: always pre-select only the primary
calendar). Settings also lacks a bulk select/none toggle (`CalendarSettings.tsx:492-510`).
Directly amplifies the duplicate-events item above. Spec home: amends `specs/0032`.

### Today timeline: false left-right stagger + bubbles overshoot their end time

**Reported:** 2026-07-08 (owner dogfooding v1.8.0 — back-to-back meetings (11:00 end /
11:00 start) render staggered as if overlapping; bubbles look too tall).

One root cause, two symptoms: `itemHeightPx` floors every bubble at `MIN_ITEM_PX = 34` at
1px/min (`lib/today-timeline.ts:21-23, 108-117`), so any meeting under 34 min renders past
its true end time — and `assignLanes` (`:272-314`) deliberately measures overlap on the
*rendered pixel interval*, so the inflated bottom collides with the next meeting's 11:00 top
→ lane split (`page.tsx:170-197`). The time comparison itself is exclusive/correct; the
min-height inflation is the bug. Fix directions: assign lanes on true time intervals (only
stagger real overlaps), then resolve *visual* crowding of adjacent short meetings separately
(denser px/hour or slight push-down), per the owner's "more spacing in the timeline" note.
Behavior is pinned by `lib/__tests__/today-timeline.test.ts` — update the expectations.

### Home: date selector + quick actions should stick while the agenda scrolls

**Reported:** 2026-07-08 (owner dogfooding v1.8.0).

The date-nav + Day/Week + Add-to-do toolbar (`app/page.tsx:726-818`, added by 0038 WS4)
lives *inside* the `overflow-y-auto` scroll region (`:724`), so it scrolls away; the greeting
header (`:680-721`) is already a fixed flex sibling. Fix: lift the toolbar out as another
`flex-shrink-0` sibling, or `position: sticky; top: 0` with a background — minding the
`max-w-[780px]` centering wrapper (`:725`).

### Live transcript: "Jump to latest" doesn't lock follow mode

**Reported:** 2026-07-08 (owner dogfooding v1.8.0). **Third report of this pain** (0019
WS1.2 hysteresis, 0029 WS4.1 jump affordance, 0038 WS6 rework) — the current design
re-attaches by proximity instead of by user intent.

`scrollToBottom` (`hooks/useAutoScroll.ts:74-86`) is a one-shot `scrollTop = scrollHeight` +
`setAutoScroll(true)`; the next scroll event re-evaluates `nextAutoFollow` against an
`AT_BOTTOM_THRESHOLD_PX = 8` re-attach window (`lib/auto-scroll.ts:16-32`) while content is
growing under the viewport, so follow drops almost immediately. There are also two
inconsistent "at bottom" thresholds (100px imperative check vs 8/24 hysteresis,
`useAutoScroll.ts:22-23` vs `auto-scroll.ts`). Fix: make Jump-to-latest set a **sticky
user-intent follow flag** that only an explicit *user* upward scroll (wheel/trackpad/keys —
not programmatic or content-growth scroll events) clears; unify the thresholds while there.

### Speaker reassignment jumps the transcript back to the top

**Reported:** 2026-07-08 (owner dogfooding v1.8.0 — every assign/swap loses the scroll
position; owner has to scroll back down each time).

Every correction path awaits a full `onRefetchTranscripts()`
(`MeetingDetails/TranscriptPanel.tsx:89-131`; `useSpeakers({ onMutated })` `:82-85`), which
replaces the whole segments array — if the paged window resets to page one, the virtualizer's
total size shrinks and `scrollTop` clamps to 0. Ironically the span-selection overlay in
`VirtualizedTranscriptView.tsx:398-465` was built scroll-safe (render-time overlay, never
mutates `segments`) and the refetch then defeats it. Fix: patch the mutated segments in
place (optimistic update, no full refetch — the server result is known), or preserve the
scroll anchor/pagination window across refetch; also verify the container isn't keyed on
something that changes with speaker identity. Spec home: `specs/0039` WS2 (span-level
correction) should absorb this.

### Meetily icon still shows on the "recording started" notification — survives reinstalls

**Reported:** 2026-07-08 (owner dogfooding v1.8.0). Supersedes the 0029 WS6.3 diagnosis
(ROADMAP "Awaiting verification" calls it environmental, fix = rebuild + `lsregister` reset) —
**multiple production re-installs did not clear it**, so it needs a real remediation.

Mechanism: the Rust-side notification sets only title/body, no icon
(`notifications/system.rs:34-41`), so macOS renders NotificationCenter's *own cached* app
icon keyed by bundle id `ai.vinyl.app`. Early Vinyl builds shipped under that id while
`icons/` still held Meetily artwork (id set in `96a8519`, icon swap Jun 27), seeding the
cache; reinstalling a correctly-iconed bundle never evicts it. All current bundle assets are
Vinyl-branded (no meetily-named assets remain). Fix levers: a documented/scripted one-time
cache reset (`lsregister -f` on the app + kill `NotificationCenter`/`usernoted`), and/or set
an explicit icon on the notification if `tauri_plugin_notification` supports it on macOS.
Verify on the production install, then update the ROADMAP line.

### Flaky-by-environment VAD fixture test (pre-existing, fails on main)

**Reported:** 2026-07-08 (0041 gate run). `vad_filter.rs:343`
`tuned_config_beats_old_strict_config_on_adversarial_fixtures` fails identically on `main`
and `integration/0041`: it asserts the OLD (pre-0004) VAD config drops at least one
`say`-generated fixture entirely, and no fixture is dropped anymore — a macOS voice/`say`
output change likely invalidated the premise, since the fixtures are synthesized at test
time. Not a product bug (the *tuned* config still passes its own assertions). Fix: pin the
fixture audio (record once, commit small WAVs) or weaken the comparative assertion to the
tuned-config guarantees. Check whether hosted CI's `say` still passes it (macOS runner
version skew).

### 0037 code-review follow-ups (valid, deferred)

**Reported:** 2026-07-05 (the `specs/0037` `/code-review` pass — the confirmed correctness /
data-loss findings were fixed in the PR; these are the deferred robustness/efficiency/dedup items).

- **Per-channel WAV rename-after-the-fact race (altitude):** `system.wav`/`mic.wav` are written
  under fixed names by the pipeline then RENAMED to `*_seg{NN}.wav` in `stop_and_save`. This races
  the writer's flush and can orphan an unscoped WAV if the process dies between write and rename.
  The `.mp4` path already avoids this by telling `IncrementalAudioSaver` its segment name up front —
  do the same for `channel_writer` (pass the segment filename) instead of renaming post-hoc.
- **Efficiency:** the create-before-start reorder adds an `api_create_meeting` round-trip *before*
  the mic tap opens (can clip opening audio) — mint the meeting id client-side and fire the INSERT
  concurrently. The stop-time FFmpeg concat runs three blocking `std::process` calls sequentially in
  an async fn (wrap in `spawn_blocking`, run the 3 disjoint concats concurrently), and re-concats
  ALL segments from scratch on every resume-stop (keep the canonical file and 2-input append the new
  segment, or defer to final stop). `scan_interrupted_recordings` fully parses each folder's
  `segments` array even though ~all are `status:"completed"` (deserialize `status` first, bail early).
- **Dedup:** `ffmpeg_concat` is a 3rd copy of the concat-demuxer invocation (extract one helper in
  `ffmpeg.rs` beside `find_ffmpeg_path`); the atomic temp+rename metadata write is a 4th copy
  (`write_metadata_atomic`); `append_transcripts_for_meeting` duplicates
  `save_transcripts_for_meeting`'s transaction scaffolding (one fn with an `allow_populated: bool`);
  segment-filename format strings repeat across ≥4 sites (one `segment_names(idx)` helper);
  frontend `armResumeRecording`/`consume` is a 3rd sessionStorage arm/consume-once copy (generic
  `sessionStash<T>`); `ResumeRecordingPrompt` hand-rolls relative time (reuse date-fns
  `formatDistanceToNow` as `TranscriptRecovery.tsx` does).
- **Minor:** `RESUMED_SESSION` global + `prior_audio_duration` could live on `RecordingManager`
  (a `is_resume()` getter) which already flows start→stop — a cleaner altitude than a process global
  (the correctness hole was closed by resetting at start). `max_audio_end_time` is now only used by a
  test — either wire it as the append-offset fallback or drop it. `segment_index` mirrors
  `segments.len()`; fold out the redundant field.

### Resume / continue an existing recording (crash recovery + "keep recording")

**Reported:** 2026-07-05 (owner testing the `specs/0036` Today view — asked whether you can
re-record a meeting that already has a recording, e.g. after a disconnect).

**Today there is no way to add to a finished recording.** Starting a recording *always* mints a
new meeting (`useRecordingStart.ts` → `api_create_meeting`), and the audio folder is named
`<title>_<YYYY-MM-DD_HH-MM>` (`audio_processing.rs::create_meeting_folder`), so a second pass of
the same meeting becomes a *separate* meeting + folder — you end up with two rows for one
conversation. On the Today view a `recorded` item doesn't even offer Join & Record
(`canJoinAgendaItem` requires `!recorded`); it routes to *open details*. `resume_recording`
(`audio/recording_commands.rs`, tray "Pause ▸ Resume") is **intra-session only** — it hard-requires
`IS_RECORDING == true` and the in-memory `RECORDING_MANAGER`, so it can't resume a stopped meeting
or survive an app quit.

**Already handled (don't rebuild):** a mid-session *audio device* drop (AirPods/Bluetooth) is
covered by the reconnect path (`get_reconnection_status` / `is_reconnecting`) — the meeting keeps
going. The gap is "the recording *stopped* and I want to continue the same meeting."

**The data model is already resume-friendly** (so this is not a rewrite): transcripts are
append-only INSERTs keyed by `meeting_id` (`transcript.rs`); `incremental_saver.rs` checkpoints
transcripts to disk during recording (crash durability); `diarization/folder_match.rs` can recover
a meeting's WAV folder.

Really two related features:
1. **Crash/quit recovery** (highest value, least surprise): on relaunch, detect an interrupted
   recording from its checkpoint folder and offer *"Resume this recording?"*.
2. **"Continue recording"** action on a recorded meeting's detail page for the "I stopped but the
   meeting kept going" case.

**Missing pieces for either:** (a) a start path that **reuses the existing `meeting_id`** instead of
creating one; (b) **audio continuity** — a second WAV segment in the *same* folder (multi-segment)
rather than a new meeting, with the offline diarization WAVs (`system.wav`/`mic.wav`) handled per
segment; (c) re-running the **offline diarization + summary over the combined** transcript set, not
just the new tail (`pipeline.rs::run` currently does `clear_meeting_speakers` + full recompute — must
cover all segments).

**Open questions:** append into one continuous WAV vs. keep numbered segments and concatenate for
playback? How does resume interact with the `specs/0036` scheduled-row adoption (a resumed
occurrence should stay the same meeting object)? Should crash-recovery auto-resume or always prompt?
Time-gap limit before "resume" instead offers "new recording" (resume a 5-min-old drop, but not
yesterday's)? Natural companion to `specs/0036` and the persistent meeting-start notification item
below.

### 0036 follow-ups (deferred in the first cut)

**Reported:** 2026-07-04 (during the `specs/0036` build — scoped out to keep the first cut
focused; none block the feature).

- **Prep-notes full-text search:** prep notes are stored/edited/shown but NOT yet indexed in
  ⌘K — the `meeting_notes_fts` external-content table would need a rebuild to add the
  `prep_markdown` column. The core "what did we decide" search is already served by the
  summary FTS; prep-agenda search is the nice-to-have. Do the FTS rebuild migration when
  picked up.
- **Settings toggle for background brief generation** (`prep_autogenerate`): the generator is
  always-on per the owner decision. A Settings switch to disable background generation (for
  zero background egress on a cloud provider) needs a settings-table column + a Settings UI
  control — deferred.
- **"Prep ready" badge on the Today timeline:** the upcoming-item Prep chip is static; showing
  a real brief-readiness state needs a prep-status flag added to `api_get_day_agenda` (a small
  Rust extension, deliberately skipped in v1).
- **GC of un-recorded `scheduled` rows:** an occurrence that's prepped but never recorded
  leaves an empty `scheduled` meeting row. It is now excluded from all lists + the agenda (the
  `origin <> 'scheduled'` filters added in the code-review pass), so it's invisible, but the row
  lingers. Fold a sweep for stale empty `scheduled` rows into the retention sweeper.

### 0036 code-review follow-ups (valid, deferred)

**Reported:** 2026-07-04 (the 0036 `/code-review` pass — the correctness findings were fixed
in the PR; these are the deferred efficiency/dedup items).

- **Efficiency:** `find_prior_series_occurrences` (a triple-EXISTS correlated-subquery scan)
  runs twice per event per background pass — `ensure_brief_for_event` computes `prior` only to
  test emptiness, then `generate_brief_for_target` refetches it + metadata; the same
  double-lookup is on the `api_get_prep` → spawn path. Pass `prior`/`meta` down instead.
  Also `source_fingerprint` is an N+1 serial `SELECT updated_at` per prior id (bounded to 2,
  but collapsible to one `IN (…)` query), computed even when the cached brief is fresh.
- **Dedup:** `generate_brief_for_target`'s provider-config + budget + LLM-closure block is a
  near-copy of `api_ask_ai_run`'s spawned task — extract a shared `build_summary_llm(...)`.
  The `PREP_RUNS` registry is a third copy of the run-registry primitive (`ASK_AI_RUNS`,
  summary `CANCELLATION_REGISTRY`) — extract a shared `RunRegistry`. The `Stage`→wire mapping
  is duplicated (`stage_str` vs `progress_payload`); put it on the `Stage` enum.
- **Frontend dedup:** `PrepNotesEditor` clones `NotepadPanel`'s debounced autosave (extract a
  `useAutosaveNotes(meetingId, command)` hook); the `api_list_people` + `nameById` loader in
  `PrepPanel` is the ~7th inline copy (extract `usePeople()`).
- **Minor (accepted):** `api_create_meeting`'s scheduled-row adoption returns early and ignores
  a passed `folder_path`/`template_id` — unreachable via the calendar Join & Record path (which
  passes neither), so left as-is; revisit if a non-calendar flow ever pre-creates a scheduled row.


### 0035 review follow-ups (valid, deferred at build time)

**Reported:** 2026-07-03 (the 0035 `/code-review` pass — findings verified but deferred;
the confirmed correctness/privacy findings were fixed in the 0035 PR itself).

Cleanup/efficiency items, roughly by value (pruned 2026-07-04 when the preview/consent
gate was removed — the debounced-preview efficiency items went with it):
- **Gather efficiency:** batch `select_doc_body`'s per-meeting queries (≤3×10 serial
  point reads per run) into 2–3 `IN (...)` queries; stop building the full numbered
  corpus twice per token estimate (`estimate_run_tokens` / `fits_single_pass`) — per-doc
  sums + fixed overhead suffice for the estimate.
- **Engine API sharpness for the next consumer (0013 3b/4b):** the engine's default
  `cited` check is weaker than `postprocess_citations` (misses `[m1]`, comma groups);
  either move normalization into the engine or stop computing `cited` there. Also consider
  moving `AggregationScope` out of the aggregation module so `database/` stops importing a
  feature module.
- **Frontend dedup:** person-filter dropdown + `api_list_people` load is a near-copy of the
  tasks hub's (extract `PersonFilterDropdown`/`usePeople`); `AnswerMarkdown` is a third
  hand-rolled markdown mini-parser (unify the line/inline tokenizer with `notes-html.ts` —
  their italics rules already diverge). The run-id buffering machinery on /ask could vanish
  entirely by minting the run id client-side (`crypto.randomUUID()` passed to the command).
- **Rust dedup:** the Ask-AI cancellation registry mirrors the summary one with subtly
  different lock-poisoning semantics — extract a shared `CancellationRegistry`; promote the
  meeting/transcript/summary fixture builders duplicated across `fts_search.rs` and
  `aggregation_engine.rs` into `tests/common/`; route gather's inline
  `summary_processes`/`meeting_notes` reads through the owning repositories.

### Persistent meeting-start notification with "Join & Record"

**Reported:** 2026-06-29 (`specs/0019` WS6.6 — deliberately not built there; an in-app
banner was rejected as clutter).

When a calendar meeting's start time arrives, fire an OS notification ("meeting has
started") with a Join & Record action that **persists until interacted with or dismissed**.
Today `CalendarAlerts.tsx` fires a one-shot native notification 2 min before start; the
persistent sonner toast is Zoom-process-driven, not clock-driven. Needs its own small spec
(clock-driven trigger + notification-action wiring into the pending-join path). **Natural
companion to `specs/0036` (Pre-call prep + Today view)** — the notification's action could
deep-link into the meeting's Prep tab; kept out of 0036's scope but should be sequenced with
it.

### Seed offline diarization from live labels (cut post-meeting latency)

**Reported:** 2026-06-29 (`specs/0019` WS2.6 — investigated & deferred, no code change).

Live diarization labels (`diarization/live.rs`) are thrown away at stop; the offline pass
recomputes everything from scratch (`pipeline.rs::run` after `clear_meeting_speakers`).
Opportunity: seed the offline pass from the live registry, or skip the recompute when live
coverage is high-confidence. Design investigation first — must not break the authoritative
offline pass or the sticky manual-override semantics (0029 WS3.2 lesson).

### Fork-cleanup tier 6: verify-then-delete judgment calls

**Reported:** 2026-07-01
([fork-deprecation audit](../docs/audits/2026-07-01-fork-deprecation-audit.md) §PR 6;
follow-on to `specs/0031`).

Each needs a quick "was this an unfinished feature?" check against this backlog/roadmap
before deleting: deprecate-tier Tauri commands (notifications command surface,
standalone system-audio wrappers, the device-reconnection trio — possibly an unfinished
AirPods-reconnect feature); verify-first frontend orphans (`LegacyDatabaseImport`,
`Calendar/UpcomingMeetings`, `BluetoothPlaybackWarning`, `useProcessingProgress`, `zod`);
the write-only `transcript_chunks` path (untangle the `summary.rs:78` JOIN first); the
`dasp` crate; `scripts/inject_transcript.py` (rebrand header or delete). Also tracked on
the roadmap's engineering-health track.

### `Fixed(1)` diarization short-circuit (micro-optimization)

**Reported:** 2026-06-25 (old ROADMAP Phase 3); demoted here by the
[2026-07-01 roadmap review](../docs/reviews/2026-07-01-roadmap-review.md) §1.

When a calendar-linked meeting has exactly one remote attendee,
`resolve_speaker_count` already forces `Fixed(1)` — the embedding/clustering pass in
`diarization/sherpa.rs` could be skipped entirely. The user-visible half of the old
"auto-name the 1:1 speaker" idea is already covered (0016's conservative auto-label +
0029's `transcripts.channel` "You" attribution); this is purely a compute saving.

### Notes-grounding prompt tuning + residual `any`s

**Reported:** 2026-06-24 (old ROADMAP Phase 2 leftover, `specs/0003`).

Revisit notes-grounding prompt strength in the summary pipeline after more real-meeting
use (does the summary lean on the user's notes enough?), and type the residual
`no-explicit-any` warnings left by `specs/0005` in the notes/summary components.

---

## Resolved log

History of backlog items that shipped (kept for traceability).

### ~~Join & Record should adopt the calendar meeting's identity~~ ✅ Shipped

**Reported:** 2026-06-26 · **Resolved:** v1.0.0 (`specs/0015` — adopts the event's title,
`calendar_event_id`, and attendees), then hardened in v1.1.0 (`specs/0019` WS6.3), v1.2.0
(`specs/0024` WS2.1 single-row safety net), and v1.3.0 (`specs/0029` — hand-off armed
before Zoom opens; attendee seeding on every adoption path).

### ~~Delete a started/recorded meeting~~ ✅ Shipped

**Reported:** 2026-06-26 · **Resolved:** v1.0.0 (`specs/0015` — delete with confirm from
the meeting page, the All-meetings list, and Home; removes transcript/notes/summary/
speakers and the on-disk recording folder, guarded to the recordings directory).

### ~~Notes-only meeting (no audio capture)~~ ✅ Shipped

**Reported:** 2026-06-26 · **Resolved:** v1.0.0 (`specs/0015` — "New note" on the sidebar
+ Home creates a no-audio meeting with the notepad; notes-grounded summary, no Transcript
tab).
