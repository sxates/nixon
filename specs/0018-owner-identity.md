# 0018 — Owner identity: claim a participant as "me" & durable owner email(s)

- **Status:** Done (2026-06-27) — built + verified (cargo check/clippy clean, +12 tests, pnpm lint clean). Needs a bundled-app runtime smoke. Decisions: voiceprints re-point to owner on claim; add-owner-email runs a backfill sweep; no un-claim in v1.
- **Owner agent(s):** rust-core-engineer (lead: `owner_emails` migration + repository, the
  merge-person-into-owner transaction, owner-aware seeding/identity exclusion, IPC) +
  frontend-engineer ("Your email addresses" Settings section + a "This is me" action on the
  participants panel). No audio-engineer work — the diarization-cap change is purely that a
  claimed person leaves the roster.
- **Roadmap phase:** Phase 3 — completes the identity work shipped in `specs/0016`/`0017`.
- **Refines:** `specs/0016-cross-meeting-identity-wave1.md` (the owner singleton
  `person-owner-self` / `OWNER_PERSON_ID`, `PeopleRepository`, enroll-on-confirm,
  `assign_speaker_to_attendee`), `specs/0017-meeting-participants.md` (`meeting_participants`,
  `seed_from_attendees`, `remote_person_ids`, the participants panel/popover).
- **No ADR.** Adds no biometric storage and no new outbound traffic. `owner_emails` is a tiny
  local lookup table (email → "this is me"); the owner identity, voiceprint gates (ADR-0007),
  and `is_current_user` semantics are unchanged in kind — we only widen "who is me" from a
  single calendar-account address to a user-curated set, and add a merge operation that
  re-points existing rows onto the existing owner person.

## Context / Problem

> *"How does the app identify 'me' on meeting invites? I just see it adding a participant with
> my email. I need a way to claim that as 'me' not another participant."*

The app's **only** signal for "this attendee is me" is EventKit's per-attendee
`isCurrentUser()`. That flag trips only when the invite address matches the *calendar
account's own* address. If you're invited under a different work alias, a personal address, or
a delegated address, EventKit returns `is_current_user = false`, so `specs/0017`
`seed_from_attendees` treats you as an ordinary invitee: it auto-creates a `people` row for
your address and rosters you as a participant. You then appear in your own meeting's roster,
your address inflates the diarization speaker cap, and your own voice can map to a separate
"person" instead of "You".

### Grounding (verified in this repo, 2026-06-27 — state of the art; do not re-derive)

- **The sole "me" signal is `is_current_user`.** `calendar/eventkit.rs:544`
  `participant.isCurrentUser()` → `Attendee { name: String, email: Option<String>,
  is_current_user: bool }` (`eventkit.rs:50`). It is the calendar account address only; a
  different alias is **not** detected.
- **The owner is a singleton `people` row.** `people/enroll.rs:34`
  `OWNER_PERSON_ID = "person-owner-self"`, display name `"You"`, created lazily by
  `ensure_owner_person` with **`email = NULL`**, `role = NULL`, `voiceprint_opt_out = 0`. It is
  found by the well-known id, not by a flag column. Because it has no email it cannot match any
  calendar attendee by address.
- **`specs/0017` is shipped.** `database/repositories/meeting_participant.rs`:
  `MeetingParticipantsRepository::{list, add, remove, count_for_meeting, remote_person_ids,
  seed_from_attendees}`. `seed_from_attendees` (`:163`) skips an attendee only when
  `a.is_current_user`, else upserts a person via `PeopleRepository::create(name, email)` and
  `add(.., "calendar")`. `remote_person_ids` (`:132`) already hard-excludes `OWNER_PERSON_ID`
  and is the diarization cap source.
- **No "my email(s)" setting exists** (grep confirmed: no `owner_email`/`ownerEmail` anywhere,
  Rust or TS). `RecordingSettings.tsx` has Speaker-diarization + the two voiceprint-consent
  toggles (`storeOthersVoiceprints` default off, `selfEnrollVoiceprint` default on) — the
  natural neighborhood for a "Your email addresses" section.
