# ADR-0007 — Biometric voiceprint storage: local-only, opt-in for others, conservative confidence tiers

- **Status:** Accepted (Brian, 2026-06-27) — with the per-person opt-out refinement in Decision #2
- **Date:** 2026-06-26 (accepted 2026-06-27)
- **Related:** `specs/0013-identity-and-organization-program.md` (the "biometric ADR" gate on Wave 1c;
  voiceprint gallery, confidence tiers, "forget this person"), `specs/0011-diarization-p3-live-and-cross-meeting-identity.md`
  (`diarization/embedding.rs` built; `identity.rs` matcher net-new), `specs/0012-people-and-role-weighted-summaries.md`
  (the `people` entity), ADR-0005 (sherpa-onnx engine; CAM++ embedding), ADR-0006 (live diarization)

## Context

Cross-meeting voiceprints are 0013's keystone: recognizing a returning voice ("Looks like Priya")
across meetings turns each confirmation into a flywheel that improves auto-labeling. The mechanism is
already partly built — `diarization/embedding.rs` extracts an L2-normalized CAM++ embedding per
diarized cluster, and `speakers.embedding`/`speakers.email` are reserved columns. What's new in
**Wave 1c** is *durable, per-person persistence*: a voiceprint **gallery** (multiple samples per
person) that survives across meetings.

That persistence is the legally sensitive step. A voiceprint (speaker embedding) of an *identifiable
person* is **biometric data**: special-category under GDPR Art. 9 and a regulated "biometric
identifier" under Illinois **BIPA** and similar state laws (which attach notice/consent and retention
duties specifically to *storing* such data). Per-meeting embeddings (the status quo, ADR-0005/0006)
are transient working data; a durable per-person gallery is the regulated artifact. 0013 makes an
approved ADR a **hard gate** on building 1c, distinct from the matcher (1a) and the People entity
(1b), which add no biometric *storage* beyond what 0011 already reserved.

The tension is real: cross-meeting recognition is also Vinyl's strongest privacy story. The same
local-only design that makes the feature legally defensible is the moat — nothing comparable exists
that keeps voiceprints off the cloud. This ADR records how we get the feature *and* the posture.

## Decision

1. **Local-only storage, no egress — ever.** Voiceprint BLOBs live solely in the local SQLite DB
   (`voiceprints` table, keyed to `people`). They are **never** sent to any LLM provider or network
   endpoint. This extends the existing summary-egress invariant (the summary path uses display names
   only, never embeddings — `specs/0011`) and is the same guarantee already enforced for per-meeting
   embeddings. No cross-device/cloud sync of voiceprints is in scope.

