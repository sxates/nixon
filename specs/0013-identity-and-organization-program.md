# 0013 — Identity & Organization Program (post-diarization synthesis)

- **Status:** Draft (program-level synthesis — **NO code changes**; sequences future specs)
- **Owner agent(s):** spec-architect (this synthesis) → fans out to rust-core-engineer,
  audio-engineer, llm-pipeline-engineer, frontend-engineer per wave
- **Roadmap phase:** spans Phase 2–5; reorders the backlog into one dependency-ordered program
- **Relationship to existing specs:** **extends/sequences** `specs/0011` (cross-meeting embeddings +
  matcher) and `specs/0012` (People entity + roles); **consumes** `specs/0008` (calendar/Zoom) and
  `specs/0003` (notes-aware summary). It does **not** re-specify the diarization mechanics in 0011/0012
  — see "Reconciliation with 0011/0012" below for exactly what 0013 supersedes vs. defers.

## Context / Problem

Brian logged a batch of backlog ideas (now scattered across `ROADMAP.md` Phases 2–5): notes-only
meetings, topic/category tags, action items, meeting analytics, pluggable Google Calendar,
per-meeting Zoom recording links, pre-call prep, Zoom-as-speaker-source, and the big one —
cross-meeting voiceprints / persistent People & roles.

Specced piecemeal these read as nine disconnected features. They are not. **Almost every item rests
on the same small set of shared primitives**, and building them as one sequenced program (foundation
first, consumers last) avoids re-litigating identity, aggregation, and standalone-content storage
three separate times. This spec is that synthesis: a vision, a shared data-model sketch, and a
**prioritized, dependency-ordered enhancement list** that respects Brian's stated priorities
(Google Calendar = high "big upgrade"; cross-meeting voiceprints = high keystone;
Zoom-as-speaker-source = low/complex).

### The five shared primitives (the whole argument)

1. **Persistent People / identity** — a stable *person* entity anchored by **email**, with voiceprints
   as an attached *matching signal* (not the identity itself). Cross-meeting voiceprints, People &
   roles, action-item owners, "feedback on a person", and "who do I meet with most" **all** resolve
   through this one entity. This is the **keystone** — most other items are dead without it.
2. **Cross-meeting aggregation** — roll-ups across meetings (topic summaries, analytics, pre-call
   prep). A query/summarization pattern that operates over *sets* of meetings rather than one.
3. **Standalone (non-recording-bound) content** — content that exists outside a single recording:
   notes-only meetings, topic notes, action items. Today every artifact (`meeting_notes`, `speakers`,
   `transcripts`) is keyed to one recorded `meetings` row; several backlog items break that assumption.
4. **The pluggable-provider pattern** — already proven for LLM providers
   (`src/{ollama,anthropic,openai,…}/`) and partially for calendar (`calendar/mod.rs` has a source
   abstraction). Extend it to **calendar** (EventKit vs. Google) and to **speaker-labeling**
   (diarization vs. Zoom transcript). Same shape, twice more.
5. **An explicit privacy posture** — anything that introduces **cloud outbound** (Google Calendar,
   Zoom cloud data) or **biometric storage** (voiceprints of other people) needs an **ADR** before
   build. Privacy is the product (`CLAUDE.md`); these are the items that can erode it, so they get the
   documented gate.

### Grounding (verified in this repo, 2026-06-25)

- **The keystone is already half-built.** `speakers.embedding BLOB` exists, explicitly "reserved for
  P3 cross-meeting identity" (`migrations/20260624000000_add_speakers_table.sql`); `speakers.email`
  exists (`…_add_speaker_email.sql`), written by P2's `assign_speaker_to_attendee`. `specs/0011`
  defines the embedding extraction + `diarization/identity.rs` cosine matcher; `specs/0012` defines the
  `people` table keyed by email. **0013 does not re-invent these — it sequences them first and shows
  what the later items hang off them.**
- **CAM++ embeddings already computed.** `3dspeaker_campplus_sv_en_voxceleb_16k`, sherpa-onnx, offline;
  `diarization/embedding.rs` (per 0011) owns `embedding_to_bytes`/`embedding_from_bytes`, L2-norm,
  cosine. The voiceprint gallery is a *storage + match-threshold* problem, not a new-model problem.
- **The calendar provider seam exists.** `calendar/mod.rs`, `calendar/eventkit.rs`,
  `calendar/zoom_link.rs`, `calendar/day_agenda.rs`, `calendar/commands.rs` shipped with `specs/0008`.
  0008 already recommended keeping a `CalendarSource` trait so a Google source can be added behind an
  explicit opt-in — exactly item #5. The seam is there; Google is a second impl + an ADR.
