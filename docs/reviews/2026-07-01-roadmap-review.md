# Roadmap review — 2026-07-01

- **Scope:** forward-looking critique of `ROADMAP.md` Phases 4–5 and the `specs/0013` program,
  grounded in what shipped through v1.3.0 and in three rounds of real-meeting feedback
  (`specs/0019`, `specs/0024`, `specs/0029`).
- **Requested by:** `specs/0030` WS6. This document recommends; it does not edit `ROADMAP.md`
  (checkbox true-up is running in parallel under 0030 WS5).

## Summary

The roadmap's *content* has aged well — almost every item on it still deserves to exist — but its
*shape* no longer matches how Vinyl actually develops. Three consecutive releases (1.1, 1.2, 1.3)
were driven not by the phase list but by feedback batches from daily dogfooding, and that feedback
is a far better prioritization signal than the Phase 4 → Phase 5 ordering. The signal says three
things clearly. First, **the user reaches for search and templates and keeps finding them
half-built** — the search pill was reported dead in both the 1.0 and 1.2 rounds, and templates were
asked for in 1.0 and then again mid-recording in 1.2. Second, **calendar/participant fidelity is
the single most persistent pain across all three rounds**, and its remaining big item
(distribution-list expansion, `specs/0027`) is explicitly blocked on the Google Calendar
integration sitting unspecced in Phase 5. Third, **nobody has asked for topics, analytics,
cross-meeting analysis, or pre-call prep even once** — the Phase 4 items that look biggest on
paper have zero demonstrated pull.

Meanwhile the roadmap is materially stale: Phase 3 is essentially finished (P2 shipped in 0.4.0,
live diarization in 0.5.0, cross-meeting identity + People in 1.0), notes-only meetings shipped in
1.0, signing/notarization shipped in 1.1 (ADR-0008), and 0029 quietly shipped the foundation of
the templates spec (`meetings.template_id` + per-meeting picker). Several 0013 assumptions
("no `calendar_event_id` column", "notes-only needs a new primitive") are now satisfied.

**Recommendation in one line:** ship the two half-built features the user keeps bumping into
(FTS5 search split out of `specs/0021`; finish `specs/0020` templates), write the Google Calendar
ADR + provider spec next because it unblocks a known pain (`specs/0027`) and feeds everything in
Wave 4, and make **action items** — not topics or analytics — the first genuinely new Phase 4
feature. Defer topic tags, analytics, and pre-call prep behind the aggregation engine; keep
Zoom-as-speaker-source parked at Wave 5.

---

## 1. What shipped vs. what the roadmap thinks (staleness)

The parallel 0030 WS5 pass owns the checkbox edits; this section is the input to it, plus the
places where staleness distorts *planning* rather than just bookkeeping.

