# Nixon — Roadmap

Local-first macOS meeting assistant, forked from meetily. Quality bar: granola.ai.
(`Nixon` = `APP_NAME`, a variable — see `docs/decisions/ADR-0002`.)

## How to read this

This file is the **single source of truth for what we build next**. Ideas flow:

1. **`specs/BACKLOG.md`** — raw intake. Any loose idea lands there first.
2. **This file** — prioritized plan. Backlog items get promoted into Now/Next/Later/Someday.
3. **`specs/`** — numbered specs, written when an item is picked up (`/spec`).
   **`specs/INDEX.md`** lists every spec with its status. Lasting decisions get an ADR in
   `docs/decisions/`.

Releases are driven by **real-meeting dogfood feedback batches** (1.1 ← `specs/0019`,
1.2 ← `specs/0024`, 1.3 ← `specs/0029`), so Now/Next is re-reviewed at each release.
The current ordering adopts the recommendations of the
2026-07-01 roadmap review: three rounds of
feedback say *finish the half-built advertised features first* (search, templates —
each asked for twice), *fix calendar fidelity* (the most persistent pain), and *don't
start the zero-pull items* (topics, analytics, pre-call prep) yet. Its open questions
are tracked at the bottom (owner).

---

## Now (in flight, 2026-07-01)

- **1.6 dogfood feedback triaged (2026-07-06).** A full day on the 1.6 build produced a
  ~22-item batch, triaged into three specs (`specs/INDEX.md`): **`specs/0038`** (8-WS batch:
  action-items polish, Ask-AI history/saved questions, Google DL expansion + attendee photos,
  multi-day/week agenda, people starring + person roll-up, live-transcript UX, settings/chrome
  cleanup, meeting-row attendees), and two graduated items — **`specs/0039`** (diarization
  accuracy: long-meeting cluster drift, span-level correction, voiceprint-pollution guard) and
  **`specs/0040`** (notes as a first-class type, stub). All three are **Draft** pending
  prioritization here. Topic tags (owner raised again) stays its own already-queued item below.
- **⚠️ Privacy invariant violation found while authoring 0038 — fix ASAP.** `src/analytics/`
  is **live** PostHog telemetry (`posthog_rs` → `us.i.posthog.com`), registered in `lib.rs` and
  fired by callers (e.g. `LanguageSelection.tsx`). This is an unsanctioned outbound path and
  contradicts "meeting data never leaves the machine" — rip it out (0038 WS7 covers it, but it
  should not wait for the whole batch). Verify what it has been sending.
- **Fork cleanup** — executing the
  fork-deprecation audit as
  `specs/0031`: ~6.5–7k LOC of dead meetily inheritance (Windows scripts + 4.5 MB binary,
  orphaned components, 16 npm packages, the parallel-Whisper subsystem, dead Tauri
  commands), plus the two live hazards it found (conflicting tailwind/postcss configs;
  the broken `--process meetily` console feature). Small PRs, each green through `/check`.