- **Standalone content has no home yet.** `meetings(id, title, created_at, updated_at)` has no
  type/origin column; `meeting_notes`, `speakers`, `transcripts` all FK to `meetings(id)` ON DELETE
  CASCADE. `meetings.folder_path` exists but is unused (`…_add_audio_sync_fields.sql`). There is **no**
  `calendar_event_id` column on meetings (0008 listed it as a deferred nice-to-have — it is now needed
  by pre-call prep). Topics, action items, and notes-only meetings are net-new tables/columns.
- **Summary already aggregates one meeting; cross-meeting roll-up is the new shape.**
  `summary/processor.rs` + `summary/service.rs` + the template system (`summary/templates/`) chunk and
  fill a report for **one** meeting's transcript+notes. Topic roll-ups and pre-call prep need the same
  LLM plumbing pointed at a *concatenation across meetings* — reuse `llm_client.rs` + a new prompt,
  not a new provider stack.

## Goals

- A single **prioritized, dependency-ordered program** (waves) that turns the nine backlog items into
  a buildable sequence, foundation-first.
- A **shared data-model sketch** showing the items reuse common tables (People/voiceprints,
  Topics/tags, Action items, standalone Notes) instead of inventing parallel structures.
- An explicit **reconciliation** with `specs/0011`/`0012`: what 0013 supersedes, extends, or merely
  sequences — so no one re-specs identity.
- An **ADR callout list** for the decisions that must be recorded before their feature is built.
- Honor stated priorities: **Google Calendar = high**, **cross-meeting voiceprints = high (keystone)**,
  **Zoom-as-speaker-source = low/complex**.

## Non-goals

- **Re-specifying diarization or the People entity.** 0011/0012 own those; 0013 references them.
- **Full DDL / IPC contracts for each item.** This is a program spec. Each wave item still graduates to
  its own numbered spec (`/spec`) with full Design/Tasks before build. The data-model here is a *sketch*
  to prove the items share tables.
- **Committing to cloud anything before its ADR.** Google Calendar and Zoom cloud items are sequenced
  but **gated** on their ADRs.
- **A CRM / contacts product, org-chart inference, multi-machine identity sync.** (Same exclusions as
  0012.)

## Reconciliation with 0011 / 0012 (read before sequencing)

| Backlog item | Already covered by | 0013's role |
|---|---|---|
| Cross-meeting voiceprints (auto-ID) | **`specs/0011`** (embedding extraction, `diarization/identity.rs` cosine matcher, suggestion UI) | **Sequence it first** as Wave 1; add the *voiceprint-gallery* framing (N samples per person, confidence tiers, "forget this person") and the **biometric ADR** that 0011 flagged but didn't fully own. |
| Persistent People & roles | **`specs/0012`** (`people` table, role-weighted summaries) | **Sequence into Wave 1** (the durable identity the rest hangs off). 0013 promotes `people` from "diarization follow-on" to "shared foundation" and notes action-items/topics/analytics also consume `people`. |
| Notes-only meetings | partial — `specs/0003` notes flow exists; ROADMAP Phase 2 | **0013 specs the standalone-content primitive** (meeting `origin`/type) that makes notes-only a small delta, not a new flow. |
| Topic/category tags, action items, analytics, pre-call prep, Google Cal, Zoom links, Zoom-as-speaker | not yet specced | **New, sequenced by 0013** with shared-table design + ADR gates. |

**Net:** 0013 **supersedes nothing**; it **extends** 0011 (gallery + biometric ADR) and 0012 (People as
shared foundation, not just a diarization feature), and **sequences** the seven unspecced items behind
them. 0011 and 0012 should ship their identity machinery as Wave 1 of this program.

## Approach

Build the **foundation primitives once**, then layer the **high-value upgrades**, then the
**consumers** that read across everything. Each item lands as its own spec, but they share these
tables and the provider seam. Lead with identity because the dependency graph fans out from it.

## Design — shared data-model sketch

Forward-only migrations under `frontend/src-tauri/migrations/` (CLAUDE.md). **Sketch, not final DDL.**

### Primitive 1 — People + voiceprint gallery (Wave 1; from 0011/0012, extended)
`people` (per `specs/0012`): `id`, `email UNIQUE`, `display_name`, `role`, `notes`, timestamps — the
durable identity. **Extension for the gallery:** rather than one `embedding BLOB` on `people`, allow
**N voiceprint samples per person** so the gallery improves over time (the flywheel):