**Phase 2 — done in practice.** Notes-only meetings shipped in 1.0 (`specs/0015`,
`meetings.origin`), and the notes-grounded summary fallback was fixed in the same release
("No transcripts available" bug, `specs/0016` fix). The remaining unchecked line ("revisit
notes-grounding prompt strength") is a tuning task, not a phase.

**Phase 3 — done except the Wave-5 Zoom item.** P2 (rename/merge + calendar pick-list +
speaker-attributed summaries) shipped in 0.4.0; live diarization shipped in 0.5.0 (`specs/0011`,
ADR-0006); cross-meeting identity, the People directory, the voiceprint gallery + consent
controls (ADR-0007), role-weighted summaries, participants, and owner identity all shipped in 1.0
(`specs/0016`–`0018`, `specs/0012`). Notably, the **"auto-name the unambiguous 1:1 speaker"**
line is mostly obsoleted: 0016's conservative auto-label already fires when a high-confidence
voice match and a calendar-attendee email agree, and 0029 WS3.4's `transcripts.channel` tag
(`migrations/20260703000002`) makes "You" attribution survive re-runs. What's left of that
roadmap line is a `Fixed(1)` short-circuit micro-optimization in
`diarization/sherpa.rs` — a backlog entry, not a roadmap feature. Recommend demoting it to
`specs/BACKLOG.md` so Phase 3 can be closed.

**Phase 4 — one assumption invalidated.** "Replace naive `LIKE`" is still accurate
(`database/repositories/transcript.rs:291`), but the UI half changed: 0029 WS6.1 made the search
pill a real button that opens the ⌘K palette — which still searches **titles only, client-side**
(`components/CommandPalette/index.tsx`). So the visible gap moved from "dead input" to "search
that can't find what someone said," which is arguably worse because it now *looks* functional.
Also `meetings.folder_path` (the "tags/folders" hook) remains unused — but 0013 correctly routes
topics through a join table instead; the roadmap's "folder_path exists but is unused" framing
should be dropped so nobody builds on that column.

**Phase 5 — two items already shipped.** "Real signing identity" landed in 1.1 (ADR-0008,
notarized DMGs) — that bullet should split so icons/exports/onboarding don't hide behind a
done item. And 0029 WS4.3 shipped the per-meeting template persistence + live picker that
`specs/0020` still lists as its Tasks 1/3 — 0020 needs re-grounding before anyone implements it
(see §3).

**New capabilities the plan hasn't absorbed.** Record-only mode + deferred transcription and
audio retention (0029 WS7) change two planning assumptions: (a) a meeting may have **no
transcript for hours or days** after it ends, and (b) transcripts can be **rewritten later** by
"Transcribe now". Anything that indexes or extracts from transcripts (FTS5 triggers, action-item
extraction, topic suggestions) must be event-driven off transcript writes, not meeting-end. And
the `transcripts.channel` column makes **owner talk-time computable without diarization** — the
"meeting analytics" item just got meaningfully cheaper for its most personal metric.

## 2. Is Phase 4 → Phase 5 still the right order? No — interleave by pull.

Three feedback rounds give a clean demand ranking, and it cuts across the phase boundary:

| Signal strength | Item | Evidence |
|---|---|---|
| Asked twice | **Real search** | 0019 note 11 (dead pill), 0029 WS6.1 (dead pill again) — the user physically reaches for it |
| Asked twice | **Templates** | 0019 note 10 (visibility/editing/auto-select), 0029 WS4.3 (wanted to pick one *during* a meeting) |
| Every round | **Calendar/participant fidelity** | 0019 WS6.3, 0024 WS2.1/WS2.2, 0029 WS2.1 — and the unresolved piece (`specs/0027` DL expansion) is blocked on Google Calendar (Phase 5) |
| Once | Custom dictionary | 0019 note 18 ("Vinyl" → "vinal") — `specs/0022`, small and real |
| Once | Ask-AI across meetings | 0019 note 12 |
| **Never** | Topics, analytics, cross-meeting analysis, pre-call prep, Zoom links, Zoom-as-speaker | zero mentions in ~55 feedback items |

Two conclusions. First, the next slice is not a Phase 4 headline feature — it's **finishing the
two features the UI already advertises** (search, templates). Both are S/M efforts with drafted
specs. Shipping features the user has asked for twice beats starting features asked for never.

Second, **Google Calendar should jump the queue from Phase 5 to "next after those."** 0013 already
rated it "big upgrade / high," but since then its case strengthened: `specs/0027` (distribution
lists render as one fake person — a real roster-corruption pain from 1.1 testing) was explicitly
deferred *until Google Calendar exists*, and every identity feature that shipped in 1.0 (People,
participants, owner emails) gets better data from it. It is also the pattern-setter for all future
cloud integrations, which is why its ADR matters more than its code (§3).

Where does that leave the big Phase 4 items? **Action items** is the one worth pulling forward.
It's the largest remaining gap against the granola.ai bar (Granola extracts action items by
default), and — unlike topics/analytics — its foundation is now fully shipped: owners resolve
through `people` (0016), meetings carry participants (0017) and `calendar_event_id` (0015), and
extraction rides the existing summary pipeline (`summary/processor.rs` templates already have
action-item sections). Topic tags, the aggregation engine, analytics, and pre-call prep should
stay sequenced per 0013 Waves 3–4 — they're coherent, but with zero pull they should not displace
the items above. One honest caveat: dogfooding is n=1 and biased toward in-meeting friction
(you *notice* a scroll jump; you don't *notice* the absence of a task hub until the habit forms).
Action items and pre-call prep are "absence" features — worth building one of them (action items)
ahead of demonstrated pull, but only one.

**A conditional structural item.** All three rounds contained the same *class* of bug: meeting
identity/lifecycle races caused by session state living in scattered frontend globals
(`currentMeeting` vs `activeRecordingMeetingId`, the pending-join stash, `window.handleRecordingStop`)
— 0019 WS6.1/6.7, 0024 WS1.1/2.1, 0029 WS2.1/5.1. Each fix was correct and each round found a new
race. 0029 claims the last of them; if the 1.3 real-meeting smoke round surfaces **another**
meeting-identity race, stop patching and spec a backend-owned recording-session state machine
(single source of truth in Rust, frontend subscribes) *before* starting any new Phase 4 feature.
If 1.3 holds clean, skip it — don't refactor a working core on principle.

## 3. Spec-quality gaps and ADR gates

**`specs/0020` (templates) — stale, needs a re-grounding pass before build.** 0029 WS4.3 shipped
its data model (`migrations/20260703000001_add_meeting_template.sql`,
`api_get/set_meeting_template`, record-screen picker, summary consumes the persisted choice).
What remains is: editable/addable templates (`api_save_template` + settings UI), registering the
four orphaned on-disk JSONs (`summary/templates/loader.rs` scans them but `defaults.rs` doesn't
register them), the active-template label, regenerate-on-change, and auto-select-by-title. Revise
0020 in place (strike Tasks 1/3, re-anchor line references) rather than writing a new spec. Its
open question "how aggressively should regenerate-on-change fire" should be decided in the spec:
confirm-before-regenerate, because summaries cost minutes on local Ollama.

**`specs/0021` (search + Ask-AI) — over-scoped; split it.** See §4.

**Google Calendar — no spec, and the ADR is the hard part.** ROADMAP and 0013 both flag it
ADR-gated (first sanctioned non-LLM outbound) but nobody has drafted either document. The ADR
(next free number: **ADR-0010**; ADR-0009 is being taken by the 0030 Keychain work) must decide:
OAuth flow shape on a desktop app (loopback redirect + PKCE, no client secret), token storage
(reuse the Keychain layer 0030 WS1 is building right now — a genuine sequencing win), the
metadata-only egress boundary and how it's verified (packet-capture check per 0013's precedent),
scope minimization (`calendar.readonly` + whether Directory/People lookup is v1 or v2), and the
Google OAuth verification timeline risk for distribution. The provider spec itself is
comparatively mechanical: a `google.rs` impl behind the existing `CalendarSource` seam
(`src/calendar/mod.rs`), provider selector mirroring the LLM-provider pattern.

