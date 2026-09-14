# 0012 — Persistent People entity & role-weighted summaries

- **Status:** Done (2026-06-27) — Tasks 1-4 (People entity / `person_id` / matcher) landed with `specs/0016` Wave 1b/1c; Tasks 5-6 (role-weighted summary + eval) landed in commit `a8f4faf`.
- **Owner agent(s):** rust-core-engineer (People entity/DB/IPC) + llm-pipeline-engineer (role-weighted
  summarization + evals) + frontend-engineer (People UI)
- **Roadmap phase:** Phase 3 — Speaker diarization, slice P3 (Task 12 of `specs/0010`, split out)
- **Parent:** `specs/0010-speaker-diarization.md` (Task 12); **depends on** `specs/0011` (embeddings +
  cross-meeting match must land first)

## Context / Problem

After `specs/0011`, Vinyl knows *who spoke* in a meeting (live + offline labels) and can *suggest* a
name for a recurring voice by matching this meeting's embeddings against prior meetings' identified
speakers (keyed by `email`). But identity is still **per-meeting**: each `speakers` row belongs to one
meeting, and the same person across ten meetings is ten unrelated rows that merely happen to share an
email. There is no app-wide notion of a **person**, and therefore no place to attach durable, useful
attributes — most importantly a **role/seniority** ("CEO", "Eng lead", "Customer").

This matters because it turns diarization from *"who spoke"* into *"whose input matters."* The
notes-aware summary (`specs/0003`, `summary/processor.rs`) already attributes points and action items to
speakers (P2 Task 9). A persistent People directory with roles lets the summary **weight contributions
by role** — a directive from a CEO or a decision from the meeting owner carries more signal than an
aside from an observer — which is a concrete step toward the granola.ai quality bar.

### Grounding (verified in this repo)

- **`email` is already the stable cross-meeting identity key.** P2's `assign_speaker_to_attendee`
  (`diarization/commands.rs`) writes `speakers.email` from the calendar roster; the migration comment
  (`20260625000000_add_speaker_email.sql`) explicitly calls email "the foundation for the P3 People
  entity." A People directory keyed by email is the natural promotion.
- **`speakers.embedding` is populated by 0011** (remote speakers, L2-normalized f32 BLOB +
  `embedding_model`). A `people` row can carry a representative embedding so a *named but un-invited*
  person (no calendar event) can still be recognized — and so People works on the same match machinery
  0011 builds in `diarization/identity.rs`.
- **The summary already consumes per-speaker labels and needs no new plumbing for metadata.**
  `summary/processor.rs::build_speaker_attributed_transcript()` + `SPEAKER_ATTRIBUTION_INSTRUCTIONS`
  prefix each transcript line with the resolved display name. Role-weighting extends the *prompt*
  (and/or pre-summary selection), not the capture/IPC stack — which is why Task 12 was separable from
  0011's audio/identity work.
- **Settings/JSON + sqlx patterns are established.** `diarization/settings.rs` (JSON in the
  identifier-derived `app_data_dir`, ADR-0004) and `SpeakersRepository` (`database/repositories/`) are
  the templates a `people` table + repository follow.

## Goals

- **A persistent, app-wide `people` entity** keyed by email, with: display name, optional role/seniority,
  optional representative voice embedding, optional notes/aliases. One person, many meetings.
- **Promote/link diarized speakers to People.** When a user names a speaker (calendar pick-list or a
  cross-meeting suggestion from 0011), that action creates-or-links a `people` row; future meetings with
  the same email/voice resolve to the same person automatically (suggested, confirmable).
- **Capture & edit roles.** A lightweight way to set a person's role (free-text + a small preset list);
  editable from a People view and inline from a meeting's speaker legend.
- **Role-weighted summaries.** The notes-aware summary (`specs/0003`) uses per-speaker role to weight
  attribution/emphasis (e.g. surface decisions/directives from higher-authority roles more prominently;
  bias action-item ownership). Evaluated to show it actually improves summaries.
