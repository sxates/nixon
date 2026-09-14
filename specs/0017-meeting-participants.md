# 0017 — Persistent meeting participants & participant-bounded diarization

- **Status:** Done (2026-06-27) — all phases built + verified (cargo check/clippy clean, 80 diarization/participant tests, pnpm lint clean). Needs a bundled-app runtime smoke.
- **Owner agent(s):** rust-core-engineer (lead: `meeting_participants` table/repo, seeding +
  auto-People, IPC, count resolution) + audio-engineer (`SpeakerCount::AtMost` post-cluster
  centroid-merge in `sherpa.rs`/`pipeline.rs`) + frontend-engineer (participants panel on detail
  + recording, person modal from a meeting screen)
- **Roadmap phase:** Phase 3 — refines the shipped identity work
- **Refines:** `specs/0015-meetings-as-first-class-objects.md` (Join & Record adopts
  `calendar_event_id`; attendees read live, never persisted), `specs/0016-cross-meeting-identity-wave1.md`
  Wave 1 (`people`/`speakers.person_id`/`PeopleRepository`), and `specs/0012` roles
  (`people.role` already lands; this spec consumes it read-only).
- **No ADR.** Adds no biometric storage and no new outbound traffic. `meeting_participants` is a
  pure identity/association join — voiceprints (ADR-0007) are untouched.

## Context / Problem

After `specs/0015`+`specs/0016`, Vinyl has durable People and links a calendar event to a meeting,
but the **invited participant list itself is ephemeral**: it is read live from EventKit on every
call and never persisted, surfaced only on Home (`api_get_day_agenda`). Five concrete gaps:

1. **Participants are invisible on the meeting detail page** — both live and after. The detail view
   (`meeting-details/page-content.tsx`) shows speakers (who spoke) but not the invited roster.
2. **You can't curate the roster.** No add/remove of a participant during or after a meeting.
3. **Invitees aren't promoted to People.** A new invitee with an email never becomes a `people`
   row until someone manually maps a *speaker* to them in the legend.
4. **Calendar attendee count is treated as an assumption, not a cap.** `resolve_speaker_count`
   turns the remote attendee count into `SpeakerCount::Fixed(remote)` — forcing *exactly* that many
   clusters. A 10-invited / 4-spoke meeting is forced to 10 clusters, splitting real speakers. The
   count should be an **upper bound**, not a target.
5. **No way to edit a Person from a meeting screen.** The reusable `PersonFormDialog` lives only on
   the `/people` route; you must leave the meeting to fix a name/role.

### Grounding (verified in this repo, 2026-06-27 — state of the art, do **not** re-derive)

- **Attendees are read LIVE, never persisted.** `api_get_meeting_attendees`
  (`diarization/commands.rs:409`) resolves via `eventkit::event_attendees_by_id(event_id)` when
  `meetings.calendar_event_id` is set (specs/0015), else `eventkit::event_attendees(title,
  started_at)`. `Attendee` (`calendar/eventkit.rs:50`) = `{ name: String, email: Option<String>,
  is_current_user: bool }`. Home uses `api_get_day_agenda` (`calendar/day_agenda.rs`) which embeds
  ≤ `MAX_INLINE_ATTENDEES = 5` attendees + `attendee_count`.
- **People exist (specs/0016 1b).** `people(id, email UNIQUE-when-not-null, display_name, role,
  notes, voiceprint_opt_out, created_at, updated_at)`; partial unique index
  `idx_people_email ... WHERE email IS NOT NULL`. `PeopleRepository`
  (`database/repositories/people.rs`): `create` (**upsert-by-email** — returns the existing person on
  email collision, normalizes empty email → NULL, generates `person-<uuid>`), `get`, `get_by_email`,
  `list`, `update`, `set_voiceprint_opt_out`, `delete` (explicit cascade), `assign_speaker_to_person`,
  `get_meeting_speaker_roles`. People IPC is `api_list_people`/`api_create_person`/`api_update_person`/
  `api_delete_person`/`api_assign_speaker_to_person` (`people/commands.rs`, registered `lib.rs:887`).