**Zoom OAuth — one ADR should gate both Zoom items.** The per-meeting recording-links item
(Phase 5 / 0013 2c) and Zoom-as-speaker-source (Wave 5) share the OAuth app and the privacy
question. Write the ADR only when 2c is picked up; don't let the small links feature sneak in a
Zoom cloud dependency without the decision on record. The links item minus auto-population
(manual URL + Keychain passcode field) needs no ADR at all — that's the right v1 cut.

**Unspecced Phase 4 items lack acceptance-criteria-grade definition.** "Cross-meeting reference
& analysis" is a direction, not a feature — as written it's unimplementable and, once Ask-AI and
topic roll-ups exist, redundant; recommend deleting the line. "Meeting analytics" needs its
metrics enumerated with their data sources (meetings/day and duration are trivial;
talk-time-share is now cheap via `transcripts.channel`; "consolidatable meetings" needs
participant-set similarity — cut it from v1). "Action items" needs the dedupe/regeneration
semantics defined up front (§4). None of these need ADRs — they're local-only — but topics and
action items both add tables, so their specs must include the forward-only migration design per
0013's sketch.

**Ask-AI has an unacknowledged privacy posture.** A cross-meeting Ask-AI query sends *many
meetings' content* to the configured provider in one shot — the largest single egress the app can
make, much bigger than a one-meeting summary. 0021 mentions the provider invariant but the spec
that ships it should surface scope in the UI ("this will send N meetings to {provider}") and
consider defaulting Ask-AI to the local provider even when summaries use a cloud one. Probably a
spec-level design requirement rather than a full ADR, but decide deliberately.

