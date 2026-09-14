# 0016 — Cross-meeting voice identity, People entity & voiceprint gallery (Cluster 2 / Wave 1)

- **Status:** Approved — building
- **Owner agent(s):** rust-core-engineer (lead: matcher, DB/IPC, People + voiceprints, delete
  cascades) + audio-engineer (offline embedding persistence + gallery-centroid match) +
  frontend-engineer (suggestion chips, People directory, voiceprint controls) +
  llm-pipeline-engineer (role-weighted summary — **follow-on, not in Wave 1**)
- **Roadmap phase:** Phase 3 — the 2.0 program's **Cluster 2 / Wave 1** keystone
- **Parent program:** `specs/0013-identity-and-organization-program.md` (Wave 1 = 1a→1b→1c)
- **Consolidates:** `specs/0011` Task 11 (cross-meeting matcher), `specs/0012` (People entity +
  role-weighted summaries — role-weighting deferred), and the now-**Accepted** ADR-0007
  (biometric voiceprint storage). This spec is the single **buildable** decomposition of those.
- **Decision record:** `docs/decisions/ADR-0007-biometric-voiceprint-storage.md` (Accepted,
  2026-06-27, incl. the per-person `voiceprint_opt_out` refinement). ADR-0007 is the **hard gate
  on Phase 1c**; Phases 1a/1b are explicitly *not* gated by it (ADR-0007 §"Scope of the gate").

## Context / Problem

After live diarization shipped (`specs/0011` Task 10, v0.5.0), Vinyl labels *who spoke* within a
meeting (`You` / `Speaker 1` / …) and lets you rename/assign them. But identity is still strictly
**per-meeting**: `Speaker 2` this week has no relationship to `Speaker 2` last week, even for the
same person. That breaks the flywheel ADR-0007 calls the keystone — every confirmation should make
the next meeting's auto-labeling better. Three concrete gaps remain:

1. **No cross-meeting matcher.** We extract a per-cluster embedding live, but the **offline** pass
   never persists it, and nothing compares a new meeting's voices against prior identified speakers.
   ("Looks like Priya" does not exist.)
2. **No durable Person.** The same human across ten meetings is ten unrelated `speakers` rows that
   merely share an `email`. There is no app-wide entity to anchor identity, attach a role, or hang a
   voiceprint gallery on — and no UI to map a detected speaker to a *known person* (only to a
   calendar attendee).
3. **No voiceprint gallery.** Cross-meeting voice recognition that *survives* across meetings (the
   regulated, ADR-0007-gated artifact) doesn't exist, nor do the privacy controls (opt-in/opt-out,
   forget, clear) ADR-0007 requires before it can ship.

### Grounding (verified in this repo, 2026-06-27 — state of the art, do **not** re-derive)

The embedding *plumbing* is built; cross-meeting *persistence and matching* are not:

- **Embedding math + extractor EXIST.** `diarization/embedding.rs` has `l2_normalize`,
  `cosine_similarity`, `weighted_mean`, `embedding_to_bytes`/`embedding_from_bytes`, the
  `EMBEDDING_MODEL_ID = "3dspeaker_campplus_sv_en_voxceleb_16k"` constant, and a `ClusterEmbedder`
  (re-embed extractor via `sherpa_rs::speaker_id::EmbeddingExtractor` — sherpa does **not** expose
  cluster centroids, confirmed in the module header). All pure math is unit-tested.
- **Per-cluster extraction WORKS through the trait.** `diarization/mod.rs` defines
  `Diarizer::diarize_with_embeddings(...) -> TurnsWithEmbeddings` (turns + `spk_N → Vec<f32>`);
  `sherpa.rs` overrides it; **`live.rs` already calls it** (`live.rs:412`). The default impl returns
  an empty map, so the **offline** path is byte-unaffected today.
- **The offline pass does NOT persist embeddings.** `diarization/pipeline.rs::persist` calls
  `SpeakersRepository::upsert(pool, meeting_id, key, display_name, is_local)` — **no embedding
  param**. `pipeline.rs` runs `diarizer.diarize_with_progress(...)` (turns only), never
  `diarize_with_embeddings`. The `speakers.embedding BLOB` column exists
  (`migrations/20260624000000_add_speakers_table.sql`) but is **never written**. There is no
  `embedding_dim` / `embedding_model` column.
