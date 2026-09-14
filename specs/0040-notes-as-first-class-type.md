# 0040 — Notes as a First-Class Type

- **Status:** Draft (stub — captured, not yet fully specced). Graduated out of the 1.6
  feedback batch (`specs/0038`), alongside `specs/0039` (diarization). Mirrors how `specs/0019`
  spun off `0020–0022`.
- **Owner agent(s):** frontend-engineer + rust-core-engineer (+ llm-pipeline-engineer for the
  summary/action-item hand-off)
- **Roadmap phase:** Post-1.6 (graduated feedback item)

> **This is a stub.** It records the owner's 1.6 dogfood feedback so the idea isn't lost and
> reserves the spec number. Flesh it out to full `TEMPLATE.md` depth (grounded `file:line`
> anchors, per-workstream Problem/Approach/Files, acceptance criteria) before build — follow the
> `specs/0029` house style. Do this after `specs/0038` lands so the two don't collide on the
> notes surfaces.

## Context / Problem

`specs/0015` (Meetings as first-class objects) shipped **notes-only meetings** — "New note"
creates a no-audio meeting with a notepad and a notes-grounded summary. In real use (1.6
dogfooding) the owner found that modeling a Note *as a meeting* is the wrong abstraction: notes
inherit meeting-only chrome (Prep tab), clutter the meeting/agenda lists, and lack a quick
capture path. The ask is a genuine **Note type distinct from a Meeting**, not a meeting variant.

### Owner feedback (verbatim)

> "Notes feature could be more robust — like a quick ability to record on a 'new note' would be
> nice. Different treatment than meetings? Don't need 'Prep' on notes. Should start with the same
> layout as recording a call — transcript and notes side by side, with a clear 'done' that kicks
> off summarization and action item detection. I don't want to see my Notes on the agenda view of
> my day. Need to show those below in a separate section, and make a visual distinction between
> Meeting notes and a plain Note."

## Goals (to be refined)

- A **Note** is a first-class object distinct from a Meeting (a `kind` discriminator or a
  dedicated origin), visually and behaviorally separate.
- **Quick-record on a Note**: record + transcribe with the same side-by-side transcript+notes
  layout used for a call — a Note can capture audio, it just isn't a scheduled meeting.
- **No Prep tab** on Notes; a clear **"Done"** action that kicks off summary + action-item
  extraction (reuse the `specs/0034` regeneration-safe extraction).
- **Home/agenda separation**: Notes never appear on the Today/agenda timeline; they get their own
  section below, with a clear visual distinction between a Meeting note and a plain Note.

## Non-goals (to be refined)

- Re-architecting the recording pipeline — a Note reuses the existing capture/transcribe path.
- Replacing `meeting_notes` storage — the notepad content model is reused.

## Sketch of the work (pre-grounding — verify before building)

- **Data model:** a `kind`/type discriminator distinguishing `note` vs `meeting` (evaluate
  reusing/renaming `meetings.origin` from `specs/0015` — `notes-only` already exists — vs. a new
  column or table). Whatever is chosen must let all list/agenda queries cheaply exclude Notes.
- **Agenda/Home:** exclude Notes from `api_get_day_agenda` and the Today timeline (`specs/0036`);
  add a separate Notes section on Home; badge/style Meeting-note vs plain-Note.
- **Quick capture:** a "New note" affordance that opens the record-style side-by-side layout with
  the audio path enabled; a "Done" that finalizes → summary + action items.
- **Detail chrome:** suppress the Prep tab for Notes.

## Open questions

- Reuse `meetings.origin` (extend the existing `notes-only` value) or introduce a first-class
  `kind` column / separate table? Migration + all list-query filters hinge on this.
- Does a recorded Note get diarization + the full meeting summary template, or a lighter
  note-specific template?
- How do Notes interact with search (`specs/0033`), action items (`specs/0034`), and the
  aggregation engine (`specs/0035`) — same indexing, or a separate surface?
- Should an existing notes-only meeting be migrated into the new Note type, or only new ones?