```
people            (id, email UNIQUE, display_name, role, notes, created_at, updated_at)   -- 0012
voiceprints       (id, person_id → people.id, embedding BLOB, embedding_model, source_meeting_id,
                   sample_quality REAL, created_at)                                       -- 0013 ext
speakers.person_id → people.id   (per-meeting occurrence links to the durable person)     -- 0012
```
- **Identity decoupled from voiceprint:** the person is anchored by `email` (from calendar attendee
  mapping; `speakers.email` already exists). Voiceprints are *matching signal* rows attached to a
  person — keep best-N or a maintained centroid (Wave-1 spec decides; default: keep best-N + compute
  centroid on read).
- **Enroll on confirmation:** when the user maps a diarized cluster → person (calendar pick-list or an
  0011 suggestion), insert a `voiceprints` row from that cluster's embedding. **"You" is free** — the
  mic channel is always the device owner, so the self voiceprint is the highest-quality, zero-effort
  enrollment (with the owner-consent caveat 0011 raised; default: enroll self, opt-out available).
- **Identify next time:** Wave-1 matcher (`diarization/identity.rs`, 0011) computes each cluster's
  embedding and cosine-matches against the gallery centroids. **Confidence tiers, not binary:**
  high → auto-label; medium → suggest-and-confirm ("Is this Priya?"); low → `Speaker N` + manual.
  False-accept is worse than asking — bias conservative (margin test + threshold, per 0011).
- **Controls (biometric ADR requirement):** "forget this person" (delete person + cascade
  `voiceprints`) and "clear all voiceprints"; explicit opt-in for storing **others'** voiceprints.

### Primitive 2 — Standalone content: meeting origin + topics + action items (Wave 2–3)
```
meetings.origin TEXT DEFAULT 'recorded'   -- 'recorded' | 'notes_only' | 'imported'  (Wave 2)
meetings.calendar_event_id TEXT           -- link back to the calendar event (pre-call prep)  (0008 deferred → now)

topics            (id, name, kind, notes_markdown, notes_json, created_at, updated_at)   -- Wave 3
                  -- `notes` columns mirror meeting_notes so a topic carries its OWN notes
meeting_topics    (meeting_id → meetings.id, topic_id → topics.id, PRIMARY KEY(meeting_id, topic_id))
topics.rollup_summary TEXT, rollup_updated_at TEXT  -- cached cross-meeting roll-up

action_items      (id, title, owner_person_id → people.id NULLABLE,   -- NULL owner = me / unassigned
                   is_mine INTEGER, due_date TEXT, status TEXT,        -- 'open'|'done'
                   source_meeting_id → meetings.id NULLABLE,           -- NULL = standalone
                   created_at, updated_at)                              -- Wave 3
```
- **Notes-only meetings (Wave 2)** are a `meetings.origin='notes_only'` row with a `meeting_notes` row
  and **no** `transcripts`/audio. The summary path (`summary/processor.rs`) already has a notes-grounding
  branch (`specs/0003`); notes-only just means "no transcript → ground on notes only." Minimal new code.