- **`PeopleRepository`** (`database/repositories/people.rs`): `create` is upsert-by-email
  (normalizes empty → NULL, mints `person-<uuid>`, returns the existing person on email
  collision); partial-unique index `idx_people_email ... WHERE email IS NOT NULL`; `get`,
  `get_by_email` (COLLATE NOCASE), `update`, `delete` (explicit cascade tx: NULLs
  `speakers.person_id`, deletes `meeting_participants` + voiceprints, then the row),
  `assign_speaker_to_person` (sets `speakers.person_id` + copies name/email onto the speaker).
- **`assign_speaker_to_attendee`** (`diarization/commands.rs:298`) upserts a person by email,
  links the speaker, then enrolls a voiceprint behind the consent gate — **even if the address
  is the owner's**, because nothing there consults the owner.
- **Diarization owner exclusion today** = `is_current_user` at seed time +
  `remote_person_ids`'s `OWNER_PERSON_ID` exclusion. There is no email-based owner exclusion.
- **Participants UI:** `components/Participants/ParticipantsPanel.tsx` (chips with an inline ×
  remove + click-to-edit-person) and `ParticipantsPopover.tsx` (in-recording). Commands live in
  `diarization/commands.rs` (`api_get_meeting_participants`/`api_add_meeting_participant`/
  `api_remove_meeting_participant`, registered `lib.rs:881-883`).
- **No `PRAGMA foreign_keys`.** Every cascade is explicit in the delete/merge transaction.

## Goals

- **A durable, multi-valued source of truth for "which addresses are me":** a new
  `owner_emails` table. Supports work + personal + alias addresses (the whole reason
  `is_current_user` misses), normalized lowercase/trimmed.
- **Resolve every owner email to the existing owner person** (`person-owner-self`). Keep the
  owner row's `email` NULL — `owner_emails` is the multi-email home.
