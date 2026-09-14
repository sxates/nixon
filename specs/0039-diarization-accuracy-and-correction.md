# 0039 — Diarization accuracy, span-level correction & voiceprint-pollution guard

- **Status:** Shipped v1.7.0 (2026-07-06) — all three workstreams. WS1 landed merge-only
  (quiet-cluster *suppression* was intentionally dropped in code review — never delete a real
  speaker); constants were re-tuned by follow-on 0041/0043 (MERGE_FLOOR 0.30→0.45,
  CONSOLIDATE_FLOOR default 0.50/0.70) and 0043 W2 swapped the embedding to TitaNet-L (macro
  DER ~33%→~8% on the `tests/diarization_tuning.rs` ground-truth harness). Correctness is guarded
  by `eval_regression_gate` (pinned per-file DER ceilings). Header was stale until 2026-08-09.
- **Owner agent(s):** audio-engineer (WS1 clustering stability + WS3 gallery guard math) +
  rust-core-engineer (WS2 span-override plumbing, WS3 enrollment gate + quarantine) +
  frontend-engineer (WS2 correction UX, WS3 controls)
- **Roadmap phase:** Post-1.6 hardening
- **Parent feedback batch:** `specs/0038` (1.6 real-meeting dogfooding). This spec **graduated
  out of 0038** as its highest-pain / highest-risk item, mirroring how `specs/0019` spun WS4/WS5/WS7
  off into `specs/0020`–`0022`. 0038 keeps the lower-risk 1.6 items; this owns the diarization
  cluster.
- **Lineage:** refines the shipped diarization stack — `specs/0010` (offline P1/P2, `speakers`
  table, rename/merge), `specs/0011` (live labels + cross-meeting matcher, `Diarizer` trait,
  accuracy gate), `specs/0016` (People + voiceprint gallery, ADR-0007), `specs/0017`
  (`meeting_participants`, `SpeakerCount::AtMost` centroid-merge), `specs/0019` WS2.3 (sticky
  per-segment overrides), and `specs/0029` WS3.1–3.4 (run registry, rename preservation,
  roster-cap / overflow-as-Unknown, `transcripts.channel` "You" attribution). It does **not**
  contradict them; it extends WS3.2's rename-preservation lesson and WS3.3's cap semantics.
- **Decision records:** ADR-0005 (sherpa engine + `Diarizer` trait), ADR-0007 (biometric
  voiceprint storage — the hard gate this spec's WS3 tightens). No new ADR unless WS3's
  purge/quarantine schema proves load-bearing (see Risks).

## Context / Problem

A full day of real-meeting dogfooding on the **1.6 build** surfaced one dominant, compounding
complaint (owner, verbatim):

> "Diarization is often incorrect, especially on longer meetings. Speaker 1 at the beginning of a
> call will be attributed to another person later in the call, with no way to correct. Worried it
> will pollute our voiceprints and make the auto-attribution less accurate."

Three distinct failures are braided together here, and each amplifies the next:

1. **Within-meeting instability on long recordings (WS1).** The offline clustering pass is
   *global* — `SherpaDiarizer::compute_turns` runs sherpa's `compute()` over the **entire**
   `system.wav` in one shot (`sherpa.rs:323,421`) at the tuned Auto threshold
   `DEFAULT_AUTO_THRESHOLD = 0.8` (`sherpa.rs:161`). On long calls a voice that drifts (mic
   distance, codec, room change) splits into two clusters, or a quiet early speaker gets fused
   into someone else later. The "Speaker 1 early → someone else late" symptom is a global
   clustering-stability problem, not an alignment bug.

2. **No ergonomic correction for a mis-attributed *span* (WS2).** Sticky per-*line* correction
   exists (`specs/0019` WS2.3: `SegmentSpeakerMenu.tsx`, `api_set_segment_speaker`,
   `transcript_speaker_overrides`, re-applied at the end of every re-diarization by
   `pipeline::persist` → `reapply`, `transcript_speaker_overrides.rs:101`). But: it is
   **one line at a time** (unusable for the "the whole first 10 minutes is wrong" case), and it
   can only move a line to an **existing** speaker (`SegmentSpeakerMenu` returns null when
   `speakers.length < 2`) — there is no way to say "this misattributed span is actually a *new*,
   fourth speaker the clusterer missed." The owner's "no way to correct" is really "no way to
   correct *at the scale the error occurs*."

3. **Voiceprint-pollution risk from confirming a wrong attribution (WS3).** Enrollment is
   **enroll-on-confirm** (`people/enroll.rs:116`): when the user assigns a detected speaker to a
   Person (`api_assign_speaker_to_person` → `people/commands.rs:169`) or attendee
   (`diarization/commands.rs:459-466`), that speaker row's stored **cluster centroid** embedding is
   inserted into the person's gallery (`VoiceprintsRepository::add_sample`, `voiceprints.rs:46`),
   gated only by consent (`decide_enrollment`, `enroll.rs:59`) — **not by attribution
   confidence**. If WS1 produced a polluted cluster (two voices fused, or a drift-split remnant),
   confirming it enrolls a bad vector, which then *auto-labels* future meetings
   (`identity.rs` gallery path, `pipeline.rs:823-850`) — exactly the "pollute our voiceprints and
   make auto-attribution less accurate" flywheel-in-reverse the owner fears. There is currently
   **no confidence gate before enrollment, no purge/quarantine of a bad voiceprint, and no
   retraction path** when a manual correction (WS2) invalidates an embedding that was already
   enrolled.

### Non-goal up front (respect the fork): do **not** replace the model

Per `/CLAUDE.md` and ADR-0005, WS1 tunes **sizing / clustering / consolidation only** — the same
lesson as `specs/0029` WS3.3 and `specs/0017`'s centroid-merge. We keep sherpa (`3D-Speaker` CAM++
embedding + pyannote segmentation) and the `Diarizer` trait; swapping to `speakrs` remains the
`specs/0011` accuracy-gate escape hatch, out of scope here.