- ~~**Google Calendar provider — spec + ADR**~~ — done: `specs/0032` + `ADR-0010`
  accepted 2026-07-02; the build shipped the same day (see Next #3).
- **v1.3 verification round** — the manual checks listed under
  [Awaiting verification](#awaiting-verification-needs-brians-machine); the ADR-0009
  signed-build Keychain check is being run today.

## Next (in order)

1. ~~**Finish templates** (`specs/0020`)~~ — **✅ implemented 2026-07-02** (re-grounded the
   same day): Settings → Templates editor (edit/add/delete + built-in overrides), all six
   templates registered, active-template label, confirm-before-regenerate, auto-select by
   title, and the persisted per-meeting template honored on every generation path (the
   Day-Agenda/auto-summarize hardcode is gone). Manual smoke tracked under Awaiting
   verification.
2. ~~**Full-text search (FTS5)**~~ — **✅ implemented 2026-07-03** (`specs/0033`; split out
   of `specs/0021`, Ask-AI stays behind, see Later): External-content FTS5 over transcripts + summaries + notes
   with write triggers, `unicode61 remove_diacritics 2`, ranked snippets in the ⌘K palette.
   The search pill was reported dead in two of three feedback rounds; today it opens a
   palette that only matches titles. Design constraints from the review §4: index off
   *transcript writes* (record-only mode / "Transcribe now" rewrites), tolerate zero-transcript
   meetings, assert FTS5 is compiled in.
3. ~~**Google Calendar provider — build**~~ — **✅ implemented 2026-07-02** (pulled ahead
   of FTS5 by owner request; `specs/0032` / ADR-0010 accepted same day): OAuth PKCE
   loopback + Keychain-only tokens, incremental `syncToken` polling into a local event
   cache (focus/staleness/10-min-timer/manual triggers), single-active-source model
   (Google connected ⇒ Google only; else EventKit — owner simplification, same day) with
   cross-source dismissal-key continuity, `gcal:` attendee routing through
   Join & Record/adoption/participant seeding, Settings Calendar card (connect,
   per-calendar toggles, reconnect banner, privacy copy). Declined events hidden by
   default. Manual smoke + packet-capture egress check (spec tasks 7–8) tracked under
   Awaiting verification; needs the owner's GCP client id in the build env.
4. ~~**Action items v1: extraction + task hub**~~ — **✅ implemented 2026-07-04** (`specs/0034`). The
   biggest remaining gap vs. granola.ai; every prerequisite shipped in 1.0 (owners via
   `people`, participants, `calendar_event_id`, summary templates already elicit
   action-item sections). Must be **regeneration-safe by design**: re-extraction diffs
   against existing items; user-completed/edited items are never touched (review §4).
   Live-during-meeting detection is explicitly out of scope for v1.

**Conditional structural item:** if the v1.3 smoke round surfaces *another*
meeting-identity/lifecycle race (the recurring bug class across 0019/0024/0029), stop
patching and spec a **backend-owned recording-session state machine** (single source of
truth in Rust, frontend subscribes) before any new feature above. If 1.3 holds clean, skip
it (review §2).

**Planning invariant (new since 1.3):** record-only mode + deferred transcription mean a
meeting may have no transcript for days, and "Transcribe now" can rewrite transcript rows.
Anything that indexes or extracts from transcripts (FTS5, action items, topic suggestions)
must be event-driven off transcript writes, not meeting end. Also: `transcripts.channel`
makes owner talk-time computable without diarization — analytics got cheaper.

## Later

Sequenced per `specs/0013` Waves 3–4; none of these have demonstrated dogfood pull yet, so
they stay behind Next.

- ~~**Aggregation engine + cross-meeting Ask-AI**~~ (0013 Wave 3a; absorbs the Ask-AI half
  of `specs/0021`) — **✅ implemented 2026-07-03** (`specs/0035`): reusable
  gather→pack→map-reduce engine (recall-oriented FTS5 gather, summaries-first doc policy,
  meeting-granularity `[M#]` citations, injected-LLM design — topic roll-ups and pre-call
  prep become new prompt sets over it), Ask-AI as first consumer: `/ask` page + ⌘K action,
  provider follows the summary config, stage-progress events, cancellation. The
  pre-send preview/confirm gate was removed 2026-07-04 in dogfooding (spec amendment:
  provider choice in Settings IS the egress consent; Enter runs immediately). Manual
  smoke verified 2026-07-04.
- **Topic tags v1** — manual tags + filtering only (`topics` + `meeting_topics` join table
  per 0013 — *not* the unused `meetings.folder_path` column; don't build on it). Topic-level
  notes can ride this (reuses the `meeting_notes` shape). AI roll-up summaries per topic
  come after the aggregation engine.
- **Custom transcription dictionary** (`specs/0022`, drafted) — S-sized gap-filler for any
  release; the pain is real (STT wrote the old product name "Vinyl" as "vinal").
- **Meeting analytics v1** — four queries and one page: meetings/day, hours-in-meetings, my
  talk-time share (via `transcripts.channel`), top collaborators. Heat map + "consolidatable
  meetings" detection are v2 (Someday).
- **Pre-call prep + Today view** — **spec drafted 2026-07-04 (`specs/0036`, awaiting approval);
  promoted from Later on owner request.** A pre-generated brief over the previous 2 occurrences
  of a recurring series (a new `pre_call_prep_prompt` over the 0035 engine — richer than the
  original "last occurrence's summary" join), carried-over open action items, and a prep-notes
  agenda that feeds the post-meeting summary; plus the greenfield **Today view** — Home becomes
  an 8am–6pm timeline (Recent recordings removed, history in All meetings). Graduates 0013 Wave
  4b. Series detection uses a persisted `calendar_series_key` (EventKit `calendarItemExternalIdentifier`
  / Google iCalUID) with normalized-title fallback.
- **Zoom cloud-recording links per meeting** — v1 is manual entry (URL + passcode in the
  Keychain), which needs no ADR. Auto-population from the Zoom API waits for the Zoom OAuth
  ADR (see Someday).
- **Exports; onboarding polish.**

## Someday / parked

- **Zoom as a speaker-label source** *(0013 Wave 5 — explicitly lowest priority; zero
  feedback pull)*. When a meeting is Zoom **cloud-recorded** and accessible, fetch the
  name-attributed transcript via the Zoom API and use it to label speakers with **real
  names** instead of/over diarization clusters. Complements, doesn't replace, diarization —
  which stays the universal fallback (covers Meet/Teams, in-person,
  participant-without-recording-access, and local-first cases Zoom can't). Open problems:
  Zoom OAuth app + `cloud_recording:read`, a **privacy ADR** (content round-trips through
  Zoom cloud), and **reconciliation** of Zoom's transcript with our real-time Whisper
  transcript (clock alignment / grafting names onto our timeline). One ADR should gate both
  this and recording-link auto-population — same OAuth app, same privacy question (review
  §3). The live variant (Meeting SDK / RTMS active-speaker) is a much larger, Zoom-only
  architectural change — out of scope.
- **Analytics v2** — calendar heat map of meeting load; participant-set-similarity
  "these meetings could be consolidated" suggestions.
- **Live action-item detection** during the meeting (auto or manually flagged) — cut from
  action items v1; revisit once the extraction path has proven itself.

*(Dropped from the old Phase 4: "Cross-meeting reference & analysis" as a standalone line —
subsumed by Ask-AI + topic roll-ups, per review §3. Demoted to backlog: the `Fixed(1)`
1:1-speaker short-circuit — 0016's auto-label + 0029's `transcripts.channel` already cover
the user-visible part.)*

## Engineering health (parallel track)

- **Fork cleanup follow-on** (after `specs/0031` lands) — the audit's verify-then-delete
  tier (audit §PR 6): deprecate-tier Tauri commands (notifications surface, system-audio
  wrappers, device-reconnection trio — unfinished AirPods feature?), verify-first frontend
  orphans (`LegacyDatabaseImport`, `UpcomingMeetings`, `BluetoothPlaybackWarning`,
  `useProcessingProgress`, `zod`), the write-only `transcript_chunks` path, `dasp`. Each
  needs a "was this an unfinished feature?" check first — tracked in `specs/BACKLOG.md`.
- **Dependency bumps: Next.js 14→15, reqwest 0.11→0.12** — deferred from `specs/0028`;
  supervised sessions only, not to be attempted unattended.
- **Mixer timestamp alignment + cpal dedicated-thread refactor** — `TODO(0028)` markers in
  place; blocked on a real-audio soak test.
- ~~**Dev-build signing identity**~~ — **✅ done 2026-07-03**: dev builds are re-signed with a
  stable Apple Development identity via a `build.runner` cargo wrapper
  (`src-tauri/scripts/cargo-dev-sign.sh`, dev config only), ending both the Screen-Recording
  re-grant gotcha and the Keychain password prompts on rebuild (ADR-0004 §Dev-build gotchas).
- **Test strategy** (`specs/0023`, ongoing) — keep growing the lifecycle-regression harness
  and Vitest coverage alongside each feature.
- **Housekeeping:** the `upstream` git remote referenced by CLAUDE.md / `/sync-upstream` is
  not currently configured (audit §6; `SETUP.md` documents how).
- **Self-hosted macOS CI runner** (2026-07-04, follow-on to the CI cost trims) — the
  `rust` job in `ci-checks.yml` is ~all of the Actions bill: macOS runners bill at 10×
  and a cold-cache run is 20–40 real minutes per PR. Registering the owner's Mac as a
  self-hosted runner and pointing that job at it (`runs-on: [self-hosted, macOS]`) makes
  those minutes free, reuses a warm local target dir (faster than the cache dance), and
  is the only path that could ever CI-gate the Metal/model-dependent tests the hosted
  runner must skip. Scope when picked up: runner install/launchd service, workflow
  `runs-on` switch with hosted fallback, and hygiene (job workspace vs. the dev checkout,
  keeping `~/Library/Caches` model downloads shared). Security is a non-issue while the
  repo is private and all PRs are ours; revisit before ever accepting external PRs.

## Awaiting verification (needs the owner's machine)

- **0020 templates smoke** — edit a template in Settings → Templates, regenerate a summary
  with it; one-click summarize from the Day Agenda uses the meeting's picked template;
  recurring-title meeting auto-selects the prior template.
- **0032 Google Calendar verification** — needs the owner's GCP OAuth client
  (`NIXON_GOOGLE_CLIENT_ID`/`_SECRET` in the build env): connect flow end-to-end; spec
  task 7 (EventKit `calendarItemExternalIdentifier` ≡ API `iCalUID` on a real
  Google-synced calendar — dedupe + dismissal transfer depend on it); spec task 8
  (packet-capture egress boundary per ADR-0010); Workspace DL shows materialized members;
  Join & Record on a `gcal:` event seeds the roster.
- **ADR-0009 Keychain signed-build check** — the 0030 WS1 migration's manual verification
  pass on a signed build. *Being run today.*
- **0029 WS1.2 — active recording hangs Zoom's join** — audio-tap ↔ `coreaudiod` contention;
  needs runtime diagnosis. Zoom-first ordering remains the supported path meanwhile.
- **0029 WS6.3 — notification shows the old Meetily icon** — re-diagnosed (0041 WS8):
  Notification Center caches the app icon per bundle id and reinstalls never evict it, so the
  earlier "rebuild + `lsregister` reset" advice didn't hold. Remediation script shipped
  (`scripts/reset-notification-icon-cache.sh`); owner to run it once against the production
  install, then verify a recording-started notification shows the Nixon icon (ADR-0004).
- **v1.3 real-meeting smoke round** — 0029's verification pass, plus the 0028 items that
  can't be unit-tested (AtomicWaker RT callback, system-audio teardown, bounded queues under
  a forced falling-behind condition). *This round is also the trigger for the conditional
  state-machine item under Next.*

## Open questions (owner — from the 2026-07-01 roadmap review)

1. ~~**Action items ahead of demonstrated pull?**~~ Resolved 2026-07-03: yes — `specs/0034`
   approved and building; the aggregation engine is specced too (`specs/0035`) but builds after.
2. **Google Calendar OAuth-verification posture:** ship for personal use now (test-user
   mode, no Google verification) and gate public distribution later? → decide in ADR-0010.
3. ~~**Ask-AI provider default:**~~ Resolved 2026-07-03 (`specs/0035`): Ask-AI follows the
   configured summary provider rather than silently downgrading to local. (The blocking
   egress confirm that originally accompanied this was removed 2026-07-04 in dogfooding —
   provider choice in Settings is the consent; see the spec amendment.)
4. **If the 1.3 smoke round finds another meeting-identity race:** does the
   recording-session state machine jump ahead of the Next queue?

---

## Shipped (history)

Full detail lives in `CHANGELOG.md` and the specs (see `specs/INDEX.md`).

| Version | Date | Highlights (specs) |
|---|---|---|
| 0.1.0 | 2026-06-23 | Fork foundation, agent workflow, dev/prod isolation groundwork (`0001`) |
| 0.2.0 | 2026-06-24 | Rebrand to `APP_NAME` (`0002`), de-meetily + tech-debt purge: Python backend, `*_old.rs`, `audio_v2`, updater (`0006`), notes-aware summary flagship (`0003`), VAD over-gating fix (`0004`), lint cleanup (`0005`), UX redesign (`0007`), calendar + Zoom join/auto-stop (`0008`), audio test harness (`0009`) |
| 0.3.0–0.4.0 | 2026-06-24/25 | Speaker diarization P1 offline + P2 rename/merge/attendee pick-list (`0010`, ADR-0005) |
| 0.5.0 | 2026-06-25 | Live diarization (`0011`, ADR-0006), Day Agenda (`0008`) |
| 0.6.0–0.7.1 | 2026-06-25/26 | Warm Editorial reskin + app-flow redesign (`0014`), release plumbing prune |
| 1.0.0 | 2026-06-27 | Meetings as first-class objects: notes-only meetings, delete, Join & Record adopts calendar identity (`0015`); cross-meeting voice identity + People + voiceprint gallery (`0016`, ADR-0007); participants (`0017`); owner identity (`0018`); role-weighted summaries (`0012`) |
| 1.1.0 | 2026-06-29 | 1.0 feedback hardening: data-loss fixes, transcript-first naming (`0019`); CI gate + regression harness (`0023`); **Developer ID signing + notarization** (ADR-0008) |
| 1.2.0 | 2026-06-30 | 1.1 feedback: lifecycle/calendar/identity fixes (`0024`), vocal-range spectrometer (`0025`), dismiss calendar events (`0026`) |
| 1.3.0 | 2026-07-01 | 1.2 feedback: Zoom interplay, attribution, live surfaces, **record-only mode + deferred transcription + audio retention** (`0029`); robustness hardening (`0028`) |
| unreleased | — | 0028 deferred follow-ups: **API keys → Keychain** (ADR-0009), masked key IPC, ffmpeg digest pinning, backpressure/partial-summary visibility (`0030`) |

Old Phase 0–3 detail (foundation, rebrand, note-enhancement, diarization P1–P3) is retained
in the specs above; the phases are closed. Two Phase 2/3 leftovers moved to
`specs/BACKLOG.md`: notes-grounding prompt tuning and the `Fixed(1)` diarization
short-circuit.

---
Each feature starts as a numbered spec in `specs/` (`/spec`) and, where it sets lasting
direction, an ADR in `docs/decisions/`.