## 4. Right-sizing

**Split: `specs/0021`.** Search (FTS5 + surfacing) and Ask-AI are different features with
different risk profiles bolted together because they arrived in one feedback note. Search is
S/M, zero egress, immediately verifiable, and twice-requested. Ask-AI is M/L, needs the
scope-resolution + chunk/combine machinery that 0013's Wave 3a "aggregation engine" *also* needs
— building Ask-AI inside 0021 would create a second aggregation path that 3a then duplicates.
Split: ship search now; fold Ask-AI into the aggregation-engine spec so the multi-meeting
gather/chunk/answer core is built once and Ask-AI, topic roll-ups, and pre-call prep are three
prompts over it.

**Split: the Phase 4 topics item.** As written it's three features: (a) manual tags + filtering,
(b) AI roll-up summaries per topic, (c) notes attached to a topic. (a) is a small standalone spec
(tables `topics` + `meeting_topics` per 0013, a tag picker, a filter in All-meetings/⌘K); (b)
depends on the aggregation engine; (c) reuses the `meeting_notes` shape and can ride (a). Shipping
(a) alone already delivers organization value and generates the tagging corpus that makes (b)
worth building.

**Under-scoped: FTS5 has hidden interactions with 1.3 features.** Three things 0021 doesn't yet
account for: (1) **record-only mode** means transcripts arrive long after the meeting row, and
"Transcribe now" *rewrites* transcript rows — the index must be maintained by
insert/update/delete triggers (external-content FTS5 table), and the backfill migration must
tolerate meetings with zero transcripts; (2) **tokenization** — 0028 fixed a crash on non-ASCII
transcript search, which is direct evidence the corpus and queries contain accents/CJK; specify
`unicode61 remove_diacritics 2` and add non-ASCII fixtures to the acceptance criteria; (3)
**verify FTS5 is compiled in** — sqlx 0.8's bundled SQLite (via libsqlite3-sys) normally enables
FTS5, but the spec's acceptance should include a startup-time `pragma compile_options` assertion
or a migration that fails loudly, not a silent no-op.

**Under-scoped: action items' regeneration semantics.** The roadmap text ("auto-extracted as part
of the summary, and/or recognized live") hides the two hard problems. Live detection is a
separate, much harder feature — cut it from v1 entirely. And extraction-at-summary-time meets
reality the moment a summary is **regenerated** (which happens constantly — template change,
notes edit): naive re-extraction duplicates every task or, worse, wipes completion state. The
spec must define stable identity for extracted items (e.g. re-extraction proposes a diff against
existing items rather than re-inserting; user-completed/edited items are never touched). This is
the action-item analog of the "manual renames survive diarization re-runs" lesson from 0029
WS3.2 — the codebase has already paid for this lesson once.

**Over-scoped as roadmap lines, fine as 0013 waves:** analytics ("consolidation suggestions" and
the heat map are v2; v1 = meetings/day, hours-in-meetings, my talk-time, top collaborators —
four queries and one page) and pre-call prep (its v1 is "show last occurrence's summary + my open
action items for this series," which is a join once action items and `calendar_event_id`
recurring-series detection exist — not the full context engine the roadmap paragraph implies).