### Two deferred backlog items this spec resolves-or-supersedes

- **"Seed offline diarization from live labels (cut post-meeting latency)"** (`specs/BACKLOG.md:178`,
  from `specs/0019` WS2.6). Adjacent but **not** adopted here: WS1 keeps the offline pass
  authoritative and instead improves *its* stability. If WS1's global-consolidation pass lands, live
  seeding becomes a pure latency optimization on top; the backlog item stays deferred but this spec
  is its prerequisite (a stable global registry is exactly what live-seeding would reconcile to).
- **`Fixed(1)` short-circuit** (`specs/BACKLOG.md:203`). Orthogonal compute saving; WS1 must not
  regress it — the 1-remote-attendee path still forces `Fixed(1)` and can still skip clustering.

### Severity & sequencing

| Pri | WS | Item | Why |
|---|---|---|---|
| **P1** | WS1 | Long-meeting attribution drift (Speaker 1 → someone else later) | Root cause of the top complaint; global clustering stability |
| **P1** | WS2 | Span-level correction + reassign-to-new-speaker | The "no way to correct" half; must persist through re-runs |
| **P1** | WS3 | Confidence gate before enrollment + purge/quarantine + retraction | Protects the cross-meeting flywheel from WS1/WS2 errors |

WS1 and WS2 ship independently (WS2 does not depend on WS1 and gives the user a manual escape hatch
even before WS1 lands). **WS3's retraction path depends on WS2's correction events**, so WS2
precedes the retraction slice of WS3; WS3's confidence gate and quarantine can land in parallel.

## Goals

- **Long-meeting stability (WS1):** a speaker introduced early keeps one stable identity for the
  whole call. Concretely: reduce global over-/under-split via a **centroid-consolidation pass** over
  the completed clustering (merge drift-split clusters of the *same* voice; keep genuinely distinct
  voices apart), reusing the existing `merge_clusters_to_at_most` / `embedding.rs` math — no new
  model, no windowed re-clustering that re-introduces churn.
- **Span correction (WS2):** the user can select a contiguous run of transcript lines and reassign
  the whole span to another speaker — existing **or a newly-created one** — in one action, from the
  transcript view. Corrections persist through re-diarization exactly like today's per-line overrides
  (the WS3.2 preservation lesson).
- **Pollution guard (WS3):** (a) an embedding contributes to a person's voiceprint gallery only when
  the attribution is **confident and/or user-confirmed**, never from a low-confidence auto-label;
  (b) the user can **purge or quarantine** a bad voiceprint and re-derive the person's gallery;
  (c) a WS2 correction that invalidates a span **retracts** the voiceprint samples that span
  contributed.
- All on-device; no new outbound traffic; no new dependency; no new model (reuses the loaded CAM++
  extractor, sherpa, `embedding.rs`).

## Non-goals

- **Replacing the diarization model / clustering algorithm.** WS1 tunes consolidation + sizing only
  (ADR-0005, `/CLAUDE.md`). `speakrs` remains the `specs/0011` gate escape hatch, not this spec.
- **Rewriting the meetily capture / mix / VAD pipeline.** Untouched.
- **Windowed / streaming re-clustering for the offline pass.** `specs/0011` chose global clustering
  precisely for accuracy; WS1 improves the *global* result, it does not window it.
- **Live-diarization changes.** Live labels (`specs/0011` Task 10) are provisional and reconciled by
  the offline pass; WS1/WS2 operate on the authoritative offline path. Seeding offline from live
  stays deferred (`BACKLOG.md:178`).
- **Explicit voice-enrollment product** ("record 10 s"). Enrollment stays opportunistic
  (ADR-0007 §4); WS3 only *gates and cleans* it.
- **Merge-people UX, cross-device voiceprint sync.** Out (as in `specs/0016`).
- **Changing manual `expected_speaker_count` semantics** (`Fixed(n)` stays exact) or the
  overflow-as-Unknown behavior (`specs/0029` WS3.3, `UNKNOWN_SPEAKER_KEY`, `sherpa.rs:196`).

---

## WS1 — Within-meeting attribution stability on long recordings (P1)

> Owner: audio-engineer (consolidation math + sherpa integration) + rust-core-engineer (persist
> path). Tunes sizing/clustering only — the `specs/0029` WS3.3 / `specs/0017` posture.

**Problem:** On long meetings a speaker's identity is unstable: "Speaker 1 at the beginning is
attributed to another person later." Two mechanisms produce this — a single voice **splits** into
two clusters over the call (drift), and two voices **fuse** or swap when one is quiet.