- **`diarization/identity.rs` does NOT exist.** No cross-meeting matcher.
- **No `people` table, no `voiceprints` table, no `speakers.person_id`.** The stable cross-meeting
  key today is `speakers.email` (`20260625000000_add_speaker_email.sql`), written by
  `assign_to_attendee`.
- **Attendee mapping exists and is the affordance we extend.** `api_get_meeting_attendees`
  (`diarization/commands.rs:275`) returns the linked event's attendees + a 1:1 `suggestion`;
  `api_assign_speaker_to_attendee` writes `display_name` + `email`. `MeetingDetails/SpeakerLegend.tsx`
  (417 LOC) + `hooks/useSpeakers.ts` (202 LOC) render the rename/assign control. There is **no**
  People route in `frontend/src/app/`.
- **Delete cascades are explicit (no FK enforcement).** The app pool connects **without**
  `PRAGMA foreign_keys = ON`, so `delete_meeting_with_transaction` (`repositories/meeting.rs:416`)
  deletes `transcript_chunks`/`summary_processes`/`transcripts`/`meeting_notes`/`speakers`
  **by hand** even though FKs declare `ON DELETE CASCADE`. Any cascade we add (person→voiceprints,
  meeting→voiceprints) **must** be enforced in our own delete transactions the same way.
- **Settings pattern.** `diarization/settings.rs` (`DiarizationSettings`, JSON in the
  identifier-derived data dir, ADR-0004) holds `diarization_enabled` / `live_diarization_enabled` /
  `expected_speaker_count`; `#[serde(default)]` keeps old files valid. New toggles go here.
- **Model path.** `diarization/models.rs::model_paths().embedding` is the on-disk CAM++ model the
  `ClusterEmbedder` loads; the offline pass already has `paths.embedding` in scope.
- **Privacy invariant already holds.** The summary path uses display names only; embeddings are
  never serialized into LLM-bound JSON (`specs/0011`). We extend, not weaken, that.

## Goals

- **1a — Cross-meeting matcher (offline).** Persist a representative L2-normalized embedding per
  *remote* speaker on the offline pass; when a meeting is diarized, **suggest** names by local cosine
  match against prior identified speakers (τ_match + runner-up margin). Suggestions, never silent
  auto-rename. 100% on-device.
- **1b — People entity.** A durable, app-wide `people` row (anchored by email, but allowing
  email-less voice-only/manual people), linked from `speakers.person_id`. A People directory UI, and
  — required — the ability to **associate a known Person with a detected speaker** in a transcript
  (not just a calendar attendee). This works **even when the person has opted out of voiceprints**.
- **1c — Voiceprint gallery + controls (ADR-0007).** A `voiceprints` table (best-N samples per
  person, centroid on read), enroll-on-confirm gated by `global_opt_in && !person.voiceprint_opt_out`,
  owner self-enroll on-by-default-with-opt-out, gallery-centroid matching with conservative
  confidence tiers, and the **required** controls: forget-person (cascade), clear-all, the global
  opt-in toggle, and the per-person opt-out (which deletes existing voiceprints + blocks future
  enrollment **without** deleting the person).
- **Each phase ships independently** and is independently verifiable. 1b depends on 1a's matcher
  existing (to extend it to People); 1c depends on 1b's `people` table.
- **No new outbound traffic; no new dependency; no new model.** Reuses the loaded CAM++ extractor and
  the existing sherpa/identity plumbing.

## Non-goals

- **Role-weighted summaries** (`specs/0012` Tasks 5–6). The `people.role` column lands in 1b so the
  data model is complete, but the summarization weighting + eval are an explicit **follow-on**
  (tracked back to `specs/0012`); Wave 1 does **not** block on it. The summary path stays
  byte-identical.
- **Live cross-meeting suggestions.** Matching runs on the **offline** pass (1a) and attaches to
  `diarization-complete`. Live labels are unchanged.
- **Explicit voice enrollment** ("record 10 s"). The gallery accrues opportunistically from confirmed
  meetings (ADR-0007 §4). No enrollment product.