- **Topics (Wave 3)** reuse the `meeting_notes` shape for *topic-level* notes (item #2c), use a
  many-to-many join (a meeting can be both "team feedback" and "key decisions"), and cache a
  cross-meeting roll-up (Primitive 3) in `topics.rollup_summary`.
- **Action items (Wave 3)** key their owner to `people` (Primitive 1) and link back to a source meeting
  (nullable → standalone to-dos). Auto-extraction reuses the summary pipeline; the task hub is a query
  over this table.

### Primitive 3 — Cross-meeting aggregation (Wave 3–4, no schema of its own)
A reusable "summarize a *set* of meetings" function: gather the transcripts/notes/summaries for a
meeting set (by topic, by person, by recurring-series), concatenate/chunk via the existing
`summary/` chunking, and fill a roll-up prompt through `llm_client.rs`. Powers topic roll-ups (#2b),
pre-call prep (#7), and "all feedback on a person" (#2 + identity). **No new provider stack** — a new
prompt + a multi-meeting gather, behind one internal API used by several features.

### Primitive 4 — Pluggable providers (Wave 2 calendar; Wave 5 Zoom-as-speaker)
- **Calendar:** extend the existing `CalendarSource` seam (`calendar/mod.rs`) with a `google.rs` impl
  alongside `eventkit.rs`; user-chosen, mirroring the LLM-provider selector. EventKit stays the
  zero-config default. Wins: organizer/RSVP, Workspace Directory names/photos/titles → feeds People +
  pre-call prep; cleaner recurring-series + conferencing data. Cost: first sanctioned non-LLM outbound
  (OAuth + API, **metadata only**), Keychain refresh tokens, Google OAuth verification. **ADR-gated.**
- **Speaker-labeling:** a `SpeakerLabelSource` seam where **diarization is the universal default** and
  **Zoom transcript** is an optional, higher-fidelity source when a Zoom cloud recording is accessible.
  Diarization stays the fallback for Meet/Teams/in-person/no-access. **ADR-gated** (cloud content +
  reconciliation). Lowest priority per Brian.

## Prioritized, dependency-ordered enhancement list (the program)

Effort: S ≈ ≤2 days · M ≈ 3–6 days · L ≈ 1–2+ weeks. "ADR?" = needs an ADR before build.

### Wave 1 — Foundation: persistent identity + voiceprint auto-ID (the keystone) — *do first*
Nearly everything below depends on a stable `people` entity. Lead here.

| # | Item | Impact | Effort | Depends on | ADR? |
|---|---|---|---|---|---|
| 1a | **Cross-meeting embeddings + matcher** (`specs/0011`, Wave-1 of this program) | Stops every meeting starting from zero; enables auto-suggest by voice | M | 0011 accuracy gate; existing CAM++ embeddings | — (covered by ADR-0006) |
| 1b | **People entity + roles** (`specs/0012`) | The durable person every later item resolves through | M | 1a (embeddings), `speakers.email` | — |
| 1c | **Voiceprint gallery + confidence tiers + controls** (0013 extension of 0011) | Flywheel: each confirm improves auto-ID; "forget person" makes biometric storage shippable | M | 1a, 1b | **YES — biometric storage/consent** |

### Wave 2 — Standalone content + the high-value calendar upgrade
| # | Item | Impact | Effort | Depends on | ADR? |
|---|---|---|---|---|---|
| 2a | **Notes-only meetings** (ROADMAP P2) | Capture in-person/phone/catch-up meetings; notepad+summary without audio | S | `meetings.origin`; `specs/0003` notes-grounding | — |
| 2b | **Pluggable Google Calendar (opt-in)** — Brian "big upgrade" | Richer participant data (organizer/RSVP/Directory names+titles) → feeds People + pre-call prep; no Calendar.app dependency | L | `CalendarSource` seam (0008); Keychain | **YES — first non-LLM cloud outbound** |
| 2c | **Per-meeting Zoom recording links + passcode** | Keep the original Zoom video/passcode on a meeting for reference | S | `meetings` columns; Keychain (passcode); 0008 link extraction | maybe (small; fold into Zoom ADR) |

### Wave 3 — Organization: topics, action items, the aggregation engine
| # | Item | Impact | Effort | Depends on | ADR? |
|---|---|---|---|---|---|
| 3a | **Cross-meeting aggregation engine** (Primitive 3) | Reusable "summarize across meetings" — unblocks topic roll-ups, pre-call prep, per-person roll-ups | M | `summary/` chunking; `llm_client.rs` | — |
| 3b | **Category/topic tags + topic notes + topic roll-up** (ROADMAP P4) | Organize/filter by topic; "all feedback on a person", "all decisions"; notes on the topic itself | M | 3a, 1b (per-person roll-up), FTS (P4) | — |
| 3c | **Action items / follow-ups + task hub** (ROADMAP P4) | Capture to-dos (auto from summary + manual/live), owner+due+source, central hub | M | 1b (owners), `specs/0003` (auto-extract) | — |

### Wave 4 — Consumers: analytics + pre-call prep (read across everything)
| # | Item | Impact | Effort | Depends on | ADR? |
|---|---|---|---|---|---|
| 4a | **Meeting analytics** (ROADMAP P4) | Meetings/day, time-in-meetings, my talk-time share, heat-map, most-frequent collaborators, consolidation suggestions | M | 1b (collaborators), diarization talk-time (0010/0011), calendar (0008) | — |
| 4b | **Pre-call prep** (ROADMAP P5) | Before a (recurring) meeting: last time's decisions, attendees' open action items, what *I* owe | M | 3a (roll-up), 3c (action items), 1b, `calendar_event_id` recurring-series | — |

### Wave 5 — Optional / complex (explicitly lowest priority — Brian)
| # | Item | Impact | Effort | Depends on | ADR? |
|---|---|---|---|---|---|
| 5a | **Zoom as a speaker-label source** (ROADMAP P3) | Real names from Zoom's attributed transcript over diarization clusters when a cloud recording is accessible | L | `SpeakerLabelSource` seam; Zoom OAuth; transcript reconciliation; 2c (same Zoom app) | **YES — Zoom cloud content + reconciliation** |

**Dependency spine:** `1a → 1b → 1c` underpins `3b/3c/4a/4b`. `3a` underpins `3b/4b`. `2b` enriches
`1b`/`4b` (better participant data) but is *not* a hard blocker (EventKit suffices). `5a` is isolated.

## ADR callout list (record before building the gated item)

1. **Biometric voiceprint storage & consent** (gates **1c**) — storing voiceprints of *other people* is
   legally sensitive (BIPA / GDPR special-category). Document: local-only-as-moat, explicit opt-in,
   "forget this person" + "clear all voiceprints", self-enrollment-by-default rationale, the
   English/VoxCeleb model caveat, and the confidence-tier (no-silent-false-accept) posture. 0011 flagged
   this; 0013 makes it a hard gate on the gallery.
2. **First sanctioned non-LLM cloud outbound — Google Calendar** (gates **2b**) — the privacy-posture
   trade-off: OAuth + API egress of *schedule metadata* (not meeting content), Keychain refresh-token
   storage, Google's sensitive-scope verification, EventKit-stays-default. (Extends/aligns with 0008's
   EventKit-over-Google ADR.)
3. **Zoom cloud data & transcript reconciliation** (gates **5a**, and the passcode/links in **2c**) —
   meeting *content* round-tripping through Zoom's cloud, `cloud_recording:read` OAuth scope, Keychain
   secret storage, and the clock-alignment/reconciliation of Zoom's transcript with our Whisper
   timeline. Lowest-priority item but highest privacy sensitivity → explicit ADR.

(ADR numbering continues from `docs/decisions/ADR-0006`; assign at spec time.)

## Tasks

This is a program spec; its "tasks" are to graduate each item to a numbered spec in dependency order
and write the gating ADRs first. Owner agents named per item.

1. [ ] **spec-architect** — write the **biometric ADR** (gates Wave 1c); confirm 0011/0012 adopt the
   voiceprint-gallery + confidence-tier framing (amend 0011 to keep best-N samples vs. a single BLOB).
2. [ ] **audio-engineer + rust-core-engineer** — ship **Wave 1** (0011 then 0012, then the 1c gallery
   extension: `voiceprints` table, gallery centroid match, confidence tiers, forget-person controls).
3. [ ] **rust-core-engineer + frontend-engineer** — spec + build **Wave 2a** (notes-only:
   `meetings.origin`, notes-only create flow, notes-grounded summary) and **2c** (Zoom links/passcode).
4. [ ] **spec-architect** — write the **Google-Calendar cloud-outbound ADR** (gates 2b); then
   **rust-core-engineer + frontend-engineer** spec + build **Wave 2b** (`calendar/google.rs` behind the
   `CalendarSource` seam, OAuth, Keychain tokens, provider selector).
5. [ ] **llm-pipeline-engineer + rust-core-engineer** — **Wave 3a** aggregation engine (multi-meeting
   gather + roll-up prompt via `summary/` + `llm_client.rs`); then **3b** topics + **3c** action items
   (with **frontend-engineer** for the topic browser + task hub).
6. [ ] **rust-core-engineer + frontend-engineer** — **Wave 4** analytics + pre-call prep
   (add `meetings.calendar_event_id`, recurring-series detection, roll-up consumption).
7. [ ] **spec-architect** — write the **Zoom cloud/reconciliation ADR**, then spec **Wave 5a** only if
   prioritized later (`SpeakerLabelSource` seam, Zoom OAuth, transcript reconciliation).

## Acceptance criteria

This spec is "done" when reviewed; each *wave item* carries its own testable criteria in its own spec,
tied to the Definition of Done in `/CLAUDE.md` (`cargo check`/`clippy` clean in `frontend/src-tauri`;
`pnpm lint` clean in `frontend`; app launches via `./clean_run.sh`; record→transcript→summary smoke
unchanged). For 0013 itself:

- The nine backlog items are mapped to a wave with impact/effort/deps/ADR, in dependency order, leading
  with the identity keystone (Wave 1).
- The shared-table sketch shows each item reusing People / Topics / Action-items / standalone-content
  rather than parallel structures, grounded in actual columns (`speakers.embedding/email`,
  `meeting_notes`, `meetings.folder_path`, the `CalendarSource` seam).
- 0011/0012 are reconciled explicitly (supersede/extend/sequence) with no re-specification.
- The three ADR gates (biometric, Google-Calendar outbound, Zoom cloud) are named with what each must
  decide and which item it blocks.
- Brian's priorities are respected: Google Calendar = high (Wave 2), voiceprints = high keystone
  (Wave 1), Zoom-as-speaker-source = lowest (Wave 5).

## Risks / open questions

- **Gallery storage shape (1c):** best-N samples vs. a maintained centroid vs. both. *Lean:* keep best-N
  `voiceprints` rows + compute centroid on read (simplest, robust to drift); revisit if match quality
  needs it. This is an amendment to 0011's single-`embedding`-BLOB design — confirm with audio-engineer.
- **Confidence-tier thresholds:** τ_high/τ_medium are CAM++/cosine-specific and must be tuned on 0011's
  benchmark; false-accept (silent wrong label) is the worst outcome → start conservative, prefer
  suggest-and-confirm over auto-label until measured.
- **Voice drift / short utterances / cross-mic / cross-lingual:** known CAM++ limits (English/VoxCeleb
  training). The gallery (multiple samples) mitigates drift; very short utterances stay `Speaker N`.
- **Biometric/legal posture (1c):** the genuine privacy moat is also the legal-sensitivity. The ADR +
  forget/clear controls + opt-in are prerequisites, not polish.
- **Google OAuth verification (2b):** the sensitive calendar scope needs Google's review before public
  distribution — a process/timeline risk independent of code. EventKit-default de-risks it (the feature
  ships value to everyone without Google).
- **Notes-only vs. recorded summary parity (2a):** confirm `summary/processor.rs` cleanly degrades to
  notes-only grounding with no transcript (it has a notes branch per `specs/0003`, but verify the
  no-transcript path doesn't assume segments exist).
- **Topic/person roll-up cost & freshness (3a/3b):** roll-ups across many meetings are LLM-token-heavy;
  cache (`topics.rollup_summary`) and recompute on demand / on new tagged meeting, not eagerly.
- **Zoom transcript reconciliation (5a):** clock alignment between Zoom's cloud transcript and our
  Whisper timeline is unsolved and the reason this is L/low-priority; diarization remaining the fallback
  bounds the risk.
- **Item splitting:** each wave item still needs its own `/spec` before build; 0013 sequences, it does
  not replace per-feature design.

## Verification

- **Review gate:** this spec is verified by review — the mapping, sketch, reconciliation, and ADR list
  are complete and consistent with 0008/0011/0012 and the actual schema.
- **Per-item:** each wave item proves out via its own spec's Verification + `/check`
  (cargo check/clippy, pnpm lint, `./clean_run.sh`, record→transcript→summary smoke). The privacy-gated
  items additionally verify **zero unexpected egress** via packet capture (per 0008/0011 precedent):
  voiceprints never leave the machine (1c); Google Calendar egress is *metadata only* to Google (2b);
  Zoom egress is the documented `cloud_recording` scope only (5a).

## Sources

- ROADMAP backlog items (Phases 2–5): `ROADMAP.md`.
- Identity foundation specced: `specs/0011-diarization-p3-live-and-cross-meeting-identity.md`
  (embeddings + `diarization/identity.rs` matcher), `specs/0012-people-and-role-weighted-summaries.md`
  (`people` table, roles).
- Calendar/Zoom seam + EventKit-over-Google rationale: `specs/0008-calendar-zoom-integration.md`;
  `frontend/src-tauri/src/calendar/{mod,eventkit,zoom_link,day_agenda,commands}.rs`.
- Notes + summary pipeline: `specs/0003-note-enhancement-pipeline.md`;
  `frontend/src-tauri/src/summary/{processor,service,llm_client}.rs` + `summary/templates/`.
- Schema grounding (this repo): `migrations/20250916100000_initial_schema.sql` (`meetings`,
  `transcripts`), `…_add_meeting_notes.sql`, `…_add_audio_sync_fields.sql` (`folder_path`),
  `…_add_speakers_table.sql` (reserved `embedding`), `…_add_speaker_email.sql`.
- Diarization engine + embeddings: `docs/decisions/ADR-0005-diarization-engine.md`,
  `docs/decisions/ADR-0006-live-diarization-approach.md`;
  `frontend/src-tauri/src/diarization/{embedding,identity}.rs` (per 0011).