**Root cause (grounded):** Clustering is **global and single-shot** —
`SherpaDiarizer::compute_turns` (`sherpa.rs:323`) hands the entire `system.wav` buffer to
`guard.compute(samples, …)` (`sherpa.rs:421`) at the fixed Auto distance threshold
`DEFAULT_AUTO_THRESHOLD = 0.8` (`sherpa.rs:161`; higher = merge more, lower = split more, per the
doc-comment at `sherpa.rs:98-104`). That threshold was swept on a ~12-min file
(`DiarizeTuning` table, `sherpa.rs:113-138`); there is **no length-adaptive behavior and no
post-clustering same-voice consolidation** except the `AtMost(n)` cap path
(`merge_clusters_to_at_most`, `sherpa.rs:554`), which only fires when a roster/manual bound exists
(`specs/0017`). In pure `Auto` mode (the common case — hand-added participants aren't linked as
remote `person_id`s, so `resolve_meeting_speaker_count` falls through to `Auto`, per `specs/0029`
WS3.3) nothing merges drift-split clusters. The only Auto-mode safeguard is a **warn-only** log
above `OVER_CLUSTER_WARN_CAP = 30` clusters (`sherpa.rs:168,471-478`) — it never acts. So a long
call that produces 8 clusters for 4 real voices is silently presented as 8 first-class speakers,
with the same voice split across `spk_1` (early) and `spk_5` (late) — the reported symptom.

**Approach (consolidation, not re-clustering — the decisive choice):** Add an **unconditional
global centroid-consolidation pass** that runs after sherpa clusters, in **all** modes (not just
`AtMost`), reusing the per-cluster centroids `diarize_with_embeddings` already computes:

1. **Same-voice merge, threshold-driven (not count-driven).** Generalize the existing
   `merge_clusters_to_at_most` (`sherpa.rs:554`) into a sibling
   `consolidate_clusters(turns, embeddings, opts)` that merges the mutually-closest cluster pair
   **while** cosine similarity ≥ a new **`CONSOLIDATE_FLOOR`** (distinct from, and higher than,
   `MERGE_FLOOR = 0.30`, `sherpa.rs:188`). This targets drift-split *same-voice* clusters (which sit
   at high cosine) without the cap forcing distinct voices together. When an `AtMost(n)` bound is
   also present, consolidation runs first (same-voice cleanup), then the existing cap logic enforces
   `n` — the two compose, they don't conflict.
2. **Duration-weighted centroids + minimum-cluster suppression.** A cluster with very little total
   speech (a few hundred ms of a crosstalk artifact) is a frequent spurious split source. Fold such
   sub-threshold clusters into their nearest above-`CONSOLIDATE_FLOOR` neighbor (or, if none, the
   `UNKNOWN_SPEAKER_KEY` bucket, `sherpa.rs:196`) rather than presenting them as speakers. Use the
   pooled-duration weighting `merge_clusters_to_at_most` already computes via
   `embedding.rs::weighted_mean`.
3. **Length-aware guardrail, still non-destructive by default.** Keep `OVER_CLUSTER_WARN_CAP`'s warn
   (`sherpa.rs:471`) but make it *inform* consolidation: on long recordings where distinct-cluster
   count is implausible for the elapsed audio, log the consolidation decision (before/after counts,
   floor used) so we can tune on real files via the existing `tests/diarization_tuning.rs` fixture.
4. **Keep it a pure, unit-testable helper** — `consolidate_clusters` takes `(turns, embeddings,
   opts) -> (turns, embeddings)` with no ONNX, exactly like `merge_clusters_to_at_most`, so the
   drift-split and quiet-speaker cases are covered by `cargo test` fixtures without a mic.

**Why consolidation over the alternatives:** windowed / sliding re-clustering was rejected by
`specs/0011` (Option B) because it re-discovers and re-numbers speakers — it would *cause* the
exact drift symptom we're fixing. Lowering the global threshold splits more (worse). Consolidation
keeps sherpa's global decision and only *repairs* the well-understood same-voice-split failure with
math we already ship and already trust for the roster cap. It degrades to a no-op when clustering is
already clean (so no-regression on short/clean calls is trivially provable).

**Persist-path interaction (grounded, must not regress):** `persist` (`pipeline.rs:331`) already
snapshots user renames (`get_identity_snapshots`, `speaker.rs:243`), unions override target keys
(`override_keys_for_meeting`), clears (`clear_meeting_speakers`, `pipeline.rs:377`), re-upserts, and
restores renames by stable-key-then-centroid (`restore_user_identities`, `pipeline.rs:453`) and
re-applies overrides (`reapply`, `pipeline.rs:387`). Consolidation changes **which `spk_N` keys
exist**, so it must run **before** the snapshot-restore centroid fallback so a consolidated key still
matches a prior rename by centroid — verify `restore_user_identities`' centroid path still lands the
name onto the surviving merged key.

**Files:** `frontend/src-tauri/src/diarization/sherpa.rs:161,168,188,323,421,471,554`
(new `CONSOLIDATE_FLOOR` const + `consolidate_clusters` helper, called inside
`diarize_with_embeddings`); `frontend/src-tauri/src/diarization/embedding.rs`
(`weighted_mean`/`cosine_similarity`/`l2_normalize`, reused); `frontend/src-tauri/src/diarization/pipeline.rs:331,377,453`
(ordering vs rename-restore); `frontend/src-tauri/tests/diarization_tuning.rs` (drift/quiet
fixtures). Settings: an optional `consolidation_floor` override alongside `diarization_threshold` in
`diarization/settings.rs` for tuning, `#[serde(default)]`.