- **100% on-device.** People, roles, and embeddings live only in the local DB; role text may be included
  in the summary prompt to the user-chosen LLM (that's the existing summary egress), but embeddings never
  are — same invariant as 0011.

## Non-goals

- **Live diarization / cross-meeting *matching* mechanics** — owned by `specs/0011`; this spec consumes
  them.
- **A CRM / contact-management product** (org charts, contact sync, photos). Roles here serve
  summarization, not contact management.
- **Explicit voice enrollment** ("record 10 s to register"). People embeddings are populated
  opportunistically from meetings (via 0011), as before; enrollment is out of scope.
- **Auto-assigning a role from title/email domain.** Roles are user-set (with presets); no inference.

## Approach

**Introduce a `people` table keyed by email, link `speakers.person_id` to it, and pass per-speaker role
into the existing summary prompt as a weighting signal.** ✅

- **People as the durable identity; speakers as the per-meeting occurrence.** A `speakers` row gains a
  nullable `person_id`. Naming a speaker (via calendar or a 0011 suggestion) upserts a `people` row
  (matched by email, else created) and sets `speakers.person_id`. The `people` row accumulates the best
  representative embedding (so voice-only recognition improves over time) and holds the role.
- **Recognition reuses 0011's matcher.** `diarization/identity.rs` already does cosine match of a new
  meeting's speakers vs prior identified speakers; extend it to also match against `people.embedding`,
  so a person is recognized even without a calendar event. The suggestion UI from 0011 gains "person"
  as a source ("Looks like Priya · CEO · 4 prior meetings").
- **Role-weighting in the summary, prompt-side first.** Lowest-risk, most-controllable: the
  speaker-attributed transcript builder annotates lines (or a per-speaker preamble) with role, and
  `SPEAKER_ATTRIBUTION_INSTRUCTIONS` is extended to tell the model to weight decisions/directives/action
  items by role. This is tunable and explainable. A stronger variant (role-aware *selection* of which
  utterances enter a length-limited summary) is a possible follow-up once the prompt approach is
  evaluated. **Default: prompt-side weighting; measure before doing selection-side.**

## Design

### Data model

Forward-only migrations under `frontend/src-tauri/migrations/`:

- **`…_add_people_table.sql`** (new):
  ```sql
  CREATE TABLE IF NOT EXISTS people (
      id              TEXT PRIMARY KEY,          -- "person-<uuid>"
      email           TEXT UNIQUE,               -- stable identity key (nullable: voice-only people)
      display_name    TEXT NOT NULL,
      role            TEXT,                      -- free-text/preset, e.g. "CEO","Eng lead","Customer"
      embedding       BLOB,                      -- representative voiceprint (L2-normalized f32 LE)
      embedding_model TEXT,                      -- which model produced `embedding` (compat, per 0011)
      notes           TEXT,
      created_at      TEXT NOT NULL,
      updated_at      TEXT NOT NULL
  );
  CREATE UNIQUE INDEX IF NOT EXISTS idx_people_email ON people(email) WHERE email IS NOT NULL;
  ```
- **`…_add_speaker_person_id.sql`** (new): `ALTER TABLE speakers ADD COLUMN person_id TEXT;` (nullable;
  links a per-meeting speaker to a person). No FK enforcement needed (the app pool runs without
  `PRAGMA foreign_keys`; mirror the existing speakers cascade-on-delete handling in
  `repositories/meeting.rs`).
- **`PeopleRepository`** (`database/repositories/people.rs`, new): `upsert_by_email`, `get_by_id`,
  `get_all`, `set_role`, `update_embedding` (keep the best/centroid representative), `link_speaker`.
- The `email`/`embedding`/`embedding_model` columns on `speakers` (from P2 + 0011) are the bridge; no
  changes to `transcripts`.

### Module / IPC

- **`diarization/identity.rs` (extend, from 0011):** also match new speakers against `people.embedding`;
  suggestion source ranking becomes: exact `email` match (calendar) > `people` embedding match > prior
  `speakers` embedding match.
- **People resolution at name-time:** `assign_speaker_to_attendee` and the 0011 suggestion-accept path
  call `PeopleRepository::upsert_by_email` + `link_speaker` + `update_embedding`.
- **Summary (`summary/processor.rs`):** `build_speaker_attributed_transcript` gains a per-speaker role
  map (resolved `speakers.person_id → people.role`); annotate the transcript/preamble with roles; extend
  `SPEAKER_ATTRIBUTION_INSTRUCTIONS` with role-weighting guidance. Byte-identical output when no roles
  are set (preserve the existing no-speaker / no-role fast path).
- **Tauri commands (`lib.rs`):** `api_get_people()`, `api_get_person(id)`, `api_set_person_role(id,
  role)`, `api_update_person(id, …)`; `api_get_meeting_speakers` extended to include
  `person_id`/`role`.

### UI (`frontend/src/`)

- **People view** (new route, e.g. `app/people/`): list of people with role, meeting count, edit role
  inline. Reuses the existing list/table styling.
- **Speaker legend** (`components/MeetingDetails/SpeakerLegend.tsx`): show role next to a named speaker;
  inline "set role" when a speaker is linked to a person; the 0011 suggestion chip gains role context.
- **Summary view:** unchanged surface; output improves. Optionally a small "weighted by role" hint.

## Tasks (ordered; owner agents in bold) — *all blocked on `specs/0011`*

1. [ ] **rust-core-engineer** — migrations `…_add_people_table.sql`, `…_add_speaker_person_id.sql`;
   `PeopleRepository`.
2. [ ] **rust-core-engineer** — link speakers→people on name/assign + suggestion-accept; carry the
   representative embedding onto the `people` row; extend `identity.rs` to match against `people`.
3. [ ] **rust-core-engineer** — People IPC commands; extend `api_get_meeting_speakers` with
   `person_id`/`role`; register in `lib.rs`.
4. [ ] **frontend-engineer** — People view + inline role editing; role in the speaker legend; role
   context in the 0011 suggestion chip.
5. [ ] **llm-pipeline-engineer** — role-weighted summary: role map into
   `build_speaker_attributed_transcript`; extend `SPEAKER_ATTRIBUTION_INSTRUCTIONS`; preserve the
   no-role byte-identical fast path.
6. [ ] **llm-pipeline-engineer** — eval: a small fixture set showing role-weighting improves
   decision/action-item attribution vs the P2 (un-weighted) baseline; record the result.

## Acceptance criteria

Tie back to the Definition of Done in `/CLAUDE.md`.

- **People persist & link:** naming a speaker creates/links a `people` row by email; a later meeting with
  the same person resolves to the same `people.id` (suggested, confirmable).
- **Roles capture & edit:** setting a role in the People view (or inline) persists and appears in that
  person's future meetings.
- **Role-weighted summary:** with roles set, the summary demonstrably emphasizes higher-authority
  contributions on the eval fixture; with **no** roles set, summary output is byte-identical to the P2
  baseline (no regression).
- **Privacy:** `people.embedding` is never sent to any LLM/network; role text travels only in the
  existing summary prompt egress; packet-capture clean apart from the chosen LLM call.
- **Gate (`/check`):** cargo check/clippy clean (`frontend/src-tauri`); pnpm lint clean (`frontend`);
  app launches via `./clean_run.sh`; record→transcript→summary smoke unchanged.

## Risks / open questions

- **Role weighting that distorts truth.** Over-weighting authority could drop a junior person's correct
  action item. *Mitigation:* weighting guides *emphasis/ownership phrasing*, never *omission*, in the
  prompt approach; the eval (Task 6) is the guardrail. Selection-side weighting only after eval.
- **Email collisions / shared inboxes / no-email people.** `people.email` is nullable and unique-when-
  present; voice-only people (no email) rely on embedding match (weaker). Merge-people UX may be needed
  later (out of scope here; note it).
- **Role taxonomy.** Free-text vs fixed presets. *Recommendation:* free-text with a small preset
  suggestion list; no inference.
- **Privacy optics of an app-wide People directory.** It's all local (ADR-0004 isolation), but document
  it clearly; consider a "forget this person" delete.

## Verification

- **Persistence/link smoke:** name a person in meeting A; record meeting B with them; confirm same
  `people.id` is suggested and role carries over.
- **Summary eval:** run the Task 6 fixture; confirm role-weighting improves attribution vs baseline and
  no-role output is unchanged.
- **Privacy:** packet-capture during summary generation shows only the chosen LLM endpoint; assert no
  embedding bytes in any payload.
- Run `/check`.

## Sources

- `specs/0010-speaker-diarization.md` (Task 12; "Future enhancement (P3+) — persistent People & roles").
- `specs/0011-diarization-p3-live-and-cross-meeting-identity.md` (embeddings + `diarization/identity.rs`
  matcher this spec extends).
- `specs/0003-note-enhancement-pipeline.md` + `frontend/src-tauri/src/summary/processor.rs`
  (`build_speaker_attributed_transcript`, `SPEAKER_ATTRIBUTION_INSTRUCTIONS`).
- `frontend/src-tauri/migrations/20260625000000_add_speaker_email.sql` (email = People foundation);
  `frontend/src-tauri/src/diarization/commands.rs` (`assign_speaker_to_attendee`).