2. **Explicit opt-in to store *others'* voiceprints, plus a per-person opt-out.** Two layers of
   consent control, because consent is per-individual, not just a global switch:
   - **Global (off by default):** persisting a voiceprint for any person who is **not the device
     owner** requires an explicit, off-by-default consent toggle. With it off, Wave 1a suggestions
     still work *within* a meeting and the People entity (1b) still maps names — but no durable
     `voiceprints` row is written for other people, and cross-meeting auto-ID simply doesn't accrue.
   - **Per-person (Brian, 2026-06-27):** every `people` row carries a **`voiceprint_opt_out`** flag.
     When set, Vinyl **never builds or stores a voiceprint for that individual** (and any existing
     samples for them are deleted), regardless of the global toggle — for consent reasons or because
     the user simply doesn't want that person's voice modeled. **Crucially, identity is decoupled from
     the voiceprint:** an opted-out person is still a first-class `people` row. The user can still
     **associate that person with detected speakers in transcripts** (manual name mapping, calendar
     mapping, role-weighting, action-item ownership all work) — they just don't get voice-based
     auto-ID. This is the whole point of "voiceprint is signal, not identity" (#4): you can be a known
     person with **no** stored voice. Enrollment is gated by `global_opt_in && !person.voiceprint_opt_out`.

   Opt-in is the gate, not a nag; opt-out is an always-available per-person override.

3. **Self-enrollment (the device owner = mic channel) on by default, opt-out available.** The mic
   channel is, by construction, the owner ("You"). Storing the owner's own voiceprint is the
   one zero-effort, highest-quality, consent-trivial case (consenting to store one's own biometric is
   not the regulated risk). It is **on by default with a clear opt-out**. Note this is a deliberate,
   recorded *change* from `specs/0011`'s earlier instinct to never voiceprint the owner: the owner is
   the safest subject and self-ID improves "You" robustness across devices/headsets.

   - **Implementation note — the owner "You" person.** The owner is represented by a **singleton reserved
     `people` row** with the fixed id **`person-owner-self`** (display name "You", no email), created
     lazily on first self-enroll (`people/enroll.rs::ensure_owner_person`). A fixed well-known id (rather
     than an `is_current_user` column) keeps the owner identifiable without a migration; self-enrolled
     voiceprints are stored under this person. The local/mic speaker key (`speakers.is_local = 1`) routes
     to this person regardless of which `person_id` the assign call passed.

4. **A voiceprint gallery keyed to the `people` entity; voiceprint is signal, not identity.** Identity
   is the `people` row, anchored by **email** (from calendar-attendee mapping; `speakers.email`
   exists). Voiceprints are *matching-signal* rows attached to a person, **best-N samples** per person
   (each tagged `embedding_model`, `source_meeting_id`, `sample_quality`), with the match **centroid
   computed on read**. Best-N (not a single stored centroid) is robust to voice drift and lets the
   gallery improve as samples accrue. The matcher only compares vectors sharing the same
   `embedding_model` (cosine across models is meaningless).

5. **Conservative confidence tiers — no silent false-accept.** Auto-labeling is gated by cosine to the
   gallery centroid *plus a runner-up margin*: **high → auto-label**, **medium → suggest-and-confirm**
   ("Is this Priya?"), **low → `Speaker N` + manual**. Thresholds are biased so a false-accept
   (mislabeling person A as person B) is strictly worse than asking. A label, once shown, is never
   silently rewritten to a *different person* by a later pass (consistent with ADR-0006
   stable-once-shown).

   - **Refinement (Brian, 2026-06-27) — email corroboration required for auto-label.** A high-confidence
     *voice* match is **not sufficient on its own** to auto-apply a label. Auto-label fires **only** when
     a high-confidence **gallery** match (cosine `≥ TAU_AUTO_LABEL`, currently `0.7`, with the runner-up
     margin) is **corroborated by a calendar-attendee email** that agrees with the matched person's email.
     A voice-only high match — however high the cosine — produces a **suggest-and-confirm**, never an
     auto-label. So the effective tiers are: **(high voice + agreeing email) → auto-label**; **(high voice
     alone, or medium) → suggest-and-confirm**; **(low) → `Speaker N`**. This is implemented in
     `diarization/identity.rs` (`auto_label` on `SpeakerSuggestion`; the offline pass applies only the
     `auto_label` ones); the on-demand `api_get_speaker_suggestions` re-fetch passes no corroborating
     emails, so it only ever suggests.

6. **Required controls (shippable prerequisites, not polish).** (a) **"Forget this person"** — deletes
   the `people` row and **cascades** their `voiceprints`. (b) **"Clear all voiceprints"** — wipes every
   stored voiceprint (gallery reset) while leaving People/identity metadata intact if the user chooses.
   (c) **"Don't store this person's voice"** — the per-person `voiceprint_opt_out` toggle (#2); setting
   it deletes any existing voiceprints for that person and blocks future enrollment, **without** deleting
   the person. These satisfy BIPA/GDPR deletion/retention expectations and must exist before 1c ships.

## Consequences / risks

- **Model caveat — English/VoxCeleb (CAM++).** The embedding model
  (`3dspeaker_campplus_sv_en_voxceleb_16k`, ADR-0005) is English/VoxCeleb-trained; accuracy degrades
  for non-English speakers and is the dominant error source. Conservative tiers (#5) contain the
  user-visible impact; a model swap stays behind the `Diarizer` trait. The `embedding_model` tag (#4)
  means a future swap invalidates comparisons cleanly rather than silently mismatching.
- **Voice drift & short utterances.** Voices drift (illness, headset, time); very short turns yield
  weak embeddings. Best-N galleries mitigate drift; sub-threshold/short utterances stay `Speaker N`
  rather than risk a false-accept. `sample_quality` lets weak samples be deprioritized.
- **Threshold tuning is empirical.** τ_match and the runner-up margin (start ~0.5 cosine for CAM++,
  per 0011) must be tuned on the diarization benchmark; ship conservative and loosen only with data.
- **Legal posture.** Local-only + opt-in-for-others + self-default-with-opt-out + forget/clear is a
  defensible BIPA/GDPR stance for a local-first tool, but this ADR is **not legal advice**; the
  consent-copy wording should get a legal read before GA marketing claims.
- **Scope of the gate.** This ADR gates **Wave 1c only**. Wave 1a (the cosine matcher over
  *already-reserved* per-meeting embeddings) and 1b (the People entity) introduce **no new biometric
  storage** and are not blocked by it.

## Verification posture

- **Packet-capture proof of no egress** (per the `specs/0011`/`0008` precedent): during a run that
  enrolls and matches voiceprints (models cached), a capture shows zero outbound traffic carrying
  embedding data; the only sanctioned egress remains the user-chosen LLM summary call, which carries
  display names only.
- **Unit-level egress assertion:** the summary serialization path asserts no `voiceprints`/embedding
  field is ever included in LLM-bound JSON.
- **Controls test:** "forget this person" cascade-deletes `voiceprints`; "clear all" empties the table
  — both verified by DB assertion.

## Alternatives considered

- **No persistence (status quo).** Every meeting starts from zero; no flywheel, no cross-meeting
  recognition — i.e. the keystone feature doesn't exist. Rejected as the *product* choice, but it
  remains exactly the behavior when the others-opt-in toggle is off.
- **Single maintained centroid per person (vs. best-N).** Simpler storage, but brittle to drift and
  discards the per-sample provenance/quality that makes deletion and tuning auditable. Rejected;
  best-N + centroid-on-read chosen.
- **On-by-default storage of others' voiceprints.** Maximizes the flywheel but stores other people's
  biometrics without explicit consent — the precise BIPA/GDPR risk. Rejected; opt-in is mandatory.