---

## WS2 — Span-level manual correction (P1)

> Owner: frontend-engineer (transcript selection + reassign UX) + rust-core-engineer (bulk override
> command). Extends the shipped per-line override plumbing; must preserve corrections through
> re-runs (the `specs/0029` WS3.2 lesson).

**Problem:** When a run of lines is mis-attributed (the whole first stretch of the call under the
wrong speaker), the only fix is the per-line `SegmentSpeakerMenu` "This line is…" control — one
line at a time — and it can only target an **already-existing** speaker. There is no way to reassign
a **span**, and no way to reassign a mis-clustered span to a **new** speaker the clusterer missed.

**Root cause / current state (grounded — the plumbing is 80% there):**
- Sticky per-line overrides work end-to-end: `TranscriptSpeakerOverridesRepository::set`
  (`transcript_speaker_overrides.rs:24`) updates the live `transcripts.speaker` **and** records a
  durable override in one transaction; `pipeline::persist` calls `reapply`
  (`transcript_speaker_overrides.rs:101`, invoked at `pipeline.rs:387`) at the **end** of every
  re-diarization, and unions override target keys into the materialized `speakers` rows
  (`pipeline.rs:361-369`) so a corrected line always resolves to a name — the corrections **already
  survive re-runs**. Commands `api_set_segment_speaker` / `api_clear_segment_speaker`
  (`diarization/commands.rs:334,361`).
- The UI gap: `SegmentSpeakerMenu.tsx` is per-line (`onReassign(transcriptId, speakerKey)`), and
  bails when `speakers.length < 2` — **no span selection, no create-new-speaker**. The transcript
  renders through `VirtualizedTranscriptView.tsx` (which already receives `speakers` +
  `onReassignSegment`) inside `MeetingDetails/TranscriptPanel.tsx`; speaker identity CRUD lives in
  `hooks/useSpeakers.ts` (`renameSpeaker`/`assignPerson`/`mergeSpeakers`) + `SpeakerLegend.tsx`.

**Approach:**
1. **Bulk override command (rust-core).** Add `api_set_segment_speakers(meeting_id,
   transcript_ids: Vec<String>, speaker_key)` — a batched form of `set` in a single transaction
   (loop or `IN (…)` bulk `UPDATE` + upsert per id), so a span reassignment is one atomic write that
   `reapply` will honor on the next pass. Reuse the exact override table; no schema change.
2. **Create-a-new-speaker on correction (rust-core).** Add `api_create_meeting_speaker(meeting_id,
   display_name) -> speaker_key` that mints a fresh `spk_<n>`/`manual_<uuid>` key + a `speakers` row
   (via `SpeakersRepository`), so a span can be moved to a brand-new identity the clusterer never
   produced. The new key participates in overrides and thus survives re-runs (it's in
   `override_keys_for_meeting`). Because it has **no embedding**, it is inert for WS3 enrollment
   (safe by construction — see WS3).
3. **Span-selection UX (frontend).** In the meeting-details transcript view: shift-click / drag to
   select a contiguous run of lines; a single "Reassign N lines to…" action offering the meeting's
   existing speakers **plus** "New speaker…" (which calls `api_create_meeting_speaker` then the bulk
   override). Reuse `speaker-colors.ts` and the `SegmentSpeakerMenu` list affordance; keep the
   per-line menu for single fixes. Optimistic update, revert on error (the WS5-style pattern from
   `specs/0029`).
