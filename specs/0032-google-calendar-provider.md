# 0032 — Google Calendar Provider (OAuth, read-only)

- **Status:** ✅ Implemented (2026-07-02) — tasks 1–6 done and gated; tasks 7–8
  (real-account key-equivalence check + packet-capture egress verification) pending on
  the owner's GCP OAuth client (`VINYL_GOOGLE_CLIENT_ID`/`_SECRET`)
- **Amendment (2026-07-02, owner decision):** **single active calendar source** — while a
  Google account is connected it is the *only* source; disconnected ⇒ EventKit only.
  The merged view and cross-source dedupe were dropped as unneeded complexity. Where this
  document says "merge", read "source selection"; dismissal-key equivalence still holds
  (it now guarantees continuity across source *switches* rather than dedupe).
- **Owner agent(s):** rust-core-engineer (OAuth/sync/merge) + frontend-engineer (settings/agenda)
- **Roadmap phase:** "Next" slice per the 2026-07-01 roadmap review §5.2 (0013 Wave 2b,
  pulled forward from Phase 5)

## Context / Problem

Calendar/participant fidelity is the most persistent pain across all three feedback rounds
(0019 WS6.3, 0024 WS2.1/2.2, 0029 WS2.1/6.2), and its biggest unresolved item is blocked on
this integration:

- **Distribution lists render as one fake person** (`specs/0027`, 1.1 feedback note 1).
  EventKit returns a DL as a single `EKParticipant` (`eventkit.rs`, `attendees_from_event`)
  and never the members; `seed_from_attendees` (`meeting_participant.rs:167-215`) then mints
  one bogus roster row. 0027 was explicitly deferred "until a real Google Calendar connection
  exists". The Google Calendar API materializes individual members as they RSVP and performs
  server-side group-member listing — most of the roster fix, with no extra scopes (ADR-0010).
- **EventKit identity is flaky at the edges**: provider syncs reissue `eventIdentifier`
  (the 0029 WS6.2 dismissal-key bug, patched with the `ext:<externalId>@<occurrence>`
  dual-key scheme), and attendee lookup falls back to brittle title+time re-location when no
  event id was persisted. The API's `eventId`/`iCalUID` are stable at the source.
- **No organizer/RSVP data** caps People (0016), participants (0017), owner identity (0018),
  and everything in 0013 Wave 4.

This is also Vinyl's **first sanctioned non-LLM egress**; the decision record is
**ADR-0010** (opt-in, read-only, Calendar-scopes-only, Keychain tokens, packet-capture-verified
metadata-only boundary). This spec is the implementation.

## Goals

- Connect one Google account (OAuth PKCE loopback), read-only, tokens in the Keychain.
- Google-sourced events flow through the **existing** pipeline: Day Agenda, upcoming list,
  Join & Record, `meetings.calendar_event_id` adoption/dedupe, participant seeding — with
  richer attendees (individually materialized DL members, organizer, real names).
- One active source at a time (2026-07-02 amendment): EventKit remains the zero-config
  default; connecting Google switches the app's calendar source to Google wholesale;
  disconnect cleanly reverts to today's EventKit-only behavior.
- No regression to the 0024/0029 calendar-identity work or the 0026/0029 dismissal scheme.

## Non-goals

