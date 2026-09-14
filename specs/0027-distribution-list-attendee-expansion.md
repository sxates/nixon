# 0027 — Distribution-List Attendee Handling & Expansion

- **Status:** Draft (design — deferred; likely depends on a real Google Calendar API integration,
  see Approach Phase 2′). Not part of the 1.2 release.
- **Owner agent(s):** rust-core-engineer + frontend-engineer
- **Roadmap phase:** Post-1.2 (graduated from `specs/0024` WS2.2)

> **Direction decided (owner, 2026-06-30):** true distribution-list expansion most likely requires
> a **real Google Calendar connection** (Calendar API over OAuth) rather than the current local
> **EventKit / iCal** read. EventKit hands us the DL as one opaque address and never the members;
> Google Calendar's API can return the full expanded invitee list for an event (and groups can be
> resolved via the Directory/People APIs), so the realistic path to "see the individuals" is a
> first-class Google Calendar integration. That is a larger piece of work — this spec is **deferred**
> until that integration is scoped. The near-term, no-integration slice (detect + label + manual
> add, Phases 1 & 3) can still land independently if we want correctness without expansion.

## Context / Problem

Graduated from `specs/0024` (1.1 feedback note 1). When a calendar invite's attendee is a
distribution list / group / mailing list, Vinyl shows **one** attendee (the list's address)
instead of the individual members.

**Root cause & hard constraint:** `attendees_from_event` (`eventkit.rs:531-553`) emits exactly one
`Attendee` per `EKParticipant`, and EventKit returns a DL as a **single** participant (the list's
address) — it does **not** expand server-side groups. `seed_from_attendees`
(`meeting_participant.rs:167-215`) then creates one `Person`/roster row from it. There is **no
local roster to expand from**, so true expansion requires an additional data source.

This is its own spec because the only paths to real expansion involve a new permission (Contacts)
or a directory integration — a product/permission decision, not a quick fix.

## Goals

- Don't mistake a distribution list for a person: never bind a DL row to a speaker/voiceprint or a
  cross-meeting identity.
- Give the user a usable way to populate the real attendees of a DL-invited meeting.
- Where locally possible, expand a DL to its members automatically.

## Non-goals

- A general corporate-directory / LDAP / Graph API integration (could be a far-future spec).
- Server-side group resolution (EventKit/macOS does not expose it).

## Approach (phased — recommend Phase 1 + 3 for the near term)

1. **Detect & label (minimum viable).** Use EventKit participant type/role
   (`eventkit.rs:531-558`) to recognize a non-individual participant and mark the roster row as a
   distribution list ("Distribution list — members unknown"). Exclude DL rows from speaker/voice
   binding and cross-meeting identity matching so a real voice is never attached to a list.
2. **Expand via local Contacts (optional, permission-gated).** If the DL address resolves to a
   macOS Contacts *group*, expand to its members. Requires Contacts permission and only works for
   groups the user has defined locally — limited, but free where it applies.
2′. **Expand via Google Calendar API (the realistic path — preferred over Phase 2).** Replace/augment
   the EventKit read with a real Google Calendar connection (OAuth). Google returns the event's full
   expanded invitee list, and group addresses can be resolved via the Directory/People APIs — so the
   members actually become visible. This is the owner-preferred direction but is a **larger
   integration** (auth, token storage, sync, privacy review) and gates this whole spec.
3. **Manual expand (escape hatch).** Let the user add the real members to a DL row
   (paste emails / pick from People) in the Participants UI, replacing the single list row.

Recommend Phase 1 (so identity stays correct) + Phase 3 (so the roster can be made right) as a
near-term, no-integration slice if needed; **true expansion (Phase 2′) waits on the Google Calendar
integration.** Phase 2 (local Contacts) is a lesser fallback and probably not worth its permission
prompt given 2′ is the real answer.

## Design

### Backend
- DL detection in `frontend/src-tauri/src/calendar/eventkit.rs:531-558` (participant type).
- A "distribution list" marker on the participant/roster (column or role value) in
  `frontend/src-tauri/src/database/repositories/meeting_participant.rs:167-215`; ensure
  diarization/identity skip DL rows.
- Phase 3: a command to replace a DL row with explicit members.

### UI
- `frontend/src/components/Participants/ParticipantsPanel.tsx` — render the DL marker; offer
  "Add members" for a DL row.

## Tasks

1. [ ] (rust) Detect DL/non-individual participants from EventKit; persist a marker; exclude from
   speaker/identity binding.
2. [ ] (frontend) Show the DL marker; "Add members" flow (Phase 3).
3. [ ] (decision) Resolve the Contacts-permission question (Phase 2) before building local-group
   expansion.

## Acceptance criteria

- Tie back to the Definition of Done in `/CLAUDE.md`.
- A DL attendee is clearly labeled as a list, is never bound to a speaker/voice, and the user has a
  way to add the real members.

## Risks / open questions

- **DECISION NEEDED — Contacts permission:** Phase 2 (local-group expansion) requires the macOS
  Contacts permission. Worth the added permission prompt for the limited set of locally defined
  groups, or skip to manual-only? This blocks Phase 2.
- **EventKit type fidelity:** confirm EventKit actually flags a DL participant distinctly (type/
  role) vs. just an address — if not, detection falls back to heuristics (no personal name, group-
  like address) which are imperfect.

## Verification

- Rust: `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal && cargo
  clippy && cargo test --features metal`.
- Frontend: `cd frontend && pnpm lint && pnpm test`.
- Manual: an invite with a DL shows a labeled list row, not a fake person; adding members replaces
  it with the real roster; the list is never offered as a speaker match.