- **A one-click "This is me"** on each participant row that, atomically, (1) adds the person's
  email to `owner_emails`, (2) **merges that auto-created person into the owner** (re-points
  speakers/voiceprints, un-rosters them everywhere, deletes the empty person), and (3) is
  idempotent if the person is already the owner. After claiming, the row disappears from the
  roster (you're not a "participant").
- **Owner-aware seeding & identity** everywhere "is this me?" is asked: an attendee whose email
  is an owner email is treated exactly like `is_current_user` (skipped in seeding; mapped to the
  owner in `assign_speaker_to_attendee`). So once claimed, future calendar seeds never re-add you.
- **A proactive Settings path** ("Your email addresses": list / add / remove) so you can declare
  your aliases up front, before any meeting.
- **Backfill on add:** adding an owner email sweeps existing rosters, removing any
  `meeting_participants` whose person's email matches (and folding those people into the owner),
  so previously-seeded "me" rows clean up.
- On-device; no new outbound traffic; no new dependency; no new model.

## Non-goals

- **No change to the owner person's identity model.** Still the singleton `OWNER_PERSON_ID`
  row, still found by id, still `email = NULL`. We do not add an email column to the owner.
- **No change to voiceprint *consent*.** The two toggles and ADR-0007 gates are untouched; we
  only decide *which person* a claimed voice belongs to (the owner), still subject to the
  owner's `self_enroll_voiceprint` gate at future enroll sites.
- **No "un-claim"/un-merge.** A merge is one-way (the inverse is intentionally absent —
  removing an owner email stops *future* matching but does not un-merge already-folded people).
  Settings remove is the address-level inverse.
- **No general merge-people UX.** "This is me" is a *special* merge into the reserved owner;
  arbitrary person↔person merge stays a later wave.
- **No contact-book/recurring-series sync, no per-account multi-owner.** One owner identity per
  install (matching today's single `person-owner-self`).

## Approach

**Add `owner_emails` as the single source of truth for "which invite addresses are me," and
make `person-owner-self` the person those addresses resolve to. Everywhere the code asks "is
this me?" — seeding, attendee-assign, the diarization cap — answer it with `is_current_user OR
lower(email) ∈ owner_emails`. The headline "This is me" action records the email and performs a
single transactional merge of the auto-created person into the owner; the cleanest enforcement
of the diarization-cap and roster rules is that a claimed person simply *leaves the roster*, so
no cap math changes.**

Why this shape, briefly:

- **A table, not a column on the owner row.** Multiple addresses is the requirement, and a
  per-address row keeps `created_at` and a clean PK; cramming a delimited list into
  `people.email` would break the partial-unique index and `get_by_email`. The owner stays
  email-NULL so it never collides in `idx_people_email`.
- **Merge into the existing owner, don't relabel the person.** The auto-created participant
  person may already carry speakers/voiceprints/roster rows across meetings. Re-pointing those
  onto `OWNER_PERSON_ID` and deleting the now-empty person keeps a *single* owner identity (the
  invariant `specs/0016` established) instead of leaving two "owner-ish" people.
- **Roster departure is the cap enforcement.** `remote_person_ids` already excludes
  `OWNER_PERSON_ID`; merging removes the claimed person's `meeting_participants` rows entirely.
  So a claimed person can never count toward `AtMost(n)` — **no `diarization/settings.rs` or
  `pipeline.rs` change is needed.** (Belt-and-suspenders: seeding also skips owner emails, so
  they never re-enter the roster.)
- **Voiceprints on claim → re-point to the owner (decision V, below).** It *is* the owner's
  voice; the owner is then subject to its own `self_enroll` gate going forward. We do not delete
  on claim (that would silently discard a legitimately-captured sample the user already
  consented to when it was a normal person), and we do not gate the re-point (the samples
  already exist; we're correcting *whose* they are, not enrolling new ones).

### Decisions to confirm (call-outs for the user)

- **(V) Voiceprint handling on claim — re-point to the owner.** Recommended over delete. See
  "Risks". If you'd rather a claim *delete* the folded person's voiceprints (strictest privacy
  posture, but loses a real sample of your own voice), say so and Task 3 flips one statement.
- **(F) Backfill sweep on add-owner-email — do it.** Cheap, matches intent, and means declaring
  an alias in Settings retroactively cleans rosters that already list you. Spec'd as the default.

## Design

### Data model — migration `frontend/src-tauri/migrations/20260630000000_add_owner_emails.sql`

Next prefix after `20260629000000_add_meeting_participants.sql`. Additive, idempotent,
forward-only; no `PRAGMA foreign_keys` (the merge cascade is explicit in Rust).

```sql
-- specs/0018: the source of truth for "which invite addresses are ME (the device owner)".
-- EventKit's per-attendee isCurrentUser() only flags the calendar account's own address, so
-- aliases / personal / delegated addresses are missed and get seeded as ordinary
-- participants. These rows say "this address resolves to the owner person (person-owner-self)".
-- Multi-valued by design (work + personal + aliases). Email is stored NORMALIZED: lowercased
-- and trimmed at the write boundary, so the PK does the dedupe and lookups are exact.
-- The owner person keeps email = NULL; this table is the multi-email home (so it never
-- collides under people's partial-unique idx_people_email).
CREATE TABLE IF NOT EXISTS owner_emails (
    email      TEXT PRIMARY KEY,   -- normalized: lower(trim(email))
    created_at TEXT NOT NULL
);
```

No FK to `people`; the owner person is the well-known `OWNER_PERSON_ID` and may not exist yet
when the first email is added (it's created lazily). The repository ensures the owner person on
the add/claim path.

### Repository — `database/repositories/owner_emails.rs` (new)

`OwnerEmailsRepository` (mirrors the house style; returns `SqlxError`; the command layer maps
to strings). A module-level `pub fn normalize_email(&str) -> Option<String>` (lower + trim,
`None` for empty) is the single normalization point, reused by seeding and assign so "is this
me?" is byte-consistent everywhere.

- `list(pool) -> Vec<String>` — all owner emails, ordered, for Settings + the membership set.
- `contains(pool, email) -> bool` — `SELECT 1 ... WHERE email = ?` after `normalize_email`;
  the "is this me by address?" primitive. (Callers that already hold the full set may compare
  against `list` instead, to avoid N queries while seeding.)
- `add(pool, email) -> Result<bool>` — `INSERT OR IGNORE` the normalized email; returns whether
  inserted. Ensures the owner person exists first (`people::enroll::ensure_owner_person`) so a
  later match has a target.
- `remove(pool, email) -> Result<bool>` — `DELETE` the normalized email (address-level inverse;
  does **not** un-merge already-folded people).

### Merge person → owner (the heart of the claim) — `people::merge::merge_person_into_owner`

New module `src/people/merge.rs` (sibling of `enroll.rs`). One function, one transaction
(explicit cascade — no `PRAGMA foreign_keys`):

```rust
/// Fold an auto-created person into the singleton owner (person-owner-self). Idempotent and
/// a no-op when `person_id == OWNER_PERSON_ID`. Returns Ok(()) even if the person is absent.
pub async fn merge_person_into_owner(pool: &SqlitePool, person_id: &str) -> Result<()>
```

Steps inside `pool.begin()` (skip entirely if `person_id == OWNER_PERSON_ID`):

1. `ensure_owner_person` (idempotent; guarantees the target row).
2. **Speakers:** `UPDATE speakers SET person_id = ? WHERE person_id = ?` (→ owner). Keeps the
   per-meeting speaker rows; only the identity link moves. (We intentionally do **not** rewrite
   `speakers.display_name`/`email` to "You" — the transcript label is a separate concern; the
   speaker→owner link is what matters for "this is me".)
3. **Voiceprints (decision V — re-point):**
   `UPDATE voiceprints SET person_id = ? WHERE person_id = ?` (→ owner). It is the owner's
   voice. No new enrollment occurs; future enrollment is still gated by `self_enroll_voiceprint`.
   *(Alternative if the user picks delete-on-claim: `DELETE FROM voiceprints WHERE person_id =
   ?` for the folded person instead — one-line swap.)*
4. **Roster:** `DELETE FROM meeting_participants WHERE person_id = ?` (the folded person) — the
   owner is not a participant in any meeting, so we remove rather than re-point (re-pointing
   could violate the `(meeting_id, person_id)` PK if the owner were already rostered, and the
   owner shouldn't be on rosters anyway).
5. **Delete the now-empty person:** `DELETE FROM people WHERE id = ?` (guarded `!= OWNER...`).
6. `commit`.

Idempotency: if called again with the same (now-deleted) id, steps 2–5 affect 0 rows and the
person-delete returns 0 — safe. If called with `OWNER_PERSON_ID`, returns early.

Unit-testable on an in-memory pool (mirror the `meeting_participant.rs` test schema: people +
speakers + voiceprints + meeting_participants): assert speakers/voiceprints re-pointed, roster
rows gone, person deleted, owner survives; re-run = no-op; owner-id = no-op.

### Owner-aware seeding & identity (where "is this me?" widens)

- **`MeetingParticipantsRepository::seed_from_attendees`** (`meeting_participant.rs:163`): load
  the owner-email set once at the top (`OwnerEmailsRepository::list`), and change the skip
  predicate from `a.is_current_user` to `a.is_current_user || owner_set.contains(normalize(a.email))`.
  So a claimed alias is never re-seeded. (Signature stays the same; it already has `pool`.)
- **`api_assign_speaker_to_attendee`** (`diarization/commands.rs:298`): before the
  upsert-person-by-email block, if `normalize_email(email)` ∈ owner emails, take the owner
  branch instead — `ensure_owner_person`, `assign_speaker_to_person(.., OWNER_PERSON_ID)`, then
  the existing enroll-on-confirm (which will route to the owner via the `is_local`/owner gate).
  Do **not** create a separate person for an owner address. (Leaves the non-owner path
  byte-identical.)
- **Diarization cap:** no change. `remote_person_ids` already excludes `OWNER_PERSON_ID`, and a
  claimed person has no `meeting_participants` rows after the merge, so it can't be counted. The
  belt-and-suspenders is that owner-aware seeding keeps it off the roster in the first place.

### Backfill sweep on add-owner-email (requirement F)

Inside `OwnerEmailsRepository::add` *after* the insert (only when a new row was inserted), run a
sweep so declaring an alias retroactively cleans rosters:

- Find people whose `email` matches the newly-added owner email: `PeopleRepository::get_by_email`
  (COLLATE NOCASE). If found and it's not already the owner, `merge_person_into_owner(pool,
  found.id)`. That removes their `meeting_participants` rows app-wide and folds them in.

This is the same merge the claim path uses, so a Settings-declared alias and a per-meeting claim
converge on identical state. The sweep is one `get_by_email` + at most one merge — cheap.

### Tauri IPC (register in `frontend/src-tauri/src/lib.rs`; new `people/commands.rs` entries or a small `owner/commands.rs` — fold into `people/commands.rs` for cohesion)

- `api_get_owner_emails() -> Vec<String>` — `OwnerEmailsRepository::list`. Settings list.
- `api_add_owner_email(email: String) -> Vec<String>` — validate non-empty (a light email
  shape check, mirroring `assign_speaker_to_attendee`'s trim guards); `add` (+ the backfill
  sweep); return the updated list. User-actionable error on failure.
- `api_remove_owner_email(email: String) -> Vec<String>` — `remove`; return the updated list.
- `api_claim_participant_as_me(meeting_id: String, person_id: String) -> ()` — the headline,
  one transaction's worth of effect from the caller's view:
  1. Load the person (`PeopleRepository::get`). If `person_id == OWNER_PERSON_ID`, return
     `Ok(())` (already me).
  2. If the person has an email, `OwnerEmailsRepository::add(email)` (records it as me; its
     internal sweep is harmless/idempotent here). If email-less, skip the add but still merge —
     note in the response/log that without an email future calendar seeds can't auto-match
     (Settings is the email path).
  3. `merge_person_into_owner(pool, person_id)`.
  Returns `Ok(())`; the panel drops the row optimistically. Errors are user-actionable.
- No new events; the panel re-fetches / mutates local state after the call. `meeting_id` is
  accepted for symmetry/telemetry and future per-meeting affordances but the merge is app-wide
  (claiming is an identity statement, not a per-meeting one).

### UI (`frontend/src/`)

- **Settings — "Your email addresses"** (`components/RecordingSettings.tsx`, placed adjacent to
  the voiceprint-consent block so all identity controls sit together): a section that lists
  `api_get_owner_emails`, an add input (→ `api_add_owner_email`, optimistic, refresh from the
  returned list), and a × per address (→ `api_remove_owner_email`). Copy: *"Tell Vinyl which
  invite addresses are you. You won't be added as a participant in your own meetings, and your
  voice maps to 'You'. Add any work, personal, or alias addresses you're invited under."* A note
  on remove: *"Removing an address stops future auto-matching; it won't undo people already
  merged into you."*
- **Participants panel — "This is me"** (`components/Participants/ParticipantsPanel.tsx`, and so
  `ParticipantsPopover.tsx` via the shared panel): add an action on each `ParticipantChip` —
  an unobtrusive inline button or an overflow item next to the existing × — labeled "This is
  me". On click → `api_claim_participant_as_me({ meetingId, personId })`, **optimistic**: remove
  the chip immediately, `toast.success("Claimed <email-or-name> as you")`; revert + error toast
  on failure (mirror `handleRemove`). Keep the existing remove (×) distinct — "remove from this
  meeting" vs "this is me" are different intents. Add a `Person` type note: nothing new on the
  type; the claim takes `personId` from the roster row.
- No change to the speaker legend in v1 (a "this is me" on a *speaker* is a reasonable
  follow-on, but the user's ask is the participant roster).

## Tasks (ordered; phased; owner agents in bold)

### Phase A — Owner-email store + merge (rust-core)
1. [ ] **rust-core-engineer** — migration `20260630000000_add_owner_emails.sql`;
   `OwnerEmailsRepository` (`repositories/owner_emails.rs`: `normalize_email`, `list`,
   `contains`, `add` [ensure-owner + backfill sweep], `remove`); register in the repo module.
   Unit-test normalization + idempotent add + the sweep folding a name/email-matched person.
2. [ ] **rust-core-engineer** — `people/merge.rs::merge_person_into_owner` (one tx: re-point
   `speakers` + `voiceprints` to owner, delete the folded `meeting_participants`, delete the
   empty person; no-op for owner-id / absent person). Unit-test re-point, roster removal,
   delete, idempotency, owner-id no-op.

### Phase B — Owner-aware seeding/identity + IPC (rust-core)
3. [ ] **rust-core-engineer** — widen "is me" in `seed_from_attendees` (skip
   `is_current_user || owner-email match`) and in `api_assign_speaker_to_attendee` (owner
   branch → owner person, no separate person). Confirm/decide voiceprint-on-claim (V): default
   re-point in `merge.rs`. Update `seed_from_attendees`'s doc + add a test (an owner-email
   attendee is skipped).
4. [ ] **rust-core-engineer** — IPC `api_get_owner_emails` / `api_add_owner_email` /
   `api_remove_owner_email` / `api_claim_participant_as_me` in `people/commands.rs`; register in
   `lib.rs`. Wire `api_claim_participant_as_me` = add-email (if any) + merge, idempotent.

### Phase C — UI (frontend)
5. [ ] **frontend-engineer** — "Your email addresses" section in `RecordingSettings.tsx`
   (list/add/remove via the three commands), placed by the voiceprint controls, with the copy
   above.
6. [ ] **frontend-engineer** — "This is me" action on `ParticipantsPanel.tsx` chips (and thus
   the recording popover): optimistic claim → `api_claim_participant_as_me`, row disappears,
   success/revert toasts; keep distinct from the existing remove ×.

## Acceptance criteria (tie to the Definition of Done in `/CLAUDE.md`)

- **Claim removes me from the roster:** clicking "This is me" on a participant calls
  `api_claim_participant_as_me`; the chip disappears, and (`sqlite3`) that meeting's
  `meeting_participants` no longer has the person, the person row is gone, the owner row
  survives, and `owner_emails` contains the (lowercased) email when the person had one.
- **Future seeds skip me:** after claiming/declaring an alias, re-resolving (re-opening /
  re-seeding) a meeting whose calendar invite includes that address adds **no**
  `meeting_participants` row for it (`seed_from_attendees` returns 0 for that attendee);
  row-count stable across two seeds.
- **My voice maps to "You":** assigning a speaker to an attendee whose email is an owner email
  links the speaker to `OWNER_PERSON_ID` (not a new person), and enroll-on-confirm routes to the
  owner under the existing `self_enroll` gate (`speakers.person_id = person-owner-self`).
- **Multi-email supported:** adding two distinct owner emails yields two `owner_emails` rows;
  both are treated as me by seeding/assign.
- **Diarization cap no longer counts me:** a meeting where one invitee is a claimed owner email
  sizes the cap from `remote_person_ids` *excluding* that person (it's no longer rostered);
  `diarization-complete`'s `seededSpeakerCount` drops by one vs. before the claim. No change to
  `Fixed`/`Auto`/`AtMost` semantics otherwise.
- **No-owner-email behavior unchanged:** with `owner_emails` empty, seeding, assign, and the cap
  are **byte-identical to `specs/0017`** (the owner-set is empty → the new predicate reduces to
  `is_current_user`; the owner branch in assign is never taken).
- **Backfill on add:** adding an owner email that matches an already-rostered person folds that
  person into the owner and removes their `meeting_participants` rows app-wide (`sqlite3`).
- **Idempotent / safe:** claiming the same participant twice, or claiming `person-owner-self`,
  is a no-op (no error, no duplicate `owner_emails`, no orphaned rows). Email normalization
  means `Me@Work.com` and `me@work.com ` are one row.
- **Cascades intact:** "forget this person" and meeting-delete still behave per `specs/0016`/
  `0017`; merge doesn't strand speakers/voiceprints/roster rows.
- **Privacy:** no new outbound traffic; `owner_emails` stores only the user's own addresses
  locally; voiceprint *consent* gates are unchanged.
- **Gate (`/check`):** `cargo check` + `cargo clippy` clean (`frontend/src-tauri`); `pnpm lint`
  clean (`frontend`); app launches via `./clean_run.sh`; record → live transcript → summary
  smoke unchanged.

## Risks / open questions

- **Voiceprint-on-claim (decision V) — primary call.** Re-pointing folds a real sample of the
  owner's voice into the owner's gallery without a fresh consent event. Rationale it's fine: the
  sample was already captured and stored under the user's consent when the person was an ordinary
  attendee (gated by `store_others_voiceprints`), and we're correcting *attribution*, not
  enrolling. The alternative (delete on claim) is stricter but discards a legitimate sample.
  **Confirm V.** Either way, claiming never *adds* a new voiceprint; future enrollment for the
  owner stays behind `self_enroll_voiceprint`.
- **Merge correctness (no `PRAGMA foreign_keys`).** The whole point of one explicit transaction
  is to avoid stranding rows. Risk: a missed table. Today the only person-referencing tables are
  `speakers.person_id`, `voiceprints.person_id`, `meeting_participants.person_id` (all handled).
  Mitigation: grep `person_id` referencing tables when this lands and assert each is covered;
  unit test asserts no orphans.
- **Shared / alias addresses.** A shared mailbox (e.g. a team alias) claimed as "me" would then
  skip *everyone* invited under it from rosters. Mitigation: Settings copy says "addresses **you**
  are invited under"; the user controls the set and can remove a too-broad address. Removing an
  address stops future matching but does not un-merge — documented in the remove copy.
- **Email normalization edge cases.** We lowercase+trim only (no plus-addressing or
  unicode-domain canonicalization). Sufficient for invite matching; `get_by_email` is already
  COLLATE NOCASE so the sweep matches case-insensitively. Document that `a+tag@x.com` is a
  distinct address (the user can add both).
- **Email-less claim.** Claiming an email-less participant merges them but records no
  `owner_emails` row, so future calendar invites can't auto-match. Surfaced in copy/log; the
  Settings add-email path is the proactive fix.
- **One owner only.** `owner_emails` has no per-account scoping; all addresses resolve to the
  single `person-owner-self`. Matches today's model; multi-identity is out of scope.
- **Bare dev binary has no calendar access** (`specs/0015` caveat): the seed-skip path is fully
  testable only in the bundled `Vinyl.app`/`Dev Vinyl`. The merge, Settings emails, and assign
  owner-branch are testable anywhere (plain DB writes + a synthetic speaker).

## Verification

- **Claim flow (bundled app):** Join & Record a calendar event you're invited to under an
  *alias* (so `is_current_user` is false and you seed as a participant). Confirm you appear on
  the roster (`sqlite3 meeting_participants`). Click "This is me" → the chip vanishes; confirm
  the person row is gone, `owner_emails` has the lowercased alias, `speakers`/`voiceprints` for
  that id are re-pointed to `person-owner-self`, owner survives.
- **Future-seed skip:** re-open the meeting (lazy re-seed) and a *second* meeting with the same
  alias invite → no participant row for you in either; seed count 0 for that attendee.
- **Voice → You:** assign a speaker to your alias attendee → `speakers.person_id =
  person-owner-self`; with `self_enroll` on, a voiceprint lands under the owner.
- **Settings path + backfill:** with a roster that already lists you (pre-claim), add that
  address in Settings "Your email addresses" → the roster row disappears app-wide; the address
  shows in the list; remove it → it leaves the list (and a fresh invite under it would re-seed
  you, confirming remove stops future matching).
- **No-owner-email regression:** with `owner_emails` empty, repeat the `specs/0017` seeding +
  cap verification — behavior identical.
- **Unit tests:** `normalize_email`; `add` idempotent + sweep folds a matching person;
  `merge_person_into_owner` (re-point speakers/voiceprints, drop roster, delete person,
  idempotent, owner-id no-op); `seed_from_attendees` skips an owner-email attendee;
  `api_claim_participant_as_me` no-op on owner / on repeat.
- Run `/check` (cargo check/clippy in `frontend/src-tauri`; `pnpm lint` in `frontend`;
  `./clean_run.sh`; record → live transcript → summary smoke).

## Sources

- Refined specs: `specs/0016-cross-meeting-identity-wave1.md` (owner singleton, `PeopleRepository`,
  enroll-on-confirm, `assign_speaker_to_attendee`), `specs/0017-meeting-participants.md`
  (`meeting_participants`, `seed_from_attendees`, `remote_person_ids`, participants UI).
  House style: `specs/0016`/`0017`.
- Code anchors (verified 2026-06-27): `calendar/eventkit.rs:50,544` (`Attendee`,
  `isCurrentUser`); `people/enroll.rs:34,82` (`OWNER_PERSON_ID`, `ensure_owner_person`);
  `database/repositories/meeting_participant.rs:132,163` (`remote_person_ids`,
  `seed_from_attendees`); `database/repositories/people.rs:54,130,234,275` (`create`,
  `get_by_email`, `delete`, `assign_speaker_to_person`); `database/repositories/voiceprints.rs:175,188`
  (`delete_for_person_tx`/`delete_for_person`); `diarization/commands.rs:298`
  (`api_assign_speaker_to_attendee`); `diarization/settings.rs:122` (`resolve_speaker_count` —
  unchanged); `people/commands.rs`, `lib.rs:881-883` (IPC registration);
  `components/RecordingSettings.tsx` (voiceprint-consent section),
  `components/Participants/ParticipantsPanel.tsx`, `ParticipantsPopover.tsx`; migrations dir
  (latest `20260629000000_add_meeting_participants.sql`).