- **Write access** of any kind (we never modify the user's calendar — same stance as 0026).
- **Multiple Google accounts** (v1 = one; the Keychain account naming leaves room).
- **Non-Google providers** (Outlook/Graph, generic CalDAV). The merge layer is the seam a
  future provider plugs into (the old roadmap's pluggable-provider ambition), but no trait
  zoo in v1 — see Approach.
- **Directory / Cloud Identity group expansion** (deterministic full DL membership) — v2,
  requires a new scope grant and an ADR-0010 amendment.
- **Webhook push** (needs a public HTTPS endpoint; local-first apps poll).
- Contact/People sync, and any change to the 0027 "label a DL row" work (that lands on top).

## Approach

A `calendar/google/` module in the Rust core: an **OAuth PKCE loopback flow** (the `oauth2`
crate), a **poll-based incremental sync** (`events.list` + `syncToken`) into a local SQLite
event cache, and a **pure merge function** that combines Google-cached events with the live
EventKit read before the existing `build_agenda`/upcoming paths. Google-sourced events adopt
the existing wire shape (`UpcomingMeeting`, `Attendee`) and id conventions, so the entire
downstream pipeline (Join & Record stash, `find_adoptable_calendar_meeting`, participant
seeding, dismissals) works unchanged on opaque ids.

Why this shape:

- **`oauth2` crate (v4.x)** over alternatives: v4 pairs with the already-pinned
  `reqwest 0.11` (Cargo.toml), gives PKCE/state/token-exchange correctness with typed errors,
  and lets us own token persistence (Keychain). `yup-oauth2` rejected — its token cache
  writes JSON to disk, violating ADR-0010's "tokens never touch SQLite/disk". Hand-rolling
  rejected — the flow is small but the failure matrix (state mismatch, error redirects,
  refresh) is exactly where a vetted crate earns its keep.
- **Poll, not push** (ADR-0010): sync on connect, on app focus, before an agenda build when
  the cache is stale (>5 min), and on manual refresh. Incremental `syncToken` keeps each poll
  to one cheap request per selected calendar in the steady state.
- **SQLite event cache** (not per-request API calls): the agenda stays fast and
  offline-tolerant, attendees ride along in the same row (no N+1 fetches — unlike the
  EventKit path's per-event attendee reads), and packet capture stays trivially auditable.
- **Pure merge function, no provider trait in v1**: `day_agenda::build_agenda` is already a
  pure, unit-tested function over plain inputs; the merge layer follows that house pattern.
  EventKit is sync ObjC FFI, Google is async sqlx — a shared trait would be awkward and
  premature for two sources. Introduce the trait when a third provider is real.

## Design

### Identity & dedupe (the load-bearing part)

- **Google item id** (becomes `UpcomingMeeting.id` and, on adoption,
  `meetings.calendar_event_id`): `gcal:<calendarId>/<instanceEventId>`. Sync uses
  `singleEvents=true`, so recurring occurrences get per-instance ids
  (`<recurringEventId>_<originalStartTime>`) — occurrence-unique by construction. The `gcal:`
  prefix is the routing discriminator for attendee lookups; EventKit ids never carry it.
  `find_adoptable_calendar_meeting` and the Join & Record stash treat the id as an opaque
  string, so 0024 WS2.1 / 0029 WS2.1 behavior is preserved verbatim.
- **External identity**: `UpcomingMeeting.external_id` = the event's **`iCalUID`**. EventKit's
  `calendarItemExternalIdentifier` for a Google-synced calendar is the same iCalUID, which
  gives us both:
  - **Dismissal keys transfer across sources**: the existing
    `ext:<externalId>@<occurrenceStart>` key (`day_agenda.rs::stable_dismiss_key`, 0029
    WS6.2) is computed identically for a Google-sourced item — an event hidden while on
    EventKit stays hidden after connecting Google, and vice versa. Both providers must emit
    `starts_at` as chrono `to_rfc3339()` UTC so the string keys align (verification task 7).
  - **Cross-source dedupe**: merge drops the EventKit copy of any event whose
    (`external_id`, parsed start instant) pair matches a Google-cached event — **Google
    wins** (richer attendees). EventKit events with no match (iCloud/Exchange/local
    calendars) pass through untouched. No Google connection ⇒ merge is the identity function
    (today's behavior, bit-for-bit).
- **Attendee lookup routing**: `event_attendees_by_id` callers (participant seeding via
  `meeting_participant.rs::seed_from_attendees`, `diarization/commands.rs`) route on the
  `gcal:` prefix → read `attendees_json` from the cache row; otherwise the existing EventKit
  path. The legacy title+time fallback (`event_attendees`) gains a same-shaped Google-cache
  lookup so meetings without a persisted event id still resolve.

### Data model

New migration `frontend/src-tauri/migrations/<next>_add_google_calendar.sql` (forward-only,
per house rules). Tokens are **not** here — Keychain only (ADR-0010):

```sql
CREATE TABLE google_calendar_account (      -- single row (v1: one account)
  id            INTEGER PRIMARY KEY CHECK (id = 1),
  email         TEXT NOT NULL,              -- shown in Settings; from calendarList 'primary'
  connected_at  TEXT NOT NULL
);

CREATE TABLE google_calendar_sync (         -- one row per calendar on the account
  calendar_id    TEXT PRIMARY KEY,
  summary        TEXT NOT NULL,             -- calendar display name
  selected       INTEGER NOT NULL DEFAULT 1,-- user's per-calendar sync toggle
  sync_token     TEXT,                      -- NULL => full (re)sync needed
  last_synced_at TEXT
);

CREATE TABLE google_calendar_events (
  id              TEXT PRIMARY KEY,         -- 'gcal:<calendarId>/<instanceId>'
  calendar_id     TEXT NOT NULL,
  ical_uid        TEXT,                     -- cross-source external identity
  title           TEXT NOT NULL,
  starts_at       TEXT NOT NULL,            -- RFC3339 UTC (chrono to_rfc3339)
  ends_at         TEXT NOT NULL,
  is_all_day      INTEGER NOT NULL DEFAULT 0,
  location        TEXT,
  zoom_url        TEXT,                     -- via calendar::zoom_link::extract_zoom_url
  organizer_email TEXT,
  my_response     TEXT,                     -- needsAction|declined|tentative|accepted
  attendees_json  TEXT NOT NULL DEFAULT '[]', -- [{name,email,isCurrentUser,responseStatus,isOrganizer}]
  status          TEXT NOT NULL DEFAULT 'confirmed',
  updated_at      TEXT NOT NULL
);
CREATE INDEX idx_gcal_events_starts_at ON google_calendar_events(starts_at);
CREATE INDEX idx_gcal_events_ical_uid  ON google_calendar_events(ical_uid);
```

Repository: `frontend/src-tauri/src/database/repositories/google_calendar.rs` (upsert events,
delete cancelled, window query for agenda/upcoming, sync-state get/set, purge-all on
disconnect), following the `dismissed_calendar_event.rs` shape.

**Secrets** (`secrets.rs` additions): account constant `gcal.refresh_token` via the existing
`SecretStore`. Access token + expiry in memory only (module-level state in `google/oauth.rs`).
Unlike API keys there is **no DB-sentinel fallback** — a failed Keychain write aborts the
connect with a user-facing error.

**OAuth client credentials**: compile-time `option_env!("VINYL_GOOGLE_CLIENT_ID")` /
`option_env!("VINYL_GOOGLE_CLIENT_SECRET")` in `google/mod.rs` (the desktop-app "secret" is
non-confidential per ADR-0010). Unset ⇒ the Settings card renders a "not configured in this
build" state; no dead Connect button. Build scripts (`build-gpu.sh` path) pass them from the
environment; document in the spec-follow-up to `docs/`.

### Sync model

`frontend/src-tauri/src/calendar/google/sync.rs`:

- **Initial sync** (per selected calendar): `GET /calendars/{id}/events` with
  `singleEvents=true`, `timeMin=now-14d`, `timeMax=now+60d`, `maxResults=250`, paged;
  store `nextSyncToken`. (The window bounds are baked into the token per Google's sync
  contract; 14 days back covers the agenda's recording-matching lookback, 60 days forward
  covers upcoming/pick-list uses.)
- **Incremental**: same endpoint with `syncToken` only. `status=cancelled` instances delete
  the cache row. HTTP `410 GONE` ⇒ clear token, transparent full resync. When `now` ages past
  the original `timeMax` horizon minus 7 days, schedule a full resync to re-extend the window.
- **Triggers**: connect completes; app window focus; any agenda/upcoming build when
  `last_synced_at` is older than **5 minutes** (staleness constant); a fixed **10-minute
  background timer** while the app runs (owner decision 2026-07-02 — keeps the "happening
  now" card and auto-record triggers fresh without a window focus); manual "Sync now".
  Single-flight guard so triggers coalesce.
- **Mapping** (`sync.rs::map_event`): title fallback "(No title)"; skip `is_all_day` at query
  time (parity with EventKit's all-day skip); `zoom_url` extracted from
  `location + description + hangoutLink + conferenceData.entryPoints[].uri` via the existing
  `extract_zoom_url`; attendee `self=true` → `is_current_user` (parity with EventKit's
  `isCurrentUser`, feeds owner identity 0018); attendee `resource=true` (rooms) dropped;
  declined-by-me events stay in the cache but are **excluded from agenda/upcoming window
  queries** (owner decision 2026-07-02: hide declined by default; the `my_response` flag
  stays in the row so a future "show declined" toggle is cheap).

### Merge layer

`frontend/src-tauri/src/calendar/merge.rs` — pure + unit-testable:

```rust
pub fn merge_events(eventkit: Vec<UpcomingMeeting>, google: Vec<UpcomingMeeting>)
    -> Vec<UpcomingMeeting>
```

Google events pass through; EventKit events are dropped iff (`external_id` matches a Google
event's `ical_uid`) AND (start instants equal after parsing, ±60s tolerance for provider
rounding). Call sites: `day_agenda::api_get_day_agenda` (also: agenda attendee fetch for
`gcal:` items reads the cached `attendees_json` instead of the per-event EventKit read) and
`commands::api_get_upcoming_meetings`. `dismiss_key` computation in `build_agenda` needs no
change — it already prefers `external_id`.

### Tauri IPC

New commands in `frontend/src-tauri/src/calendar/google/commands.rs`, registered in
`frontend/src-tauri/src/lib.rs`:

- `api_google_calendar_status() -> { configured, connected, email, lastSyncedAt,
  calendars: [{id, summary, selected}] }` — `configured` = client id baked into the build.
- `api_google_calendar_connect() -> { email }` — runs the full flow: ephemeral-port loopback
  listener, browser open via the existing `open_external_url` allowlisted path, PKCE
  exchange, Keychain store, calendar-list fetch, initial sync. 5-minute abandonment timeout;
  cancellable; returns a user-friendly error string on every failure leg.
- `api_google_calendar_disconnect()` — best-effort token revoke, Keychain delete, purge
  `google_calendar_*` tables (keep `dismissed_calendar_events` — keys are cross-source).
- `api_google_calendar_set_calendar_selected(calendarId, selected)` — toggles + resyncs.
- `api_google_calendar_sync_now()`.
- Event `google-calendar-auth-required` (Rust→frontend) emitted when a refresh fails with
  `invalid_grant` (revoked/expired) so the UI shows the reconnect prompt.

### UI

- **Settings — a "Calendar" card** (extract into
  `frontend/src/components/CalendarSettings.tsx`, wired where `RecordingSettings.tsx` /
  `app/settings/page.tsx` + `app/_components/SettingsModal.tsx` mount today's calendar
  permission UI): macOS Calendar row (existing EventKit status/connect, unchanged) + Google
  row: Connect button → "Complete the connection in your browser" pending state → connected
  state showing account email, last sync, per-calendar checkboxes, "Sync now", Disconnect
  (confirm dialog). **Privacy copy** (ADR-0010 wording): "Read-only. Vinyl downloads event
  details (titles, times, attendees) to this Mac. Your recordings, transcripts, and notes
  are never uploaded — to Google or anyone." Plus a first-connect explainer for the
  "Google hasn't verified this app" interstitial (expected for a personal build; Advanced →
  Continue).
- **Reconnect prompt**: listen for `google-calendar-auth-required` → toast + a persistent
  Settings-card banner ("Google Calendar disconnected — reconnect"). Agenda silently
  degrades to EventKit + last-good cache; no error states in the agenda itself.
- **`frontend/src/lib/googleCalendar.ts`**: thin never-throw wrappers over the commands,
  mirroring `lib/calendar.ts` house style.
- No agenda redesign: Google items render through the existing `DayAgendaItem` pipeline.
  (Optional, cheap: a source dot/tooltip on hover — defer unless trivially clean.)

### Failure modes

| Condition | Behavior |
|---|---|
| Offline / API unreachable | Serve cache; skip sync; no toast (agenda already best-effort) |
| Access token expired | Refresh once, retry request |
| Refresh fails `invalid_grant` (revoked) | Mark disconnected, emit `google-calendar-auth-required`, keep cache read-only until reconnect/disconnect |
| `410 GONE` on syncToken | Clear token, full resync, silent |
| `403/429` quota | Exponential backoff; retry on next trigger |
| Loopback port unavailable | Retry with new ephemeral port (bind port 0) |
| User abandons consent | Timeout after 5 min; command returns "cancelled"; no state written |
| Keychain write fails on connect | Abort connect with error; nothing persisted (no DB fallback for tokens) |

## Tasks

1. [x] (rust-core-engineer) **OAuth flow + secrets.** Add `oauth2 = "4"` to
   `frontend/src-tauri/Cargo.toml`; `calendar/google/{mod.rs,oauth.rs}`: PKCE S256 +
   state, loopback listener (bind `127.0.0.1:0`, single request, minimal "you can close
   this window" response), token exchange/refresh, revoke; `gcal.refresh_token` account in
   `secrets.rs`; in-memory access-token cache. Unit tests with `MemoryStore` + a mocked
   token endpoint (no network in CI).
2. [x] (rust-core-engineer) **Migration + repository.** The three tables above;
   `database/repositories/google_calendar.rs`; registered in `repositories/mod.rs`; covered
   by the `db_lifecycle` migration test.
3. [x] (rust-core-engineer) **Sync engine.** `calendar/google/sync.rs`: calendar-list fetch,
   initial + incremental sync, 410 handling, cancellation of instances, `map_event`
   (all-day skip, zoom-link extraction, attendee mapping incl. `self`/`resource`), horizon
   re-extension, single-flight trigger coalescing. Unit tests over canned `events.list`
   JSON fixtures (incl. a recurring instance, a cancelled instance, a group attendee with
   RSVP-materialized members, a non-ASCII title).
4. [x] (rust-core-engineer) **Merge + routing.** `calendar/merge.rs` (pure, unit-tested:
   dedupe by iCalUID+start, EventKit-only passthrough, Google precedence, identity when
   disconnected); wire into `day_agenda::api_get_day_agenda` and
   `commands::api_get_upcoming_meetings`; `gcal:` routing in `event_attendees_by_id` /
   `event_attendees` consumers (`diarization/commands.rs`,
   `database/repositories/meeting_participant.rs` seeding path). Test: dismissal key for a
   Google-sourced occurrence equals the EventKit-era `ext:<uid>@<start>` key.
5. [x] (rust-core-engineer) **Commands + events.** `calendar/google/commands.rs`, the five
   commands + `google-calendar-auth-required` event, registration in `lib.rs`; disconnect
   purge semantics (tables + Keychain + revoke; dismissals retained).
6. [x] (frontend-engineer) **Settings UI + lib.** `lib/googleCalendar.ts`;
   `components/CalendarSettings.tsx` (connect/pending/connected/disconnect, calendar
   toggles, privacy + unverified-app copy, reconnect banner); mount in `app/settings/page.tsx`
   / `SettingsModal.tsx`; `google-calendar-auth-required` listener + toast. `pnpm test`
   coverage for the status-driven render states.
7. [ ] (rust-core-engineer, verification) **Key-equivalence check.** On a real Google-synced
   macOS calendar, log EventKit `calendarItemExternalIdentifier` vs API `iCalUID` for the
   same events (the existing `day_agenda` debug log + a temporary sync-side log). If they
   diverge (e.g. suffix normalization), add a normalization shim in `merge.rs` **before**
   shipping — dedupe and dismissal transfer both depend on it.
8. [ ] (owner + rust-core-engineer) **Egress verification.** Packet capture per ADR-0010:
   disconnected ⇒ zero Google traffic; connected ⇒ only `accounts.google.com` /
   `oauth2.googleapis.com` / `www.googleapis.com` GETs + token POSTs; no request body ever
   contains transcript/notes/summary content. Grep logs + SQLite for token material (must
   be absent).

## Acceptance criteria

Ties to the Definition of Done in `/CLAUDE.md` (cargo check/clippy/test, pnpm lint/test,
`clean_run.sh`, record→transcript→summary smoke), plus:

1. **Connect**: from Settings, the browser flow completes; the card shows the account email;
   events from selected Google calendars appear in the Day Agenda within one sync, with
   individual attendees (names + emails) on invited events.
2. **DL fidelity (the 0027 payoff)**: a Google Workspace event invited via a group shows the
   individually materialized members the API returns (RSVP'd/expanded), not only the single
   group address — verified on a real Workspace meeting.
3. **Identity non-regression**: "Join & Record" on a Google-sourced item produces exactly one
   meeting titled from the event, `calendar_event_id = gcal:...`, roster seeded — the 0024
   WS2.1 / 0029 WS2.1 invariants hold on both source types (extend the existing lifecycle
   regression tests with a `gcal:`-id fixture).
4. **Single source** (amended 2026-07-02): while Google is connected the agenda shows
   only Google-cache events (each event once — no EventKit read at all); disconnecting
   restores the EventKit view. Unit-tested at the source-selection helper + manual check.
5. **Dismissals transfer**: an event hidden pre-connect (EventKit `ext:` key) stays hidden
   post-connect, and vice versa (unit test + manual).
6. **Disconnect**: tokens gone from Keychain (Keychain Access shows none under the bundle-id
   service), `google_calendar_*` tables empty, agenda reverts to EventKit, packet capture
   shows zero Google egress thereafter.
7. **Token/offline robustness**: revoking access at myaccount.google.com produces the
   reconnect prompt (no error loop); offline launch renders the cached agenda.
8. **Privacy**: task 8's packet-capture + token-material checks pass; masked-or-absent is the
   only way token state ever reaches logs/UI.
9. **No-Google builds**: without `VINYL_GOOGLE_CLIENT_ID`, all suites pass and Settings shows
   the not-configured state — the feature is inert, not broken.

## Risks / open questions

- **EventKit external-id ≠ iCalUID in some sync configurations** — would break dedupe and
  dismissal transfer. Mitigated by task 7 (verify before ship, shim if needed); worst case,
  dedupe falls back to (normalized title + start instant) matching.
- **Google's group expansion is partial by design** (ADR-0010): members appear as they RSVP /
  as Google's async listing lands. A never-RSVP'd DL may still show as one address. If real
  meetings still show opaque DLs, the v2 answer is the Directory/Cloud Identity scope —
  an ADR-0010 amendment, not a workaround here. 0027's detect-and-label slice remains worth
  landing on top.
- **`oauth2` v4 vs the reqwest 0.11 pin**: if a future reqwest 0.12 bump lands, move to
  `oauth2` v5 then; don't bump reqwest for this feature.
- **Unverified-app UX**: the interstitial + 100-user cap are fine for personal use but a
  hard stop for distribution (ADR-0010 consequence). Testing-status is NOT acceptable
  (7-day refresh-token expiry ⇒ weekly re-consent).
- **Cache growth**: bounded by the ±(14d/60d) window; the full-resync horizon re-extension
  naturally prunes (delete rows outside the new window on full resync).

### Open questions (owner) — ANSWERED 2026-07-02

1. **ADR-0010 posture confirmation**: ✅ **Accepted** — ship unverified ("In production",
   click through the interstitial, ≤100 users) against the owner's personal GCP project;
   Google verification only if/when Vinyl is publicly distributed. ADR-0010 → Accepted.
2. **Poll cadence / battery**: ✅ **Add the 10-minute timer** — background sync every 10 min
   while the app runs, on top of focus/staleness/manual triggers (folded into Sync model).
3. **Client-id delivery**: ✅ **Build-time env** (`VINYL_GOOGLE_CLIENT_ID`/`_SECRET`),
   per the spec default — no runtime paste UI.
4. **Precedence check**: ✅ **Google wins wholesale** (default taken — no EventKit field
   preferred).
5. **Declined events**: ✅ **Hide declined by default** — excluded from agenda/upcoming
   queries; the cached `my_response` flag makes a future toggle cheap (folded into Sync
   model → Mapping).

## Verification

- Rust: `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal &&
  cargo clippy && cargo test --features metal` (new: oauth unit tests with `MemoryStore`,
  sync fixture tests, `merge.rs` dedupe/dismiss-key tests, `db_lifecycle` migration pass).
- Frontend: `cd frontend && pnpm lint && pnpm test`.
- Manual smoke (bundled `Vinyl.app`, per ADR-0004): connect → agenda shows Google events with
  attendees → Join & Record a Google-sourced event → one correctly titled meeting with
  seeded roster → record→transcript→summary path intact → hide an event, reconnect-cycle,
  dismissal persists → disconnect → EventKit-only agenda + zero Google egress (packet capture).