- **Cross-device / cloud sync of voiceprints or People.** Local DB only (ADR-0007 §1).
- **Merge-people UX, aliases, contact sync, org charts.** Later. A `notes` field exists; merge is out.
- **Embedding-model swap / speakrs.** Behind the `Diarizer` trait already; `embedding_model` tagging
  (1a) makes a future swap safe, but no swap here.

## Approach

**Build in strict dependency order — 1a → 1b → 1c — reusing every existing seam.**

- **1a** turns on what's already wired: call `diarize_with_embeddings` from the offline
  `pipeline.rs` (live already does), persist the per-remote-speaker embedding through a widened
  `SpeakersRepository::upsert`, and add a pure `diarization/identity.rs` cosine matcher that ranks
  the *same* attendee pick-list the UI already shows. No biometric *storage* beyond the
  already-reserved `speakers.embedding` column (ADR-0007 §"Scope of the gate") — so 1a is **not**
  ADR-gated.
- **1b** promotes `email` to a first-class `people` row and adds `speakers.person_id`. Naming a
  speaker (calendar, suggestion, or — new — picking an existing Person) upserts-or-links a person.
  The matcher (1a) is extended to also consider `people`. **Crucially, identity is decoupled from
  voiceprints** (ADR-0007 §2): the "assign to Person" affordance works for any person, opted-out or
  not. No biometric storage in 1b either.
- **1c** is the ADR-0007-gated step: a `voiceprints` gallery keyed to `people`, enroll-on-confirm
  behind the two-layer consent gate, centroid-on-read matching with confidence tiers, and the full
  set of deletion/consent controls. This is the only phase that creates durable biometric data.

Why this over alternatives: a single stored centroid per person was rejected by ADR-0007 §"Alternatives"
(brittle to drift, loses provenance) in favor of best-N + centroid-on-read. A parallel
`meeting_attendees` table was rejected by `specs/0015` — `people` + `speakers.email`/`person_id` is
the durable home. Building the matcher inside `pipeline.rs` rather than a new command keeps it on the
existing `diarization-complete` event the meeting view already listens to.

## Design

### Data model (forward-only migrations under `frontend/src-tauri/migrations/`)

All migrations are additive and idempotent (`IF NOT EXISTS` / `ALTER ADD`); existing rows stay valid.
**No migration enables `PRAGMA foreign_keys` and no delete relies on cascade firing** — cascades are
enforced in delete transactions in Rust (see Grounding).

**Phase 1a — `…_add_speaker_embedding_meta.sql`:**
```sql
-- specs/0016 1a: tag the reserved speakers.embedding so a future embedding-model
-- swap can't silently compare incomparable vectors (cosine across models is junk).
-- Nullable; existing rows (all NULL embedding) are unaffected.
ALTER TABLE speakers ADD COLUMN embedding_dim   INTEGER;  -- f32 count in the BLOB
ALTER TABLE speakers ADD COLUMN embedding_model TEXT;     -- e.g. "3dspeaker_campplus_sv_en_voxceleb_16k"
```

**Phase 1b — `…_add_people_table.sql` + `…_add_speaker_person_id.sql`:**
```sql
-- specs/0016 1b: durable, app-wide person. Identity is decoupled from voiceprints
-- (ADR-0007 §2): a person can exist with NO email and NO stored voice.
CREATE TABLE IF NOT EXISTS people (
    id                 TEXT PRIMARY KEY,            -- "person-<uuid>"
    email              TEXT UNIQUE,                 -- stable key; NULLABLE (voice-only/manual people)
    display_name       TEXT NOT NULL,
    role               TEXT,                        -- free-text/preset (role-weighting is a follow-on)
    notes              TEXT,
    voiceprint_opt_out INTEGER NOT NULL DEFAULT 0,  -- ADR-0007 §2: 1 = never model this person's voice
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);
-- UNIQUE on email is enforced ONLY when email IS NOT NULL — SQLite treats NULLs as
-- distinct, so many email-less people can coexist, which is exactly what we want.
CREATE UNIQUE INDEX IF NOT EXISTS idx_people_email ON people(email) WHERE email IS NOT NULL;
```
```sql
-- specs/0016 1b: link a per-meeting speaker occurrence to the durable person.
-- Nullable. No FK enforcement (pool runs without PRAGMA foreign_keys); the
-- meeting-delete tx already deletes speakers, so no person rows are touched on
-- meeting delete (people outlive meetings, by design).
ALTER TABLE speakers ADD COLUMN person_id TEXT;  -- → people.id, nullable
CREATE INDEX IF NOT EXISTS idx_speakers_person_id ON speakers(person_id) WHERE person_id IS NOT NULL;
```
*Decision — NULL email UNIQUE:* the `WHERE email IS NOT NULL` partial index gives us "unique when
present, unlimited when absent." A person created by mapping a manual name or a voice-only cluster has
no email; setting an email later that collides is rejected at the repository layer with a clear error
(callers fall back to linking the existing person).