4. **Corrections outlive re-diarization (verify, don't rebuild).** `reapply` already covers this;
   the acceptance test is span-correct → re-diarize → span intact, extending the WS3.2 regression.

**Files:** `frontend/src-tauri/src/database/repositories/transcript_speaker_overrides.rs:24,101`
(bulk `set_many`); `frontend/src-tauri/src/diarization/commands.rs:334,361`
(`api_set_segment_speakers`, `api_create_meeting_speaker`; register in
`frontend/src-tauri/src/lib.rs`); `frontend/src-tauri/src/database/repositories/speaker.rs`
(new-speaker mint); `frontend/src/components/MeetingDetails/SegmentSpeakerMenu.tsx`,
`frontend/src/components/VirtualizedTranscriptView.tsx`,
`frontend/src/components/MeetingDetails/TranscriptPanel.tsx`,
`frontend/src/components/MeetingDetails/SpeakerLegend.tsx`, `frontend/src/hooks/useSpeakers.ts`.

---

## WS3 — Voiceprint-pollution guard (P1)

> Owner: rust-core-engineer (enrollment gate + purge/quarantine + retraction) + audio-engineer
> (re-derivation math). Tightens ADR-0007's enroll-on-confirm; adds cleanup + retraction the
> gallery has never had.

**Problem:** A wrong in-meeting attribution can poison a person's durable voiceprint, degrading
future auto-attribution — the owner's explicit fear. Today nothing prevents it and nothing cleans it
up.

**Root cause (grounded, three gaps):**
- **(a) No confidence gate at enrollment.** `enroll_voiceprint_for_speaker` (`enroll.rs:116`) reads
  the speaker row's stored cluster centroid (`get_speaker_embedding`, `speaker.rs:197`) and inserts
  it into the gallery (`VoiceprintsRepository::add_sample`, `voiceprints.rs:46`) whenever
  `decide_enrollment` (`enroll.rs:59`) says the **consent** gate passes
  (`store_others_voiceprints && !voiceprint_opt_out`, or owner self-enroll). There is **no check on
  how confident/clean the attribution is.** Worse, the offline pass **auto-labels** high-confidence
  gallery matches by calling `PeopleRepository::assign_speaker_to_person` **directly**
  (`pipeline.rs:823-841`) — which today does *not* enroll (enrollment only fires through the
  `api_*` commands, `people/commands.rs:169` and `diarization/commands.rs:459-466`). That's the one
  saving grace; but any user confirmation of a *polluted* cluster (a WS1 fusion, or a
  `UNKNOWN_SPEAKER_KEY` mixed bucket if it ever carried an embedding) enrolls a bad vector.
- **(b) No purge / quarantine.** The only gallery-deletion controls are all-or-nothing per person:
  `set_voiceprint_opt_out` (deletes all of a person's samples, `people.rs:197`), `delete_for_person`
  (`voiceprints.rs:183`), and `api_clear_all_voiceprints` (`voiceprints.rs:193`,
  `diarization/commands.rs:254`). There is **no way to remove one bad sample** and keep the good
  ones, and no way to see *which meeting* a sample came from (though `voiceprints.source_meeting_id`
  is stored — `20260628000002_add_voiceprints.sql`).
- **(c) No retraction on correction.** When WS2 corrects a span, any voiceprint sample previously
  enrolled from that (now-known-wrong) cluster stays in the gallery forever. `add_sample` records
  `source_meeting_id` but nothing keys a sample back to the **speaker cluster** it came from, so
  there's no handle to retract by.

**Approach:**
1. **Confidence-gated enrollment (rust-core + audio).** Add an attribution-confidence signal to the
   enroll gate. `decide_enrollment` (`enroll.rs:59`) grows a `confidence: EnrollConfidence` input:
   - **User-confirmed** (an explicit `api_assign_speaker_to_person` / `..._to_attendee` from a human)
     → highest trust → enroll (subject to the existing consent gate). This is today's path; it stays.
   - **Auto-label** (`pipeline.rs:823` gallery match) → **must never enroll** (it already doesn't;
     make that an *explicit, tested invariant* rather than an accident of which function is called).
   - **Cluster quality**: refuse enrollment for a centroid derived from too little pooled speech, or
     from the `UNKNOWN_SPEAKER_KEY` overflow bucket (which is a mix of voices and already carries no
     centroid, `sherpa.rs:191-196` — assert it), or from a span that has an active manual override
     (a corrected/contested cluster is not a clean voiceprint source). Thread a
     `sample_quality` score (currently always `None`, `enroll.rs:176-178`) so best-N later
     deprioritizes weak samples.
2. **Purge & quarantine + re-derive (rust-core).** Give the gallery per-sample controls:
   - `api_delete_voiceprint_sample(voiceprint_id)` and a new `VoiceprintsRepository::delete_sample` /
     `list_for_person` (returning `id, source_meeting_id, created_at, sample_quality`) so the user
     can see and remove **one** bad sample.
   - **Quarantine** (soft-delete) rather than hard-delete by default: add a nullable
     `quarantined_at TEXT` column (migration `20260709000000_add_voiceprint_quarantine.sql`) so a
     suspected-bad sample is excluded from `centroid_for_person`/`all_centroids`
     (`voiceprints.rs:109,130`) matching **without** losing provenance, and can be restored if the
     purge was wrong. `centroid_for_person` and `best_n_for_person` filter `quarantined_at IS NULL`.
   - **Re-derive**: after purge/quarantine the person's centroid is recomputed on read
     (`centroid_for_person` already computes on read — no stored centroid to rebuild), so "re-derive"
     is automatic once bad samples are excluded; expose `api_get_person_voiceprint_count`
     (`diarization/commands.rs:266`) alongside a live/quarantined breakdown for the UI.
3. **Retraction on WS2 correction (rust-core, depends on WS2).** When a span is reassigned away from
   speaker `S` (WS2's `api_set_segment_speakers` / `api_create_meeting_speaker`), **retract** the
   voiceprint sample(s) that `S`'s cluster contributed for that meeting. Requires a back-link: add
   `source_speaker_key TEXT` to `voiceprints` (same migration) so a sample records
   `(source_meeting_id, source_speaker_key)`. On a correction that materially changes what `S` is,
   quarantine `voiceprints WHERE source_meeting_id = ? AND source_speaker_key = ?` for the affected
   person and log it — conservative (quarantine, not delete) so an over-eager retraction is
   recoverable.
4. **Owner ("You") unaffected by remote pollution.** The mic channel is `LOCAL_SPEAKER_KEY`
   (`align.rs:25`) and enrolls only under the singleton owner person (`OWNER_PERSON_ID`,
   `enroll.rs:34`) via the self-enroll gate — remote mis-attribution can't poison the owner gallery,
   and WS1 consolidation runs only on the system channel. Assert this holds.

**Files:** `frontend/src-tauri/src/people/enroll.rs:34,59,116,176`
(confidence input to `decide_enrollment`; refuse weak/overridden/unknown-bucket clusters);
`frontend/src-tauri/src/database/repositories/voiceprints.rs:46,109,130,183,193`
(`quarantined_at`/`source_speaker_key` columns; `delete_sample`, `list_for_person`, quarantine
filter in centroid reads); new migration
`frontend/src-tauri/migrations/20260709000000_add_voiceprint_quarantine.sql`;
`frontend/src-tauri/src/diarization/commands.rs:254,266,334`
(`api_delete_voiceprint_sample`, `api_quarantine_voiceprint_sample`, list command; retraction hook
from `api_set_segment_speakers`); `frontend/src-tauri/src/database/repositories/people.rs:197,291`
(assign/opt-out paths unaffected); `frontend/src-tauri/src/diarization/pipeline.rs:823-850`
(assert auto-label never enrolls); UI: `frontend/src/components/People/` (per-sample list + purge on
the People detail), `frontend/src/components/RecordingSettings.tsx` (existing gallery controls),
`frontend/src/hooks/useSpeakers.ts` (retraction feedback). Register all commands in
`frontend/src-tauri/src/lib.rs`.

---

## Design

### Data model (forward-only migration under `frontend/src-tauri/migrations/`)

Next prefix after `20260708000300_*` → **`20260709000000_add_voiceprint_quarantine.sql`**. Additive,
idempotent (`ALTER … ADD COLUMN`); existing rows stay valid. No `PRAGMA foreign_keys` — cascades
stay explicit in delete transactions (per `specs/0016` grounding).

```sql
-- specs/0039 WS3: soft-delete (quarantine) a suspected-bad voiceprint sample and
-- back-link it to the speaker cluster it came from, so a WS2 span correction can
-- retract exactly the samples that cluster contributed. Both nullable; existing
-- samples (all NULL) remain live and un-retractable-by-cluster (acceptable — they
-- predate the back-link and can still be quarantined by sample id).
ALTER TABLE voiceprints ADD COLUMN quarantined_at     TEXT;  -- NULL = live; set = excluded from centroid/match
ALTER TABLE voiceprints ADD COLUMN source_speaker_key TEXT;  -- the spk_N cluster this sample was enrolled from
CREATE INDEX IF NOT EXISTS idx_voiceprints_source
    ON voiceprints(source_meeting_id, source_speaker_key)
    WHERE source_speaker_key IS NOT NULL;
```

No new table for WS1 (pure in-memory consolidation) or WS2 (reuses `transcript_speaker_overrides`,
`speakers`). WS2's new-speaker mint reuses the existing `speakers` schema (`20260624000000`) with a
NULL embedding.

### Tauri IPC (register in `frontend/src-tauri/src/lib.rs`; command+event pattern)

- **WS2:** `api_set_segment_speakers(meeting_id, transcript_ids, speaker_key)`,
  `api_create_meeting_speaker(meeting_id, display_name) -> speaker_key`.
- **WS3:** `api_list_person_voiceprints(person_id) -> Vec<VoiceprintSample>`
  (`{id, source_meeting_id, created_at, sample_quality, quarantined}`),
  `api_quarantine_voiceprint_sample(voiceprint_id, quarantined: bool)`,
  `api_delete_voiceprint_sample(voiceprint_id)`. Retraction is invoked **inside**
  `api_set_segment_speakers` (best-effort, log-and-continue), not a standalone command.
- No new events. `diarization-complete` (`pipeline.rs:865`) is unchanged on the wire; WS1
  consolidation just yields a cleaner `speaker_count`. UI re-fetches speakers/voiceprints after
  correction, as today.

### UI (`frontend/src/`)

- **WS2 span correction:** `VirtualizedTranscriptView` gains shift-click/drag line selection + a
  "Reassign N lines" action (existing speakers + "New speaker…"); `SegmentSpeakerMenu` stays for
  single lines. `SpeakerLegend` shows manually-created speakers.
- **WS3 gallery controls:** People detail (`components/People/`) lists each voiceprint sample with
  its source meeting + date and a per-sample **Quarantine / Delete**; a **Re-derive** affordance is
  implicit (centroid recomputes on read). Settings (`RecordingSettings.tsx`) keeps the global
  toggle + "Clear all" (`specs/0016` 1c).

## Tasks (ordered; owner agents in bold)

### WS1 — clustering stability
1. [ ] **audio-engineer** — `sherpa.rs`: add `CONSOLIDATE_FLOOR` const + pure
   `consolidate_clusters(turns, embeddings, opts) -> (turns, embeddings)` (same-voice merge above the
   floor, quiet-cluster suppression, duration-weighted centroids); call it in
   `diarize_with_embeddings` **before** the `AtMost` cap; compose with `merge_clusters_to_at_most`.
   Unit-test drift-split and quiet-speaker fixtures (no ONNX).
2. [ ] **rust-core-engineer** — verify/adjust `pipeline.rs:331-453` ordering so
   `restore_user_identities`' centroid fallback still lands renames on consolidated keys; add the
   optional `consolidation_floor` setting (`diarization/settings.rs`, `#[serde(default)]`).
3. [ ] **audio-engineer** — extend `tests/diarization_tuning.rs` on a real long `system.wav` fixture;
   record before/after distinct-speaker counts + chosen floor in the spec/ADR-0005 addendum.

### WS2 — span correction
4. [ ] **rust-core-engineer** — `transcript_speaker_overrides.rs` bulk `set_many`;
   `api_set_segment_speakers`, `api_create_meeting_speaker` (mints `speakers` row, NULL embedding);
   register in `lib.rs`. Confirm `reapply` + `override_keys_for_meeting` cover the new key.
5. [ ] **frontend-engineer** — span selection in `VirtualizedTranscriptView` + "Reassign N lines to…"
   (existing speakers + "New speaker…"); keep per-line `SegmentSpeakerMenu`; optimistic update;
   `useSpeakers` wiring. Regression test: span-correct → re-diarize → span intact (extends WS3.2).

### WS3 — pollution guard
6. [ ] **rust-core-engineer** — migration `20260709000000_add_voiceprint_quarantine.sql`
   (`quarantined_at`, `source_speaker_key`, index); `VoiceprintsRepository`: write
   `source_speaker_key` from `add_sample`; `delete_sample`, `list_for_person`, filter
   `quarantined_at IS NULL` in `centroid_for_person`/`all_centroids`.
7. [ ] **rust-core-engineer + audio-engineer** — confidence gate: `EnrollConfidence` param on
   `decide_enrollment`; refuse weak/overridden/`UNKNOWN_SPEAKER_KEY` clusters at
   `enroll_voiceprint_for_speaker`; make "auto-label never enrolls" (`pipeline.rs:823`) an explicit,
   tested invariant.
8. [ ] **rust-core-engineer** — retraction hook inside `api_set_segment_speakers`: quarantine
   `voiceprints` for `(source_meeting_id, source_speaker_key)` of the corrected-away speaker
   (best-effort, logged); commands `api_list_person_voiceprints`,
   `api_quarantine_voiceprint_sample`, `api_delete_voiceprint_sample`; register in `lib.rs`.
9. [ ] **frontend-engineer** — per-sample voiceprint list + quarantine/delete on People detail;
   retraction feedback toast; keep global controls in `RecordingSettings`.

### Cross-cutting
10. [ ] **spec-architect** — if the quarantine/source-link schema proves cross-cutting beyond this
    spec, an ADR-0005 addendum (consolidation floor + retraction semantics) or a short new ADR.

## Acceptance criteria

Tie to the Definition of Done in `/CLAUDE.md` (`cargo check`/`clippy`/`test` clean in
`frontend/src-tauri`; `pnpm lint`/`pnpm test` clean in `frontend`; app launches via
`./clean_run.sh`; record → live transcript → summary smoke unchanged).

- **WS1 — stability:** on a long multi-speaker `system.wav` fixture where a single voice drifts,
  the consolidation pass yields **one** stable key for that voice (not an early + late split);
  distinct-speaker count is plausible for the audio; a clean short call diarizes **byte-identically**
  to today (consolidation no-ops); `restore_user_identities` still lands a prior rename on the merged
  key. Unit tests cover drift-split, quiet-cluster, and no-op cases without ONNX.
- **WS2 — span correction:** selecting a contiguous run of N lines and reassigning them (to an
  existing speaker **or** a newly-created one) updates all N in one action; after a re-diarization
  the span keeps the corrected speaker (regression test); the new speaker appears in the legend and
  resolves to a name. Reproduce the owner's case: first-stretch mis-attribution corrected in one
  gesture.
- **WS3 — pollution guard:** (a) a low-confidence auto-label writes **no** voiceprint row
  (`sqlite3` assertion); a user confirmation of a clean cluster still enrolls one; a cluster with an
  active manual override or the `unknown` bucket enrolls nothing. (b) Quarantining one sample
  excludes it from `centroid_for_person` while the row survives with provenance; deleting removes it;
  the person's other samples still match. (c) Correcting a span (WS2) quarantines the voiceprint
  samples that span's speaker contributed for that meeting (`source_meeting_id`+`source_speaker_key`
  assertion). The owner ("You") gallery is never touched by remote correction/consolidation.
- **Privacy / no-regression:** no new outbound traffic; no embedding bytes in any IPC/summary payload
  (existing `specs/0016` assertion still holds); with WS3 gates default-consistent and no
  corrections, behavior matches 1.6.

## Risks / open questions

- **`CONSOLIDATE_FLOOR` is empirical (WS1) — the primary risk.** Too high → drift-splits not merged
  (no improvement); too low → two genuinely distinct voices fused (worse than the split). It must sit
  **above** `MERGE_FLOOR = 0.30` (the anti-false-merge cap floor) since consolidation should be
  *more* conservative than a user-requested cap. Ship conservative, tune on real long files via
  `tests/diarization_tuning.rs`. **Open:** single global floor vs. length-adaptive.
- **Consolidation vs. the `AtMost` cap ordering.** Consolidating same-voice clusters first can drop
  the count below a roster cap — intended and safe (cap is an upper bound), but verify the
  `diarization-complete` `seededSpeakerCount`/`speakerCountMode` (`pipeline.rs:850-864`) still reads
  correctly when both fire.
- **WS2 new-speaker keys and re-runs.** A user-created `manual_<uuid>` key has no embedding, so a
  later diarization can't re-derive it from audio — it persists **only** via the override table. If
  the user later clears the override, the span reverts to the clusterer. Document; it's the intended
  sticky-override semantics.
- **WS3 retraction over-reach.** A correction might legitimately keep *most* of a cluster right and
  only move a slice; blanket-quarantining all of that speaker's samples for the meeting could discard
  a good voiceprint. **Mitigation:** quarantine (recoverable), not delete; only trigger on a
  *material* reassignment (span covers a large fraction of the speaker's turns), and surface it so the
  user can restore. **Open:** what fraction counts as "material," and should retraction be opt-in via
  a toast rather than automatic?
- **Legacy samples have no `source_speaker_key`.** Pre-migration voiceprints can't be retracted by
  cluster (only by sample id in the new per-sample UI). Acceptable; note it.
- **No-embedding on the `unknown` bucket.** WS3's gate relies on `UNKNOWN_SPEAKER_KEY` carrying no
  centroid (`sherpa.rs:191-196`) — assert this in a test so a future change can't silently start
  enrolling the mixed bucket.
- **Auto-label path is the sharpest edge (WS3).** It currently avoids enrollment only because it
  calls the repository method, not the `api_` command (`pipeline.rs:823` vs `people/commands.rs:169`).
  A future refactor that routes auto-label through the command would start enrolling low-confidence
  matches. The tested invariant (task 7) is the guard.

## Verification

- **Rust:** `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal &&
  cargo clippy && cargo test --features metal --test db_lifecycle --test diarization_tuning`. Add:
  `consolidate_clusters` drift/quiet/no-op unit tests (WS1); bulk-override + new-speaker + survive-
  re-run tests (WS2); enroll-confidence-gate, quarantine-excludes-from-centroid, and
  correction-retracts-samples tests (WS3); owner-gallery-untouched + `unknown`-bucket-no-embedding
  assertions.
- **Frontend:** `cd frontend && pnpm lint && pnpm test`. Add: span-selection reassign + optimistic-
  revert tests; per-sample voiceprint list/quarantine tests.
- **Manual smoke (real long meeting, dev build):** record a 30-min+ multi-person call → run Identify
  speakers → confirm a single early-speaker keeps one identity through to the end (WS1) → select the
  first mis-attributed stretch, reassign it to the right / a new speaker in one gesture, re-run
  diarization, confirm the correction holds (WS2) → assign a clean cluster to a Person (enrolls),
  then correct a span away from a previously-confirmed speaker and confirm its voiceprint sample is
  quarantined; open People detail, purge a bad sample, confirm matching improves next meeting (WS3).
- **Privacy check:** packet capture during correction + enrollment (models cached) shows zero egress.
- Run `/check` (cargo check/clippy, pnpm lint, `./clean_run.sh`, record → transcript → summary
  smoke). Production-build sanity per `/CLAUDE.md` (`./build-gpu.sh` / `./upgrade-vinyl.sh`).

## Sources

- Parent batch: `specs/0038` (1.6 feedback). Lineage: `specs/0010`, `specs/0011`, `specs/0016`,
  `specs/0017`, `specs/0019` WS2.3, `specs/0029` WS3.1–3.4. Decisions: ADR-0005, ADR-0007. Backlog:
  `specs/BACKLOG.md:178` (live-seed offline), `:203` (`Fixed(1)` short-circuit). House style:
  `specs/0029`.
- Code anchors (verified 2026-07-06): `diarization/sherpa.rs:161` (`DEFAULT_AUTO_THRESHOLD`), `:168`
  (`OVER_CLUSTER_WARN_CAP`), `:188` (`MERGE_FLOOR`), `:196` (`UNKNOWN_SPEAKER_KEY`), `:323`
  (`compute_turns`), `:421` (global `compute`), `:471` (warn-only guard), `:554`
  (`merge_clusters_to_at_most`); `diarization/pipeline.rs:331` (`persist`), `:377`
  (`clear_meeting_speakers`), `:387` (`reapply`), `:453` (`restore_user_identities`), `:823-850`
  (auto-label), `:865` (`EVENT_COMPLETE`); `diarization/align.rs:25` (`LOCAL_SPEAKER_KEY`);
  `diarization/commands.rs:254,266,334,361,459` (voiceprint + segment-override + attendee-enroll
  commands); `database/repositories/transcript_speaker_overrides.rs:24,101` (`set`, `reapply`);
  `database/repositories/speaker.rs:197,243` (`get_speaker_embedding`, `get_identity_snapshots`);
  `database/repositories/voiceprints.rs:46,109,130,183,193` (`add_sample`, `centroid_for_person`,
  `all_centroids`, `delete_for_person`, `clear_all`); `people/enroll.rs:34,59,116,176`
  (`OWNER_PERSON_ID`, `decide_enrollment`, `enroll_voiceprint_for_speaker`); `people/commands.rs:169`
  (enroll trigger); `frontend/src/components/MeetingDetails/SegmentSpeakerMenu.tsx`,
  `VirtualizedTranscriptView.tsx`, `hooks/useSpeakers.ts`; migrations dir (latest
  `20260708000300_*`).