## 5. Recommended next specs

In order. The first two are small and clear the "advertised but half-built" debt; the third is
the strategic one. (Alongside these: revise `specs/0020` in place per §3 — it's a true-up, not a
new spec.)

### 5.1 `specs/0031 — Meeting full-text search (FTS5)` — split from 0021

**Problem.** The search affordance has been reported broken in two of three feedback rounds. As
of 1.3 the pill opens the ⌘K palette, but the palette filters meeting *titles* client-side; the
orphaned backend `api_search_transcripts` (`api/api.rs`, `transcript.rs:291`) still does a naive
`LIKE` and nothing renders it. The user cannot answer "which meeting did we discuss X in" — the
core retrieval promise of a meeting assistant — and every week of dogfooding grows the corpus
that makes `LIKE`-over-everything less viable.

**Approach.** An external-content FTS5 table over `transcripts` (and `summaries` + `meeting_notes`
— notes-only meetings shipped in 1.0, so notes must be searchable) with insert/update/delete
triggers, a backfill migration, and a `unicode61 remove_diacritics 2` tokenizer. Replace
`search_transcripts` with a ranked-snippet query; extend the ⌘K palette to a two-tier result list
(title matches, then content matches with snippets) deep-linking to the meeting and matched
segment. Explicitly handle the 1.3 lifecycle: index updates fire on transcript writes (covers
record-only + "Transcribe now" rewrites), and audio-retention sweeps don't touch the index
(transcripts are kept). **Rough acceptance:** a phrase spoken in a recorded meeting, a phrase in
a notes-only meeting, and an accented/CJK phrase are each findable via ⌘K with a snippet and
deep-link; retention sweep and re-transcription leave the index consistent (cargo test fixture);
palette title-search behavior is unchanged for empty index; startup asserts FTS5 is compiled in.
Owners: rust-core-engineer (migration/query), frontend-engineer (palette). Ask-AI is explicitly
out — it moves to the aggregation-engine spec.

### 5.2 `ADR-0010 + specs/0032 — Google Calendar provider` (0013 Wave 2b)

**Problem.** Calendar fidelity is the most persistent feedback theme, and its biggest unresolved
item — distribution-list invites rendering as one fake participant (`specs/0027`) — is explicitly
deferred until a real Google Calendar connection exists. EventKit also can't provide organizer/
RSVP/Directory data, which caps the quality of People (0016), participants (0017), and everything
in 0013 Wave 4. This is also Vinyl's first sanctioned non-LLM outbound traffic, so the *decision*
(egress boundary, consent, verification) matters more than the code and sets the pattern for
every future integration.

**Approach.** ADR-0010 first: opt-in provider behind the existing `CalendarSource` seam
(`src/calendar/mod.rs`), EventKit stays the zero-config default; loopback-redirect OAuth with
PKCE; refresh token in the Keychain layer shipping in 0030 WS1; `calendar.readonly` scope,
metadata-only egress verified by packet capture; Google verification risk documented with the
"EventKit covers everyone meanwhile" mitigation. Then 0032: `calendar/google.rs`, a provider
selector in Settings mirroring the LLM-provider pattern, attendee mapping into the existing
seeding path (`meeting_participants` + auto-People), and DL expansion where the API returns
member lists — closing 0027. **Rough acceptance:** with Google connected, an event created in
Google Workspace appears in the Day Agenda with full individual attendees (including a
DL-invited event showing members, where the org exposes them); disconnecting deletes tokens and
reverts to EventKit with zero Google egress thereafter (packet-capture check); EventKit-only
users see no behavior change; tokens never appear in SQLite or logs. Owners: spec-architect
(ADR), rust-core-engineer (OAuth/provider), frontend-engineer (settings/selector).

### 5.3 `specs/0033 — Action items v1: extraction + task hub` (0013 Wave 3c)