**Phase 1c — `…_add_voiceprints_table.sql` (ADR-0007-gated):**
```sql
-- specs/0016 1c (ADR-0007): best-N voice samples per person; centroid computed on
-- READ (not stored) — robust to drift, keeps per-sample provenance for deletion/tuning.
-- person→voiceprints cascade is enforced in the delete transaction (no PRAGMA FKs).
CREATE TABLE IF NOT EXISTS voiceprints (
    id                TEXT PRIMARY KEY,           -- "vp-<uuid>"
    person_id         TEXT NOT NULL,              -- → people.id (cascade in app logic)
    embedding         BLOB NOT NULL,              -- L2-normalized f32 LE (embedding.rs serde)
    embedding_dim     INTEGER NOT NULL,
    embedding_model   TEXT NOT NULL,              -- matcher only compares same-model vectors
    source_meeting_id TEXT,                       -- provenance; nullable if the meeting is later deleted
    sample_quality    REAL,                       -- pooled-seconds / extractor confidence; for best-N + deprioritizing weak samples
    created_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_voiceprints_person_id ON voiceprints(person_id);
```

### Repositories (`frontend/src-tauri/src/database/repositories/`)

- **`speaker.rs` (extend):**
  - `upsert(pool, meeting_id, speaker_key, display_name, is_local, embedding: Option<&[u8]>,
    embedding_dim: Option<i64>, embedding_model: Option<&str>)` — bind the three new columns; on
    `ON CONFLICT` update them too (so a re-run refreshes the embedding). All existing call sites pass
    `None, None, None` except the offline persist.
  - `get_identified_with_embeddings(pool) -> Vec<IdentifiedSpeaker>` — rows with **both** a non-null
    `embedding` **and** an identity key (`email IS NOT NULL OR person_id IS NOT NULL`), returning
    `(meeting_id, speaker_key, display_name, email, person_id, embedding, embedding_model)`. The
    matcher's prior-art source.
  - `link_person(pool, meeting_id, speaker_key, person_id)` — set `speakers.person_id` (1b).