- **`speakers.person_id`** (nullable) links a per-meeting diarized speaker to a person
  (`migrations/20260628000001_add_speaker_person_id.sql`). `speakers` is **who spoke**; it is NOT
  the roster.
- **Diarization speaker-count is the key change site:**
  - `diarization/sherpa.rs`: `enum SpeakerCount { Auto, Fixed(u32) }`. Auto = agglomerative
    clustering at the tuned `DEFAULT_AUTO_THRESHOLD = 0.8`; `Fixed(n)` forces sherpa's
    `num_clusters = n` (threshold ignored). **sherpa exposes no native "at most n".**
  - `diarization/settings.rs::resolve_speaker_count(settings, calendar_attendees) -> (SpeakerCount,
    SpeakerCountSource)`: precedence = manual `expected_speaker_count` → `Fixed(n)`/`Manual`; else
    remote (exclude-self) calendar count → **`Fixed(remote)`/`Calendar`**; else `Auto`. The
    `Calendar → Fixed` branch is the "assumes everyone speaks" bug (#4). `SpeakerCountSource` is
    `Manual|Calendar|Auto`, serialized lowercase on `diarization-complete`.
  - `diarization/pipeline.rs::resolve_meeting_speaker_count` (async) does a live
    `lookup_calendar_attendees` (still `eventkit::event_attendees(title, started_at)` at `pipeline.rs:702`,
    NOT the by-id path) at diarize time, emits `speakerCountSource` + `seededSpeakerCount` on
    `diarization-complete`, where `seeded = Fixed(n) → Some(n)`, `Auto → None`.
  - `diarize_with_embeddings` (`mod.rs:98`, overridden in `sherpa.rs:437`) already returns
    `TurnsWithEmbeddings = (Vec<SpeakerTurn>, HashMap<spk_N, Vec<f32>>)`; the offline pipeline calls
    it (`pipeline.rs:366`). `embedding.rs` has `cosine_similarity`, `l2_normalize`, `weighted_mean`,
    `embedding_to_bytes`/`embedding_from_bytes`, `EMBEDDING_MODEL_ID`.
- **People UI:** `/people` route (`app/people/page.tsx`) with **reusable** `PersonFormDialog`
  (`app/people/PersonFormDialog.tsx`, props `{ open, onOpenChange, person?, onSaved }`) +
  `ForgetPersonDialog.tsx`. `SpeakerLegend.tsx` + `hooks/useSpeakers.ts` already offer calendar
  attendees AND People in the assign pop-list + "Looks like" chips. `MeetingIdentityHeader.tsx`
  is the detail-page header. Recording chrome: `components/GlobalRecordingBar.tsx`.
- **No `PRAGMA foreign_keys`.** The app pool connects without it, so every cascade is explicit in
  the delete transaction (see `repositories/people.rs::delete`, `repositories/meeting.rs`).

## Goals

- **Persist an editable participant roster per meeting** in a new `meeting_participants` join table,
  distinct from `speakers`. Seeded from the calendar event, hand-curatable.
- **Show participants on the meeting detail page — live while recording AND after** — and reachable
  during recording (a panel off the record screen / `GlobalRecordingBar`).
- **Auto-generate a People row for each new invitee** (with their email) when a meeting is linked to
  a calendar event; insert a `meeting_participants` row (source `'calendar'`). Idempotent.
- **Manual add/remove of a participant at any time**, during and after, without touching the Person
  or any speaker assignment.
- **Make the participant/attendee count a MAXIMUM, not an assumption.** Calendar-derived counts
  become `SpeakerCount::AtMost(n)` (Auto clustering then post-cluster centroid-merge down to ≤ n);
  the **manual** `expected_speaker_count` stays `Fixed(n)` (explicit exact statement).
- **Edit a Person from a meeting screen** via the existing `PersonFormDialog` as a modal.
- On-device; no new outbound traffic; no new dependency; no new model.

## Non-goals

- **No voiceprint / ADR-0007 changes.** `meeting_participants` stores no embeddings. Enroll-on-confirm
  and the gallery are unchanged.
- **No persisted attendee *snapshot* columns.** We persist People + a join, not a frozen copy of
  EventKit fields. The live calendar read remains the source for *display* enrichment on Home.
- **No participant→speaker auto-binding.** A participant is "invited/known"; a speaker is "spoke."
  We cross-reference for display but do not force a mapping.
- **No role editing model change.** `people.role` (specs/0012) is shown read-through; role-weighted
  summary stays a `specs/0012` follow-on.
- **No recurring-series participant memory, no contact-book sync, no merge-people UX, no bulk edit.**
- **No change to manual-override semantics.** A user who types an exact expected count still gets
  `Fixed` — they asserted an exact number.

## Approach

**Add one join table (`meeting_participants`) as the durable, editable roster; seed it from the
calendar event by upserting People; change only the *calendar-derived* count to an upper bound via a
new `SpeakerCount::AtMost` implemented as post-cluster centroid merge; and reuse every existing
component (`PeopleRepository`, `PersonFormDialog`, the assign pop-list).**

- **Roster = `meeting_participants(meeting_id, person_id, source, created_at)`** — the participant
  list is People, so a participant *is* already an app-wide identity (satisfies auto-People and gives
  us role/notes for free). `source` distinguishes `'calendar'` (seeded) from `'manual'` (added).
- **Seeding** runs when a meeting is linked to a calendar event: for each remote invitee, upsert a
  `people` row (email → `PeopleRepository::create` upsert-by-email; email-less → create-by-name) and
  insert a `meeting_participants` row. Idempotent: `INSERT OR IGNORE` on the `(meeting_id, person_id)`
  PK. The owner (`is_current_user`) is **excluded** from the roster (the roster is the *other*
  people; "You" is implicit and is the mic channel, consistent with diarization's exclude-self).
- **Count-as-max** = the *calendar/participant*-derived branch of `resolve_speaker_count` returns
  `SpeakerCount::AtMost(remote_count)`; manual stays `Fixed`. `AtMost(n)` runs Auto clustering then
  **iteratively merges the two closest cluster centroids (cosine) until cluster count ≤ n**, reusing
  the per-cluster embeddings already produced by `diarize_with_embeddings`. The count is sourced from
  the **persistent roster** (remote, exclude owner) first, falling back to the live calendar lookup
  only when no participants are persisted, so manual roster edits move the cap.
- **Person modal from a meeting** = render the already-reusable `PersonFormDialog` from the
  participants panel (and speaker legend) — a wiring task, not a new component.

Why this over alternatives: a parallel `meeting_attendees` snapshot table was already rejected by
`specs/0015` — People + a join is the durable home and gives auto-People for free. A native sherpa
"at most" doesn't exist; teaching `Fixed` to clamp down would discard the embeddings we already have,
whereas centroid-merge is deterministic, reuses `cosine_similarity`/`weighted_mean`, and degrades to
Auto when `current ≤ n` (so the no-participant path is byte-identical). Seeding via the existing
`PeopleRepository::create` upsert-by-email reuses the collision policy already shipped in 0016.

## Design

### Data model (forward-only migration under `frontend/src-tauri/migrations/`)

Next prefix after `20260628000002_add_voiceprints.sql` → **`20260629000000_add_meeting_participants.sql`**.
Additive, idempotent; no `PRAGMA foreign_keys` (cascades are explicit in Rust).

```sql
-- specs/0017: the durable, editable participant roster for a meeting — distinct from
-- `speakers` (who actually spoke). Each participant IS an app-wide person (people.id),
-- so auto-People and role/notes come for free. `source` = how the row got here.
-- No PRAGMA foreign_keys on the pool → meeting-delete and person-delete cascades are
-- enforced in the existing delete transactions (see meeting.rs / people.rs).
CREATE TABLE IF NOT EXISTS meeting_participants (
    meeting_id  TEXT NOT NULL,                 -- → meetings.id
    person_id   TEXT NOT NULL,                 -- → people.id
    source      TEXT NOT NULL DEFAULT 'manual',-- 'calendar' (seeded) | 'manual' (added)
    created_at  TEXT NOT NULL,
    PRIMARY KEY (meeting_id, person_id)         -- idempotent seeding: INSERT OR IGNORE
);
CREATE INDEX IF NOT EXISTS idx_meeting_participants_person ON meeting_participants(person_id);
```

Cascade wiring (explicit, mirroring existing transactions):
- `repositories/meeting.rs::delete_meeting_with_transaction` — add
  `DELETE FROM meeting_participants WHERE meeting_id = ?` alongside the existing `speakers` delete.
- `repositories/people.rs::delete` — add `DELETE FROM meeting_participants WHERE person_id = ?`
  inside the same tx that NULLs `speakers.person_id` (so forgetting a person also un-rosters them).

### Repository — `database/repositories/meeting_participant.rs` (new)

`MeetingParticipantsRepository` (mirrors `speaker.rs`/`people.rs`; returns `SqlxError`):

- `list(pool, meeting_id) -> Vec<MeetingParticipant>` — JOIN `meeting_participants` → `people`,
  returning `{ person_id, display_name, email, role, source }` ordered by `display_name COLLATE
  NOCASE`. This is the roster the UI renders (camelCase serde).
- `add(pool, meeting_id, person_id, source) -> Result<bool>` — `INSERT OR IGNORE`; returns whether a
  row was inserted (idempotent).
- `remove(pool, meeting_id, person_id) -> Result<bool>` — `DELETE` the join row **only**; never
  touches `people` or `speakers`.
- `remote_person_ids(pool, meeting_id) -> Vec<String>` — person_ids on the roster (the roster is
  already remote-only — owner excluded at seed time). Used to size the diarization cap.
- `seed_from_attendees(pool, meeting_id, attendees: &[Attendee]) -> Result<usize>` — for each
  attendee with `!is_current_user`: upsert a person (`PeopleRepository::create(display_name, email)`)
  then `add(.., 'calendar')`. Email-less attendees are **created by name** (0016 already supports
  email-less people, and an un-rostered invitee is worse than a name-only one). Returns count added.
  Idempotent across re-resolution via the PK + upsert-by-email.

### Seeding call sites (rust-core-engineer)

Seeding must fire whenever attendees are resolved for a calendar-linked meeting, and be idempotent:

1. **At Join & Record creation** (the meeting is created with `calendar_event_id`, specs/0015): after
   creation, resolve attendees by id and `seed_from_attendees`. (The frontend `joinAndRecord` path
   already has the event's attendees inline from `DayAgendaItem`; pass them through a new
   `api_seed_meeting_participants(meetingId)` call, or seed server-side in the create path — see IPC.)
2. **Lazily in `api_get_meeting_participants`** (below): if the roster is empty AND the meeting has a
   `calendar_event_id`, resolve attendees and seed before returning — so legacy/linked meetings
   self-heal on first view. Idempotent, so the two paths can't duplicate.

### Tauri IPC (register in `frontend/src-tauri/src/lib.rs`; new `participants/commands.rs` or fold into `diarization/commands.rs`)

- `api_get_meeting_participants(meeting_id) -> Vec<MeetingParticipant>` — returns the roster; if empty
  and `calendar_event_id` is set, seed-then-return (the self-heal path). The detail + recording UIs
  read this.
- `api_add_meeting_participant(meeting_id, person_id?, display_name?, email?) -> MeetingParticipant`
  — if `person_id` given, link the existing person; else create-or-upsert a person
  (`PeopleRepository::create`) from `display_name`/`email`, then `add(.., 'manual')`. Returns the
  rostered participant. Works during and after recording (it's a plain DB write).
- `api_remove_meeting_participant(meeting_id, person_id) -> bool` — removes the join row only.
- No new event. The detail/recording panels re-fetch after add/remove; the existing
  `diarization-complete` already drives speaker refresh.

### Diarization count-as-max (audio-engineer for `AtMost` impl; rust-core for resolution)

**`SpeakerCount` (`diarization/sherpa.rs`)** — add a variant:
```rust
pub enum SpeakerCount {
    Auto,
    Fixed(u32),   // manual override: force EXACTLY n (unchanged)
    AtMost(u32),  // specs/0017: upper bound — Auto, then merge down to ≤ n
}
```
`with_config` maps `AtMost` to `AUTO_NUM_CLUSTERS` (`-1`) so sherpa clusters freely; the cap is
applied **after** clustering. Store the mode (already kept in `self.speaker_count`) so the
over-cluster guardrail still only warns in `Auto`.

**Post-cluster centroid merge (`sherpa.rs`, inside `diarize_with_embeddings`)** — after sherpa
returns turns + the per-cluster `spk_N → Vec<f32>` embedding map, if mode is `AtMost(n)` and the
distinct remote cluster count `> n`:
1. Build a working set of `(key, centroid)` from the embedding map (centroids already L2-normalized).
   Clusters with no embedding (too short to embed) can't be merged by cosine — keep them as-is and
   count them toward the budget; if they alone exceed `n`, stop (we never drop a real speaker).
2. Repeat until cluster count ≤ n: compute pairwise `cosine_similarity` over current centroids, find
   the **most-similar pair**, merge them — combine their turns under one key and set the merged
   centroid via `weighted_mean(a, a_turns_dur, b, b_turns_dur)` then `l2_normalize`. Use pooled
   speech duration as the weight (turns carry start/end).
3. Re-key merged turns to a stable surviving key and rebuild the embedding map so `persist` writes
   one row per merged speaker.

This reuses `embedding.rs` math entirely; O(k²·iterations) with k = raw cluster count (tens at most)
— negligible vs. ONNX inference. Add a module const for an optional **merge floor** (don't merge a
pair whose similarity is below a guard, e.g. only merge while best-similarity ≥ a low bar) so the cap
can't force-merge two genuinely distinct voices when the real count already exceeds `n` and the audio
disagrees — in that case we cap at the natural count and log. Spec the algorithm as a pure helper
(`fn merge_clusters_to_at_most(turns, embeddings, n) -> (turns, embeddings)`) so it unit-tests
without ONNX.

**Resolution (`diarization/settings.rs::resolve_speaker_count` + a roster-aware wrapper)**:
- Manual `expected_speaker_count = Some(n>=1)` → `Fixed(n)` / `Manual` (**unchanged**).
- Else, remote count `>= 1` → **`AtMost(remote)` / `Calendar`** (changed from `Fixed`). Update the
  doc-comment and the existing unit tests (`calendar_seeds_remote_count_excluding_self`,
  `zero_manual_falls_through_to_calendar`) to assert `AtMost`.
- Else `Auto` / `Auto` (**unchanged**).
- **Source the count from the persistent roster first** (`pipeline.rs::resolve_meeting_speaker_count`):
  load `MeetingParticipantsRepository::remote_person_ids(meeting_id)`; if non-empty, build the count
  from its length (these are already remote-only) and pass `AtMost(len)`/`Calendar`; only if the
  roster is empty fall back to the existing live `lookup_calendar_attendees` path. So manual roster
  edits change the cap.

**Pipeline emit (`pipeline.rs`)** — extend the `diarization-complete` payload:
- `SpeakerCountSource` gains nothing new on the wire by default, but introduce a distinct
  mode string for the cap. Recommended: keep `speakerCountSource = "calendar"` and add a sibling
  field `speakerCountMode: "fixed" | "at_most" | "auto"` (derived from the `SpeakerCount` variant),
  so the UI can say "Estimated up to N speakers from participants" vs "Forced N". `seededSpeakerCount`
  becomes the **cap** for `AtMost(n)` (`Some(n)`), the forced count for `Fixed(n)`, `None` for `Auto`.
  Update the `seeded` match in `run()` to include the `AtMost` arm.

### UI (`frontend/src/`)

- **Participants panel — detail page** (`app/meeting-details/page-content.tsx` /
  `MeetingIdentityHeader.tsx`): a Participants section reading `api_get_meeting_participants`. Render
  each as a chip/row (name, role if set, a `'calendar'`/`'manual'` affordance is optional) with an
  add control ("Add participant" → pick an existing Person via `api_list_people` OR type name/email →
  `api_add_meeting_participant`) and a remove (×) per row → `api_remove_meeting_participant`. Visible
  for `recorded` and `notes_only` meetings. Cross-reference speakers lightly: if a participant's
  `person_id` matches a `speakers.person_id` for this meeting, badge "spoke" — read-only, no coupling.
- **Participants while recording**: surface the same panel on the record screen / a popover reachable
  from `components/GlobalRecordingBar.tsx`, reading the same `api_get_meeting_participants` for the
  `currentMeeting`. Add/remove works live (plain DB writes). For a Join & Record meeting the roster is
  already seeded; for an ad-hoc recording it starts empty and is hand-added.
- **Person modal from a meeting** (requirement F): render the existing `PersonFormDialog`
  (`app/people/PersonFormDialog.tsx`) from the participants panel (edit a rostered person) and from
  `SpeakerLegend` (edit the person behind a speaker). It already takes `{ open, onOpenChange, person,
  onSaved }`; `onSaved` re-fetches the roster. Lift it out of the `/people` route directory into a
  shared location (e.g. `components/People/PersonFormDialog.tsx`) and update the `/people` import — no
  behavior change. `/people` stays.
- **No new copy on the count source needed for v1**; the existing "Estimated N speakers from
  calendar" string can become "Estimated up to N" when `speakerCountMode === 'at_most'`.

## Tasks (ordered; phased; owner agents in bold)

### Phase A — Persistent roster + auto-People (rust-core)
1. [ ] **rust-core-engineer** — migration `20260629000000_add_meeting_participants.sql`;
   `MeetingParticipantsRepository` (`repositories/meeting_participant.rs`: `list`, `add`, `remove`,
   `remote_person_ids`, `seed_from_attendees`) + `MeetingParticipant` model; register in the repo
   module. Wire cascades into `meeting.rs::delete_meeting_with_transaction` and `people.rs::delete`.
2. [ ] **rust-core-engineer** — IPC `api_get_meeting_participants` (seed-then-return when empty +
   `calendar_event_id` set), `api_add_meeting_participant`, `api_remove_meeting_participant`; register
   in `lib.rs`. Seed at Join & Record creation (server-side in the create/link path, or via the new
   command from `joinAndRecord`).

### Phase B — Count-as-MAX (audio-engineer + rust-core)
3. [ ] **audio-engineer** — add `SpeakerCount::AtMost(u32)`; map to auto `num_clusters` in
   `with_config`; implement pure `merge_clusters_to_at_most(turns, embeddings, n)` (pairwise cosine,
   merge-closest via `weighted_mean`+`l2_normalize`, duration-weighted, merge-floor guard, never drop
   a real speaker) and call it from `diarize_with_embeddings` when mode is `AtMost`. Unit-test the
   helper (incl. "10 clusters / cap 4 → ≤ 4", "current ≤ n → unchanged", merge-floor stops early).
4. [ ] **rust-core-engineer** — `resolve_speaker_count`: calendar branch returns `AtMost(remote)`
   (manual stays `Fixed`); update its doc + the existing settings unit tests. In
   `pipeline.rs::resolve_meeting_speaker_count`, prefer `MeetingParticipantsRepository::remote_person_ids`
   over the live calendar lookup. Extend the `diarization-complete` emit with `speakerCountMode`
   (`fixed|at_most|auto`) and the `AtMost` arm of `seeded`; log the cap.

### Phase C — UI (frontend)
5. [ ] **frontend-engineer** — Participants panel on the detail page (`page-content.tsx` /
   `MeetingIdentityHeader.tsx`): list + add (existing Person or new name/email) + remove; "spoke"
   badge cross-reference; visible for recorded + notes_only.
6. [ ] **frontend-engineer** — Participants panel reachable while recording (record screen /
   `GlobalRecordingBar` popover) reading the same command for `currentMeeting`; add/remove live.
7. [ ] **frontend-engineer** — move `PersonFormDialog` to `components/People/` and render it as a
   modal from the participants panel + `SpeakerLegend` (edit person without leaving the meeting);
   `onSaved` re-fetches; update the `/people` import. Update the count-source copy to "up to N" when
   `speakerCountMode === 'at_most'`.

## Acceptance criteria (tie to the Definition of Done in `/CLAUDE.md`)

- **Participants visible live + after:** opening a calendar-linked meeting (recording or saved) shows
  the seeded roster on the detail page; the same roster is reachable while recording.
- **Auto-People:** linking a meeting to a calendar event with N remote invitees creates/upserts N
  `people` rows (email when present, name-only otherwise) and N `meeting_participants` rows with
  `source='calendar'`; re-resolving the meeting adds **no duplicates** (PK + upsert-by-email
  idempotent; `sqlite3` row-count stable across two seeds).
- **Manual add/remove:** adding a participant (existing Person or new name/email) inserts one
  `meeting_participants` row; removing one deletes **only** the join row — the `people` row and any
  `speakers.person_id` link survive (`sqlite3` confirms the person and speaker rows are untouched).
  Both work during recording and after.
- **Count as MAX (the headline):** a meeting with **10 invited / 4 who actually speak** diarizes to
  **≤ 4-ish** clusters (the natural Auto count, never force-merged below it), and is **never forced to
  10**. `diarization-complete` reports `speakerCountMode='at_most'`, `seededSpeakerCount=10`.
- **Manual exact still exact:** with `expected_speaker_count = Some(n)` set in Settings, the mode is
  `Fixed(n)` / `speakerCountMode='fixed'` (unchanged behavior).
- **No-participant / un-diarized no-regression:** a meeting with no roster and no calendar attendees
  diarizes **byte-identically** to today (Auto path; `merge_clusters_to_at_most` is never reached);
  a meeting that isn't diarized is unaffected; the summary path is byte-identical.
- **Cascades:** deleting a meeting removes its `meeting_participants` rows; "forget this person"
  removes their `meeting_participants` rows AND NULLs `speakers.person_id` (existing 0016 behavior
  preserved) — verified by `sqlite3` counts.
- **Person modal:** editing a person from the participants panel / speaker legend updates the person
  (`api_update_person`) without navigating away; the roster reflects the new name on `onSaved`.
- **Privacy:** no new outbound traffic; `meeting_participants` stores no embeddings; the only egress
  remains the sanctioned LLM summary call (display names only).
- **Gate (`/check`):** `cargo check` + `cargo clippy` clean (`frontend/src-tauri`); `pnpm lint` clean
  (`frontend`); app launches via `./clean_run.sh`; record → live transcript → summary smoke unchanged.

## Risks / open questions

- **Centroid-merge tuning (Phase B) — primary risk.** Merging the closest pair until ≤ n can merge
  two genuinely distinct quiet voices when the cap is low. Mitigations: cap is an *upper bound* only
  (Auto already decided the natural count; merge only runs when natural > cap), a **merge-floor**
  guard refuses to merge below a similarity bar (capping at the natural count + a log instead of
  fabricating a false merge), and the user retains the in-meeting merge/rename UX. Tune the floor on
  the existing `tests/diarization_tuning.rs` real file; ship conservative.
- **Email-less people (Phase A).** Name-only invitees create name-only People (0016 supports this).
  Risk: two different "John" invitees across meetings collide on name in the directory but NOT in the
  DB (separate `person-<uuid>` rows, no email unique). Acceptable — merge-people is a later wave. We
  create-by-name rather than skip so they show on the roster.
- **Participant vs speaker confusion (Phase C).** Two lists (invited vs spoke) on one screen can
  confuse. Keep them visually distinct, label clearly, and keep the only cross-reference a read-only
  "spoke" badge — do not auto-bind.
- **Idempotent seeding races.** The Join & Record create-time seed and the lazy seed-on-view could
  both fire. The `(meeting_id, person_id)` PK + `INSERT OR IGNORE` + upsert-by-email make double-seed
  a no-op; assert it in a test (seed twice, count unchanged).
- **Roster-as-cap vs live calendar.** Once a roster exists it overrides the live calendar count for
  sizing — intended (manual edits should move the cap), but means removing a participant lowers the
  cap on the next diarize. Document; it's the desired behavior.
- **Merge perf.** k² pairwise over raw clusters (tens) per iteration is trivial vs. ONNX; no concern,
  but bound iterations defensively.
- **Owner exclusion.** The roster excludes `is_current_user`; if EventKit ever fails to flag the
  owner, a self-row could appear. Mitigate by also excluding any attendee email matching the owner
  identity if available; otherwise accept (user can remove).
- **Bare dev binary has no calendar access** (specs/0015 caveat): seeding is only fully testable in
  the bundled `Vinyl.app`/`Dev Vinyl`. Manual add/remove and the cap (with a manual roster) are
  testable anywhere.

## Verification

- **Roster + auto-People:** in the bundled app, Join & Record a calendar event with several
  invitees → confirm `meeting_participants` rows (`source='calendar'`) and matching `people` rows
  (`sqlite3`); re-open the meeting → row counts unchanged (idempotent). Confirm the roster shows on
  the detail page and while recording.
- **Manual add/remove:** add a participant by name and by existing Person; remove one → `sqlite3`
  shows the join row gone, the `people` row and `speakers` rows intact; repeat during a live
  recording.
- **Count as MAX:** with the diarization-tuning fixture (or a crafted 4-voice `system.wav`) on a
  meeting whose roster has 10 remote people, run diarization → `diarization-complete` shows
  `speakerCountMode='at_most'`, `seededSpeakerCount=10`, and the persisted distinct speaker count is
  ≤ ~4 (the natural count), never 10. Set `expected_speaker_count=4` in Settings → mode `fixed`,
  exactly 4. Clear the roster + calendar → mode `auto`, behavior identical to today.
- **Cascades:** delete the meeting → `meeting_participants` for it = 0; forget one of its people →
  their `meeting_participants` rows = 0 and `speakers.person_id` NULLed.
- **Unit tests:** `merge_clusters_to_at_most` (cap reduces, no-op when ≤ n, floor stops early, never
  drops a key); updated `resolve_speaker_count` tests assert `AtMost` for the calendar branch.
- Run `/check` (cargo check/clippy in `frontend/src-tauri`; `pnpm lint` in `frontend`;
  `./clean_run.sh`; record → live transcript → summary smoke).

## Sources

- Refined specs: `specs/0015-meetings-as-first-class-objects.md` (calendar_event_id, no attendee
  snapshot), `specs/0016-cross-meeting-identity-wave1.md` (People, `speakers.person_id`,
  `PeopleRepository`, explicit cascades), `specs/0012-people-and-role-weighted-summaries.md`
  (`people.role`). House style: `specs/0016`.
- Code anchors (verified 2026-06-27): `diarization/commands.rs:409` (`api_get_meeting_attendees`,
  by-id then title/time); `calendar/eventkit.rs:50,406,486` (`Attendee`, `event_attendees`,
  `event_attendees_by_id`); `calendar/day_agenda.rs` (`MAX_INLINE_ATTENDEES`, `attendee_count`);
  `diarization/settings.rs:121` (`resolve_speaker_count`, `SpeakerCountSource`);
  `diarization/sherpa.rs:56` (`SpeakerCount`), `:437` (`diarize_with_embeddings`);
  `diarization/pipeline.rs:617` (`resolve_meeting_speaker_count`), `:670` (`lookup_calendar_attendees`),
  `:495` (`seeded`/emit); `diarization/embedding.rs:48,62,88` (`l2_normalize`, `cosine_similarity`,
  `weighted_mean`); `database/repositories/people.rs` (`PeopleRepository`),
  `database/repositories/speaker.rs`, `database/repositories/meeting.rs` (delete cascade);
  `people/commands.rs` (People IPC), `lib.rs:741,877,887` (command registration);
  `app/meeting-details/page-content.tsx`, `components/MeetingDetails/MeetingIdentityHeader.tsx`,
  `components/MeetingDetails/SpeakerLegend.tsx`, `hooks/useSpeakers.ts`,
  `app/people/PersonFormDialog.tsx`, `components/GlobalRecordingBar.tsx`;
  migrations dir (latest `20260628000002_add_voiceprints.sql`).