**Problem.** The clearest remaining feature gap against the granola.ai bar: meetings produce
commitments, and Vinyl currently drops them on the floor once the summary scrolls by. Every
prerequisite shipped in 1.0 — owners resolve through `people`, meetings carry participants and
`calendar_event_id`, and summary templates already elicit action-item sections — so this is now
an integration of existing parts, not a new subsystem. It is also the prerequisite that makes
pre-call prep (0013 Wave 4b) worth building later ("what do I owe this meeting?").

**Approach.** An `action_items` table per 0013's sketch (`owner_person_id → people`, `is_mine`,
`due_date`, `status`, nullable `source_meeting_id`); extraction as a structured second pass on
the existing summary output (llm-pipeline-engineer: a JSON-emitting prompt over the generated
summary + notes, owner names resolved against the participant roster → `people`), never blocking
summary delivery. **Regeneration-safe by design:** re-extraction computes a diff against existing
items — new items are proposed, user-edited/completed items are never modified or duplicated
(stable content-key matching; the 0029 WS3.2 lesson). UI: an action-items block on the meeting
page (confirm/edit/dismiss extracted items, add manual ones) and a task-hub page (sidebar) with
mine/others/all filters and per-item deep-links to the source meeting. Live-during-meeting
detection is explicitly out of scope. **Rough acceptance:** a recorded meeting whose transcript
contains clear commitments yields extracted items with correct owners from the roster; marking
one done then regenerating the summary neither duplicates nor reopens it; a manual item with no
meeting works; the hub filters by owner and links back to sources; no egress beyond the one
configured LLM call. Owners: llm-pipeline-engineer (extraction), rust-core-engineer (table/IPC),
frontend-engineer (hub + meeting block).

### After these

Aggregation engine + Ask-AI (0013 3a, absorbing 0021's second half), then topic tags (split per
§4), then analytics v1 and pre-call prep. `specs/0022` (dictionary) is a good S-sized gap-filler
whenever a release needs one — the pain is real ("Vinyl" → "vinal") and the design is sound.
Zoom recording-links can ship ADR-free in its manual-entry form; everything Zoom-cloud stays at
Wave 5 behind its ADR.

## Suggested structural changes to ROADMAP.md (for the 0030 WS5 pass)

1. Collapse Phases 0–3 into a linked "Shipped" section (pointers to specs/versions); the
   scroll-past-history format hides the live plan.
2. Add a **Now / Next / Later** section at the top, reviewed at each release, since feedback-batch
   releases are the actual cadence; keep the phase taxonomy below as the long-range map.
3. Move Google Calendar from Phase 5 into the "Next" slice with its ADR gate noted; move the
   remaining templates/search work in from the spec backlog (they're on no phase today —
   `specs/0020`–`0022` are invisible in ROADMAP.md).
4. Delete "Cross-meeting reference & analysis" as a standalone line (subsumed by Ask-AI + topic
   roll-ups); demote "auto-name 1:1 speaker" to `specs/BACKLOG.md` as a `Fixed(1)` optimization.
5. Note under Phase 4 that FTS/extraction features must key off transcript writes, not meeting
   end (record-only mode + deferred transcription shipped in 1.3).

## Open questions for Brian

1. **Action items ahead of demonstrated pull?** §2 argues yes (granola parity, foundations
   ready), but if daily use says otherwise, the aggregation engine + Ask-AI is the alternative
   third spec — Ask-AI *was* asked for once.
2. **Google Calendar timing vs. OAuth-verification friction:** dev/personal use needs no Google
   verification (test-user mode), so the integration can ship for you now and gate public
   distribution later. Acceptable posture for the ADR?
3. **Ask-AI provider default:** when summaries use a cloud provider, should cross-meeting Ask-AI
   still default to local (Ollama) given the larger egress? (Spec-level decision, needs your
   call.)
4. **If the 1.3 smoke round surfaces another meeting-identity race**, do you want the
   recording-session state-machine spec prioritized over 0031–0033? (§2's conditional.)