- **`people.rs` (new, mirrors `speaker.rs`):** `create`, `get_by_id`, `get_all` (with a meeting-count
  join for the directory), `update` (display_name/role/notes), `upsert_by_email`, `delete`
  (transaction: clear `speakers.person_id` for this person → delete `voiceprints` for this person →
  delete the `people` row — explicit cascade), `set_voiceprint_opt_out(pool, id, opt_out)` (when
  turning **on**, deletes the person's `voiceprints` in the same tx; never deletes the person).
- **`voiceprints.rs` (new, 1c):** `insert(person_id, embedding, dim, model, source_meeting_id,
  quality)`, `best_n_for_person(person_id, n)` (ORDER BY `sample_quality` DESC, `created_at` DESC),
  `centroid_for_person(person_id) -> Option<(Vec<f32>, model)>` (best-N → `weighted_mean`/mean →
  `l2_normalize`, all from `embedding.rs`), `count_for_person`, `delete_for_person`, `clear_all`.

### Matcher — `diarization/identity.rs` (new, 1a; extended in 1b & 1c)

Pure module, **no network, no DB** beyond a passed-in candidate slice (so it unit-tests). Inputs:
this meeting's `Vec<(speaker_key, embedding, embedding_model)>` (remote only) + the candidate prior
art. Output:

```rust
pub struct SpeakerSuggestion {
    pub speaker_key: String,
    pub suggested_name: String,
    pub suggested_email: Option<String>,
    pub suggested_person_id: Option<String>, // 1b
    pub confidence: f32,                       // best cosine
    pub basis: SuggestionBasis,               // Calendar | PriorSpeaker | Person | VoiceprintGallery
}
```

- Compare **only** vectors sharing the same `embedding_model` (ADR-0007 §4).
- For each current remote speaker, cosine vs every candidate; accept the best **iff** `best ≥ τ_match`
  **and** `best − runner_up ≥ margin` (rejects ambiguous/confident-wrong — ADR-0007 §5).
- **τ_match starts ~0.5** (CAM++ cosine, per `specs/0011`); `margin` starts ~0.06. Both are module
  consts, tuned later on the benchmark. Ship conservative.
- **Ranking of sources** (1b/1c): exact `email` match (calendar) > `people` gallery-centroid match
  (1c) > `people` (no gallery, email/manual) > prior `speakers` embedding match. Strongest wins,
  carries its `basis` for the UI.
- **Confidence tiers (1c):** `high → auto-label` (only for gallery-centroid matches that clear the
  high threshold), `medium → suggest-and-confirm`, `low → Speaker N`. 1a/1b only ever *suggest*
  (no auto-label) — auto-label is exclusively a 1c gallery behavior.
- **Privacy unit test:** assert `SpeakerSuggestion` (and the `diarization-complete` payload that
  carries it) contains **no** embedding bytes; assert the module makes no I/O.

### Pipeline (`diarization/pipeline.rs`, 1a)

- Replace the turns-only `diarize_with_progress` call with `diarize_with_embeddings` (live already
  does this; the trait + sherpa impl are ready). Keep the progress emit (wrap or drop the per-chunk
  callback — acceptable; offline progress is coarse).
- In `persist`: for each **remote** key (`!= LOCAL_SPEAKER_KEY`), serialize its embedding via
  `embedding_to_bytes`, and pass `Some(bytes), Some(dim), Some(EMBEDDING_MODEL_ID)` to `upsert`.
  `local`/"You" stays NULL (ADR-0007 §3 self-enroll is a **1c** behavior, not here).
- After persisting, load candidates via `get_identified_with_embeddings`, run `identity.rs`, and
  attach `suggestions: Vec<SpeakerSuggestion>` to the existing `EVENT_COMPLETE` payload.

### Tauri IPC (register all in `frontend/src-tauri/src/lib.rs`; command+event pattern)

- **1a:** `api_get_speaker_suggestions(meeting_id) -> Vec<SpeakerSuggestion>` (re-runs the match for
  a saved meeting); `diarization-complete` payload gains `suggestions`.
- **1b:** `api_get_people()`, `api_get_person(id)`, `api_create_person(displayName, email?, role?,
  notes?)`, `api_update_person(id, …)`, `api_delete_person(id)`, `api_assign_speaker_to_person(
  meeting_id, speaker_key, person_id)` (links + sets `speakers.display_name`/`email` from the
  person — works regardless of `voiceprint_opt_out`).
- **1c:** `api_get_voiceprint_settings()` / `api_set_store_others_voiceprints(bool)` (global opt-in,
  default **false**, in `DiarizationSettings`); `api_set_person_voiceprint_opt_out(id, bool)`;
  `api_clear_all_voiceprints()`; `api_get_person_voiceprint_count(id)`. Enroll-on-confirm is **not**
  a standalone command — it fires inside `api_assign_speaker_to_person` /
  `api_assign_speaker_to_attendee` (insert a `voiceprints` row from the speaker's stored embedding
  **iff** `global_opt_in && !person.voiceprint_opt_out`; owner self-enroll iff the speaker is
  `is_local` and the owner person isn't opted out).

### Settings (`diarization/settings.rs`, 1c)

Add `#[serde(default)] pub store_others_voiceprints: bool` (default `false`) and
`#[serde(default = "default_true") ] pub self_enroll_owner: bool` (default `true`, ADR-0007 §3 opt-out).
Old files default correctly.

### UI (`frontend/src/`)

- **1a — suggestion chips** (`components/MeetingDetails/SpeakerLegend.tsx`, `hooks/useSpeakers.ts`):
  on a speaker row, a dismissible chip ("Looks like Priya · matched 2 meetings" / "· from calendar")
  that pre-fills the existing rename/assign control on click. Show the `basis`. **Dismiss = not
  re-offered** for that meeting (local state, mirrors `specs/0011`'s posture). Consume `suggestions`
  from `diarization-complete` and `api_get_speaker_suggestions`.
- **1b — People directory** (new route `app/people/`): list (name, role, meeting count) + detail
  (edit name/role/notes, delete). Reuse existing list/table primitives (`components/ui/`). **Extend
  the speaker-assign affordance** in `SpeakerLegend` so the pick-list offers *existing People*
  (`api_get_people`) alongside calendar attendees → `api_assign_speaker_to_person`. Available even
  when the person is opted out.
- **1c — voiceprint controls:** in Settings ("Speakers & diarization", `components/RecordingSettings.tsx`):
  the global **"Store other people's voiceprints"** toggle (off, with the ADR-0007 consent copy) and
  **"Clear all voiceprints"**. On the People detail: a **"Don't store this person's voice"** toggle
  (`voiceprint_opt_out`, warns it deletes existing samples) and **"Forget this person"** (delete +
  cascade, confirm dialog). Show a per-person sample count.

## Tasks (ordered; each phase independently shippable; owner agents in bold)

### Phase 1a — Cross-meeting matcher (offline path) — *not ADR-gated*
1. [ ] **rust-core-engineer** — migration `…_add_speaker_embedding_meta.sql` (`embedding_dim`,
   `embedding_model`); widen `SpeakersRepository::upsert` with `embedding/embedding_dim/embedding_model`
   (update existing call sites to `None,None,None`); add `get_identified_with_embeddings`. Update
   `SpeakerModel` (`database/models.rs`) if it must read the new columns.
2. [ ] **audio-engineer** — `pipeline.rs`: switch to `diarize_with_embeddings`; in `persist`, write
   per-**remote** embedding (`embedding_to_bytes` + `EMBEDDING_MODEL_ID`); `local` stays NULL.
3. [ ] **rust-core-engineer** — `diarization/identity.rs`: pure cosine matcher (τ_match + margin,
   same-model filter, `SpeakerSuggestion`/`SuggestionBasis`); `api_get_speaker_suggestions`; attach
   `suggestions` to `diarization-complete`; register in `lib.rs`. **Unit-test the no-leak/no-I/O
   invariant.**
4. [ ] **frontend-engineer** — dismissible suggestion chips in `SpeakerLegend`/`useSpeakers`
   pre-filling the rename/assign control; show basis; dismiss = not re-offered this meeting.

### Phase 1b — People entity — *not ADR-gated*
5. [ ] **rust-core-engineer** — migrations `…_add_people_table.sql`, `…_add_speaker_person_id.sql`;
   `PeopleRepository` (CRUD, `upsert_by_email`, `delete` with explicit cascade,
   `set_voiceprint_opt_out`); `SpeakersRepository::link_person`; `PersonModel` in `models.rs`.
6. [ ] **rust-core-engineer** — People IPC (`api_get_people`/`get_person`/`create`/`update`/`delete`)
   + `api_assign_speaker_to_person` (links + copies name/email; opt-out-agnostic); extend
   `identity.rs` to also rank `people`; register in `lib.rs`.
7. [ ] **frontend-engineer** — People directory route (`app/people/`); extend the `SpeakerLegend`
   assign pick-list to offer existing People → `api_assign_speaker_to_person` (works for opted-out
   people).

### Phase 1c — Voiceprint gallery + controls — **GATED on ADR-0007 (Accepted)**
8. [ ] **rust-core-engineer** — migration `…_add_voiceprints_table.sql`; `VoiceprintsRepository`
   (insert, best-N, `centroid_for_person`, delete_for_person, clear_all); wire person→voiceprints
   cascade into `PeopleRepository::delete` and `set_voiceprint_opt_out`.
9. [ ] **rust-core-engineer** — settings `store_others_voiceprints` (false) + `self_enroll_owner`
   (true); enroll-on-confirm inside `api_assign_speaker_to_person`/`api_assign_speaker_to_attendee`
   gated by `global_opt_in && !person.voiceprint_opt_out` (owner self-enroll on `is_local`);
   controls IPC (`set_store_others_voiceprints`, `set_person_voiceprint_opt_out`,
   `clear_all_voiceprints`, `get_person_voiceprint_count`); register in `lib.rs`.
10. [ ] **audio-engineer** — extend `identity.rs` to match against `people` gallery centroids
    (`centroid_for_person`) with the high/medium/low confidence tiers (high → auto-label, the only
    auto-label path).
11. [ ] **frontend-engineer** — voiceprint controls UI: global toggle + "clear all" in Settings;
    per-person opt-out + "forget this person" + sample count on People detail; ADR-0007 consent copy.
12. [ ] **rust-core-engineer** — privacy unit assertion: the `diarization-complete`/suggestion
    serialization and the summary payload contain **no** embedding/voiceprint bytes.

### Follow-on (NOT Wave 1 — tracked to `specs/0012`)
- [ ] **llm-pipeline-engineer** — role-weighted summary (`people.role` → `build_speaker_attributed_
  transcript` + `SPEAKER_ATTRIBUTION_INSTRUCTIONS`) + eval; preserve the no-role byte-identical path.

## Acceptance criteria (tie to the Definition of Done in `/CLAUDE.md`)

- **1a — embeddings persisted, owner excluded:** after an offline pass, every **remote** speaker row
  has a non-null `embedding` + `embedding_dim` + `embedding_model`; `local`/"You" stays NULL
  (`sqlite3` check).
- **1a — suggestion + no false-accept:** diarizing a meeting containing a previously-named speaker
  (same voice, prior `email`) yields a `SpeakerSuggestion` with the name + basis; a plausible-but-wrong
  voice produces **no** confident suggestion (τ_match + margin respected). Dismissed suggestions are
  not re-offered for that meeting.
- **1b — People persist & link, opt-out-agnostic:** creating a Person and assigning it to a detected
  speaker sets `speakers.person_id` and the display name; this **succeeds for a person with
  `voiceprint_opt_out = 1`**; a later meeting's matcher surfaces the same person (suggested).
- **1c — enroll gating:** with the global opt-in **off**, assigning a speaker to a non-owner person
  writes **no** `voiceprints` row; with it **on** and the person not opted out, exactly one row is
  written from that cluster's embedding; the owner self-enrolls by default (opt-out honored).
- **1c — controls:** "forget this person" deletes the `people` row **and** cascades their
  `voiceprints` (and clears `speakers.person_id`); "clear all voiceprints" empties the table; turning
  on per-person opt-out **deletes that person's voiceprints and blocks future enrollment** while the
  `people` row survives (all verified by DB assertion).
- **Privacy:** a unit test asserts the summary payload and the suggestion/`diarization-complete`
  payloads contain no embedding/voiceprint bytes; a packet capture during an enroll+match run (models
  cached) shows zero egress except the sanctioned LLM summary call (display names only).
- **No regression:** with no People/voiceprints and matcher producing nothing, behavior is identical
  to v0.5.0; the summary is byte-identical (role-weighting not yet wired).
- **Gate (`/check`):** `cargo check` + `cargo clippy` clean (`frontend/src-tauri`); `pnpm lint` clean
  (`frontend`); app launches via `./clean_run.sh`; record → live transcript → summary smoke unchanged.

## Risks / open questions

- **Cascade correctness without `PRAGMA foreign_keys` (1b/1c).** Person→voiceprints and
  forget-person→`speakers.person_id` cleanup **must** be explicit in delete transactions (mirror
  `repositories/meeting.rs`). A missed manual delete = orphaned biometric data (a privacy bug, not
  just a leak). Covered by the controls DB-assertion test.
- **τ_match / margin are empirical (1a).** CAM++ is English/VoxCeleb-trained (ADR-0007 caveat);
  too-low τ → confident-wrong suggestions (worst UX). Ship conservative (~0.5 / ~0.06), tune on the
  benchmark, always confirmable; gallery auto-label (1c) needs a *higher* bar than suggest.
- **Owner self-enroll provenance (1c).** Self-enroll requires an owner `people` row; decide its
  bootstrap (a singleton "You" person created on first self-enroll, `is_current_user`-style). Note,
  don't over-build — one owner person.
- **NULL-email collisions (1b).** Setting an email on an email-less person that collides with an
  existing person must be rejected gracefully (link the existing one). Repository-layer guard.
- **Re-run idempotency (1a).** `persist` clears + re-upserts speakers each pass; the widened upsert
  must refresh the embedding on `ON CONFLICT` so re-diarization doesn't strand a stale vector.
- **Enroll on attendee-assign too (1c).** `api_assign_speaker_to_attendee` must also enroll (an
  attendee assignment is a confirmed identity) — but it writes `email`, not `person_id`; it must
  upsert-or-link a `people` row first, then enroll. Keep the two assign paths consistent.
- **ADR scope.** 1a/1b add **no** biometric storage beyond the already-reserved `speakers.embedding`
  and are not blocked by ADR-0007; only 1c is. Do not ship 1c controls partially — all four
  (opt-in, opt-out, forget, clear-all) are prerequisites (ADR-0007 §6).

## Verification

- **1a:** name a speaker via calendar in meeting A (writes `email` + now `embedding`); re-diarize a
  meeting B with the same person → `api_get_speaker_suggestions` returns the name + basis; one-click
  applies. `sqlite3` confirms remote embeddings written, `local` NULL.
- **1b:** create a Person (no email); assign to a detected speaker in a transcript; confirm
  `speakers.person_id` set and the assign succeeded; set `voiceprint_opt_out=1` on that person and
  repeat the assign → still succeeds.
- **1c:** with global opt-in off, assign → no `voiceprints` row; turn it on, assign → one row; turn on
  per-person opt-out → row deleted, person remains, re-assign writes nothing; "forget this person" →
  person + voiceprints gone, `speakers.person_id` cleared; "clear all" → table empty.
- **Privacy:** packet capture during enroll+match (models cached) shows zero egress; unit tests
  assert no embedding bytes in summary/suggestion payloads.
- Run `/check` (cargo check/clippy in `frontend/src-tauri`; `pnpm lint` in `frontend`;
  `./clean_run.sh`; record → live transcript → summary smoke).

## Sources

- ADR + program: `docs/decisions/ADR-0007-biometric-voiceprint-storage.md` (Accepted);
  `specs/0013-identity-and-organization-program.md` (Wave 1 = 1a→1b→1c).
- Consolidated specs: `specs/0011-diarization-p3-live-and-cross-meeting-identity.md` (Task 11 matcher,
  `identity.rs` design, `get_identified_with_embeddings`); `specs/0012-people-and-role-weighted-summaries.md`
  (`people` table, role-weighting follow-on); house style: `specs/0015-meetings-as-first-class-objects.md`.
- Code anchors (verified 2026-06-27): `diarization/embedding.rs` (math + `ClusterEmbedder` +
  `EMBEDDING_MODEL_ID`); `diarization/mod.rs` (`diarize_with_embeddings`/`TurnsWithEmbeddings`);
  `diarization/live.rs:412` (already calls it); `diarization/pipeline.rs::persist` (no embedding
  today); `database/repositories/speaker.rs` (`upsert`/`assign_to_attendee`);
  `database/repositories/meeting.rs:416` (explicit delete cascade, no `PRAGMA foreign_keys`);
  `diarization/commands.rs:275` (`api_get_meeting_attendees`); `diarization/settings.rs`
  (`DiarizationSettings`); `diarization/models.rs` (`model_paths().embedding`);
  `migrations/20260624000000_add_speakers_table.sql` (`embedding BLOB`),
  `…_add_speaker_email.sql`; `frontend/src/components/MeetingDetails/SpeakerLegend.tsx`,
  `frontend/src/hooks/useSpeakers.ts`, `frontend/src/components/RecordingSettings.tsx`; `lib.rs:861`
  (command registration).
