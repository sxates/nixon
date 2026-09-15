# 0011 — Speaker diarization P3: live labels & cross-meeting identity

- **Status:** Partially done — live labels shipped (0.5.0); cross-meeting identity matcher + People entity not yet built (see 0013 / 0015).
- **Owner agent(s):** audio-engineer (lead: live diarization, embeddings) + rust-core-engineer
  (DB/IPC/matching) + frontend-engineer (live labels + identity-suggestion UI)
- **Roadmap phase:** Phase 3 — Speaker diarization, slice P3 (Tasks 10–11 of `specs/0010`)
- **Parent:** `specs/0010-speaker-diarization.md` (P1 shipped `v0.3.0`; P2 shipped `v0.4.0`)
- **Follow-on:** `specs/0012-people-and-role-weighted-summaries.md` (Task 12, split out — see Approach)
- **Decision record:** `docs/decisions/ADR-0006-live-diarization-approach.md` (drafted alongside)

## Context / Problem

P1 (offline, post-meeting diarization on the system channel; mic = `You`) and P2 (rename/merge,
calendar-attendee association, speaker-attributed summary) are **shipped**. Today the experience is:
you record, you stop, you press "Identify speakers", and ~30–60 s later the saved transcript gains
`You` / `Speaker 1` / `Speaker 2` labels that you can rename or map to a calendar attendee.

Two gaps remain against the granola.ai bar, and one quality concern overhangs both:

1. **No labels during the call.** The live transcript that scrolls while you record is still a flat,
   speaker-less wall of text (`transcript-update` carries `source: "Audio"` hardcoded — see
   `audio/transcription/worker.rs:211`). Live labels are the headline P3 feature (Task 10).
2. **Every meeting starts from zero.** `Speaker 2` in today's call has no relationship to `Speaker 2`
   in last week's call, even when it's the same person. We already persist a stable `email` identity
   key (P2) and reserve a per-speaker `embedding BLOB`, but **nothing is written to `embedding` yet**
   and nothing matches a new meeting's voices against prior ones (Task 11).
3. **Accuracy is "not entirely accurate" (the owner, observed on real P2 calls) and under-tested.** Live
   diarization *inherits* whatever accuracy the offline engine has, and cross-meeting matching is only
   as good as the embeddings we store. Shipping live labels on top of a shaky base would surface the
   shakiness in the worst possible place — in real time, visibly churning. **P3 must therefore treat
   accuracy as a first-class gate, not an afterthought** (see "Accuracy gate" below).

### Grounding (verified in this repo, 2026-06-25)

- **Offline pipeline is whole-file batch.** `diarization/pipeline.rs::run()` resolves `system.wav`
  (`audio::channel_writer::system_channel_wav`), decodes it, runs `SherpaDiarizer::diarize()` over the
  *entire* buffer, then aligns turns to transcript segments by max temporal overlap
  (`align_system_turns_to_segments`) and persists. This is inherently a post-meeting shape — sherpa's
  `Diarize::compute(samples, …)` (see `diarization/sherpa.rs`) clusters the full conversation at once.
- **The engine is behind a trait.** `diarization/mod.rs` defines `Diarizer::diarize(&self,
  samples_16k_mono, sample_rate) -> Result<Vec<SpeakerTurn>>`; `SherpaDiarizer` is the only impl.
  ADR-0005 deliberately kept this trait so `speakrs` can be swapped in for P3 "if accuracy/latency
  demands". P3 is exactly that decision point.
- **Per-channel audio already exists, live.** `audio/channel_writer.rs::ChannelWavWriter` streams
  `mic.wav` + `system.wav` at **16 kHz mono** during the call, tapped *pre-mix* in `pipeline.rs`,
  via `write_window()`. The same `write_window` feed can fan out to a live diarizer with zero new
  capture work — the clean, separated system channel is already flowing through memory.
- **The mic short-circuit is settled and applies live unchanged.** `align.rs` labels mic-channel
  speech `LOCAL_SPEAKER_KEY` ("local" → "You") without clustering; only the *system* channel needs a
  diarizer. P3 live diarization runs on the same system stream.
- **Segments already carry recording-relative time.** `TranscriptUpdate { audio_start_time,
  audio_end_time, duration, … }` (`worker.rs:36`) is the same timeline diarization aligns against.
  Adding a `speaker` field to this event is the only event-contract change live labels need.
- **DB is ready for cross-meeting identity but unused.** `speakers` table (migration
  `20260624000000_add_speakers_table.sql`) has `embedding BLOB` (nullable, **never written**) and
  `email TEXT` (migration `20260625000000_add_speaker_email.sql`, written by P2's
  `assign_speaker_to_attendee`). `SpeakersRepository::upsert(...)` does **not** take an embedding —
  the orchestration in `pipeline.rs::persist()` upserts `local`→"You" / `spk_N`→"Speaker N+1" with no
  vector. The embedding column is a reserved home; populating it is net-new P3 work.
- **sherpa already computes embeddings internally but discards them.** `SherpaDiarizer::diarize()`
  maps sherpa's `Segment { start, end, speaker: i32 }` to `SpeakerTurn` and throws away the per-cluster
  centroid embedding that clustering produced. We must surface a representative embedding per cluster —
  either via a sherpa-rs API that exposes centroids, or by re-running the embedding model over each
  cluster's pooled audio (the embedding model `3dspeaker_campplus_sv_en_voxceleb_16k.onnx` is already
  downloaded by `models.rs`). This is the central Task 11 implementation question (see Open questions).
- **Summary already consumes speaker labels.** `summary/processor.rs::build_speaker_attributed_
  transcript()` + `SPEAKER_ATTRIBUTION_INSTRUCTIONS` (added in P2 Task 9) prefix each line with the
  resolved display name. Role-weighting (Task 12) extends this; it does not need new plumbing into the
  summary, only richer per-speaker metadata — another reason Task 12 is separable.
- **Frontend already renders labels + identity affordances.** `components/VirtualizedTranscriptView.tsx`,
  `MeetingDetails/SpeakerLegend.tsx`, `hooks/useSpeakers.ts`, `hooks/useDiarization.ts`,
  `lib/speaker-colors.ts` exist from P1/P2. Live labels reuse the color/legend system; cross-meeting
  suggestions reuse the rename pick-list affordance.

### Recommendation up front: split Task 12 into its own spec ✅

`specs/0010` itself flags Task 12 ("persistent **People** entity … role-weighted summaries … **Likely
its own spec**"). It is a distinct concept (an app-wide People directory, a roles UI, summarization
weighting) with its own data model, its own UI surface, and its own llm-eval work — and it *depends on*
the embeddings and matching that Task 11 lands. Bundling it would make 0011 sprawl and couple a
shippable live-diarization release to a larger product feature. **This spec (0011) scopes Tasks 10–11
(live + cross-meeting identity); Task 12 lives in `specs/0012` as the follow-on.** 0011 lays the
foundation 0012 builds on (it writes `embedding`, defines the match algorithm, and keeps `email` as the
identity key) so 0012 is mostly UI + a `people` table + summarization weighting.

## Goals

- **Live speaker labels during recording.** As the live transcript scrolls, each segment shows a
  speaker (`You` / `Speaker 1` / …), updating in near-real-time, **without the offline pass having to
  run first** and **without competing destructively with live STT** (the latency budget is explicit).
- **Label stability.** A label shown against earlier text must not visibly churn as more audio arrives.
  We define and enforce a *stable-once-shown* contract (see Design → Label stability).
- **Live → offline continuity.** When recording stops, the existing offline pass (P1) runs and produces
  the *authoritative* labels; the live labels must reconcile to it without a jarring relabel of the
  whole transcript. Offline remains the source of truth for accuracy.
- **Cross-meeting identity.** Persist a representative voice embedding per diarized speaker, and when a
  new meeting is diarized, **suggest** names by matching its speakers' embeddings against prior
  meetings' embeddings and the `email` identity key ("Looks like Priya"). Suggestions, never silent
  auto-assignment.
- **100% on-device, embeddings never leave the machine.** Matching is local cosine similarity over
  locally-stored vectors; no embedding is ever sent to any LLM provider or network endpoint. This is a
  hard privacy invariant (`CLAUDE.md`).
- **Accuracy is measured, not assumed.** Establish a DER/attribution benchmark before building live, and
  gate the live build on the offline engine clearing an agreed bar (see Accuracy gate). Keep the
  `Diarizer` trait so `speakrs` can be adopted if sherpa can't clear it.
- **Live diarization off by default / opt-in**, gated on the same `diarization_enabled` setting (plus a
  separate live sub-toggle), consistent with P1's opt-in posture.

## Non-goals

- **Task 12 — People entity, roles, role-weighted summaries.** Deferred to `specs/0012`. 0011 only
  *writes the embeddings and the match* that 0012 consumes.
- **Real-time source separation / overlap resolution.** Same as P1: we label the dominant speaker per
  segment; clean separation of simultaneous talkers is out of scope.
- **A global voiceprint enrollment flow** ("record 10 s to register your voice"). Cross-meeting identity
  here is *opportunistic* (built from meetings you've already had), not an enrollment product. Explicit
  enrollment, if ever, is a 0012-or-later concern.
- **Re-diarizing the mic channel.** Mic = `You` stays a channel-level short-circuit, live and offline.
- **Windows/Linux, cloud diarization.** Excluded, per P1.
- **Identity matching across *different machines / users*.** Embeddings are local to this install's DB.

## Approach

### Task 10 — Live diarization: periodic incremental re-diarization on a sliding/growing window ✅

Three options were considered for "how do labels appear during recording":

| Option | What it is | Verdict |
|---|---|---|
| **A. True online/streaming diarization** | A streaming diarizer emits speaker turns sample-by-sample as audio arrives. | **Rejected for now.** sherpa-onnx's diarization API is *offline-only* (`Diarize::compute(full_buffer)` — confirmed in `sherpa.rs`); there is no streaming clustering. `speakrs` is also a batch pyannote pipeline. True online diarization (e.g. diart-style incremental clustering) has no mature Rust binding and is the highest-risk, lowest-accuracy path. |
| **B. Low-latency incremental re-clustering on a sliding window** | Keep only the last N s of system audio; re-cluster that window every few seconds; map this window's local cluster ids onto stable global ids via embedding match. | Viable but **assignment churn is severe** — a sliding window keeps re-discovering speakers and re-numbering them; stitching windows is exactly the hard online-diarization problem we're trying to avoid. |
| **C. Periodic re-diarization on the *growing* buffer** ✅ | Every T seconds, run the *existing offline `SherpaDiarizer`* over the system audio captured **so far** (the whole call to date), align only the *new* segments since the last pass, and keep already-shown labels stable via the stability layer below. | **Recommended.** Reuses the shipped, tested offline path verbatim — no new engine, no streaming clustering, no new model. Clustering over the whole-so-far buffer is as accurate as we can get mid-call (it's literally the offline algorithm, just run earlier and repeatedly). The cost is re-running clustering on a growing buffer; mitigated below. |

**Why C over B:** the whole reason P1 is accurate is that it clusters the *complete* conversation.
Option B throws that away for latency; option C keeps it and pays a recompute cost instead. Given the
accuracy concern the owner raised, **we should not trade accuracy for latency in the diarization step** —
labels can lag a few seconds; they must not be wrong-and-flickering.

**Concretely (C):**
- A new `diarization/live.rs` `LiveDiarizer` owns a ring/accumulating buffer of the **system** channel
  at 16 kHz, fed from the same `ChannelWavWriter::write_window` tap in `pipeline.rs` (fan-out, not a
  second capture). Mic-channel windows are *not* sent — mic = `You` short-circuits, live, with **zero
  latency** (a mic-dominant segment gets `You` the instant it's transcribed, independent of the diarizer).
- A background task wakes every **T = ~5–10 s** (Decision) and, only if ≥ ~3 s of *new* system speech
  has accrued and no pass is in flight, runs `SherpaDiarizer::diarize()` over the system audio so far on
  a `spawn_blocking` thread, **CPU, single-threaded** (as offline).
- After each pass, align the *new* transcript segments (those after the last aligned `audio_end_time`)
  to the fresh turns and emit their labels. Already-emitted segments are reconciled only through the
  stability layer (they don't visibly churn).
- **Authoritative reconciliation at stop:** the existing P1 offline pass still runs on the final
  `system.wav` and remains the source of truth. Because live used the *same engine on a prefix* of the
  same audio, the final relabel is usually a no-op or small; the UI applies it as one quiet update.

#### Label stability (the hard problem) — *stable-once-shown* with an embedding-anchored id map

Cluster ids from sherpa are **per-run and arbitrary** (`spk_0` this pass may be `spk_1` next pass).
Showing `Speaker 2` against a line and later flipping it to `Speaker 1` is the failure mode to prevent.

Rule: **a label, once shown for a segment, is never silently changed by a later live pass.** We achieve
this with a *stable speaker registry* for the live session that maps each pass's raw cluster ids onto
**session-stable keys** via the embedding centroid of each cluster:
- Maintain a list of session speakers, each with a running mean embedding and a stable key (`spk_0`,
  `spk_1`, … allocated in first-seen order).
- After a pass, for each raw cluster, take its centroid embedding and match it (cosine ≥ τ_session) to
  an existing session speaker; if matched, reuse that stable key; if not, allocate a new one.
- Newly-aligned segments get the stable key. Previously-shown segments keep their key. If a later pass
  *strongly* contradicts an earlier assignment (rare; e.g. a quiet early speaker gets split), we do
  **not** rewrite the live transcript — the **offline pass at stop** is what corrects it, atomically.
- This makes "label instability" a *bounded, deferred* problem: live labels are provisional but stable;
  the one authoritative correction happens once, at stop, not continuously on screen.

This is the single decision worth an ADR (ADR-0006): *periodic re-diarization on the growing buffer +
embedding-anchored stable id map + offline-pass-is-authoritative*, rather than streaming or sliding
windows.

#### CPU / latency contention with live STT

P1 deliberately ran offline so it wouldn't compete with Whisper/Parakeet for CPU/GPU. Live re-introduces
contention; we bound it:
- **Diarizer runs CPU-only** (`provider: "cpu"`, already set in `sherpa.rs`), never touching the
  Metal/CoreML path STT uses, so it competes for CPU cores but not the GPU.
- **Single in-flight pass, debounced.** Never start a pass while one is running or while STT is visibly
  backlogged; skip a tick rather than queue. Passes are `spawn_blocking` (off the async executor and off
  the UI thread), as the offline path already is.
- **Cost is bounded by call length, and that's acceptable.** Re-clustering a growing buffer is O(call
  length) per pass; for a 1-hour call a late pass clusters ~1 h of system audio every T s. If profiling
  shows this is too heavy late in long calls, the fallback (Decision) is to *cap the diarization window*
  to the last M minutes for live passes only (offline-at-stop still does the whole thing) — a pragmatic
  hybrid of C and B that keeps accuracy where it matters (recent, on-screen text). **Default: full
  buffer; measure before capping.**
- **Hard rule: live diarization must never starve STT.** Acceptance criteria include "live transcript
  latency with live diarization ON is within X% of OFF" (see Acceptance).

#### Event + frontend changes

- Extend `TranscriptUpdate` (`worker.rs:26`) with `pub speaker: Option<String>` (the *display name*,
  resolved, e.g. `You`/`Speaker 2`) — `None` until the first pass labels that segment. Keeping it the
  resolved name (not the raw key) means the frontend renders it with the existing label/color path and
  needs no extra lookup mid-call. (The persisted authoritative `speaker_key` is still written offline.)
- A new event `live-diarization-update { meeting_id, segments: [{ sequence_id, speaker }] }` carries
  *retroactive* labels for already-emitted segments that a later pass first resolved (the common case:
  the segment was emitted with `speaker: None`, the next pass labels it). The frontend patches those
  rows in place. This is additive; segments already labeled are not re-sent.
- Frontend: `VirtualizedTranscriptView` already renders a per-segment speaker label/color; live mode
  feeds it from the live events instead of the post-hoc `useSpeakers` fetch. The recording view (the
  live transcript panel) gains the same color/legend treatment the meeting-details view has.

### Task 11 — Cross-meeting identity: store per-speaker embedding, suggest names by cosine match ✅

**Persist a representative embedding per diarized speaker** (the reserved `speakers.embedding BLOB`):
- At diarization time (offline pass; and reused by live's stable registry), obtain a representative
  **L2-normalized f32 embedding** per *remote* cluster (`spk_N`). The local speaker (`You`/`local`) gets
  no embedding (it's the device owner, identified by channel, and we should not voiceprint the owner
  silently). Store as little-endian f32 bytes in the BLOB.
- **How to get the embedding** (the implementation choice, Open question): preferred is to extend
  `SherpaDiarizer` to expose the per-cluster centroid sherpa's clustering already computes; if sherpa-rs
  doesn't surface it, re-run the embedding model (`3dspeaker_campplus…onnx`, already loaded) over each
  cluster's concatenated speech (a few representative turns) and mean-pool. Either way the math lives in
  `diarization/embedding.rs` (new) and `SpeakersRepository::upsert` grows an `embedding: Option<&[u8]>`
  parameter.

**Match a new meeting's speakers against prior meetings + email:**
- New module `diarization/identity.rs`: given this meeting's `(spk_N, embedding)` set, query all prior
  `speakers` rows that have a non-null `embedding` **and** an `email` (i.e. a *named, identified* prior
  speaker — an unnamed `Speaker 3` from a past call is a poor suggestion source). For each current
  speaker, compute **cosine similarity** to each candidate; if the best match ≥ **τ_match** (Decision,
  start ~0.5 for CAM++ cosine; tune on the benchmark) and beats the runner-up by a margin (reject
  ambiguous matches), produce a suggestion `{ speaker_key, suggested_name, suggested_email, confidence }`.
- **Interaction with the P2 attendee pick-list** (the key design point): the existing
  `api_get_meeting_attendees` (`diarization/commands.rs:228`) already returns the linked calendar
  event's attendees + a 1:1 `suggestion`. Cross-meeting identity is a **second suggestion source** that
  *ranks the same pick-list*: when a meeting has a calendar event, prefer **email-keyed** matches
  (suggest the attendee whose email matches an embedding-matched prior speaker — strongest signal:
  voice + invite agree); when there's no event, fall back to a pure embedding suggestion ("Looks like
  Priya (from 3 prior meetings)"). The frontend shows it as a pre-filled, dismissible suggestion on the
  speaker row, identical affordance to P2 — never an auto-rename.
- **False-match handling:** suggestions are always *confirmable*, shown with their basis ("matched
  Priya's voice from 2 meetings" / "from calendar"). The user can reject; rejection is **not** persisted
  as a negative example in 0011 (keep it simple), but a rejected suggestion is not re-offered for that
  meeting. The margin test + τ_match guard against the most damaging case (confidently wrong).
- **Privacy guarantee:** all of the above is local cosine over local BLOBs. `identity.rs` makes **no**
  network calls and the embedding is **never** included in any text/JSON sent to an LLM provider (the
  summary path uses only display names). Add a unit-level assertion and a packet-capture verification
  step (see Verification).

### Accuracy gate (gates the Task 10 build; precedes it) ✅

The owner flagged P2 accuracy as "not entirely accurate." Live diarization would amplify that. **Before
building live, we run an accuracy spike** (audio-engineer, ~1–2 days) that:
1. Builds a small **labeled benchmark**: 3–5 real-ish multi-speaker system-channel clips with a
   ground-truth turn timeline (extend the `specs/0009` macOS `say` harness for synthetic + capture a
   couple of real 2–3 person Zoom calls), and computes **DER** and **segment-attribution accuracy** for
   the current sherpa config.
2. **Tunes sherpa first** (cheapest win): `DiarizeConfig.threshold` (currently `0.5`),
   `min_duration_on` (`0.3`), `min_duration_off` (`0.5`) in `sherpa.rs`, and Auto-vs-Fixed cluster
   count. These directly drive over/under-segmentation — the most likely cause of "not accurate."
3. **Only if tuned sherpa can't clear the bar**, evaluate **`speakrs`** (pure-Rust pyannote
   `community-1`, CoreML, ~7% DER reported) behind the existing `Diarizer` trait — the swap ADR-0005
   anticipated. This is a contained change (new `Diarizer` impl + dylib/BLAS packaging) precisely
   because of the trait.

**Recommendation: the gate is a real gate.** Live diarization is built *after* the offline engine clears
an agreed attribution bar (proposed ≥ 85% segment-attribution on the benchmark; Decision). This also
hardens P2's quality, which ships value even if live slips.

## Design

### Data model

No new tables for 0011. One additive, forward-only migration plus a repository signature change:

- **`speakers.embedding`** — already exists (reserved BLOB). Start **writing** it: representative
  L2-normalized f32 vector per *remote* speaker, little-endian bytes. `local`/`You` stays NULL.
- **New migration `…_add_speaker_embedding_meta.sql`** (forward-only) — add nullable
  `embedding_dim INTEGER` and `embedding_model TEXT` to `speakers` so a future embedding-model change
  doesn't make old vectors silently incomparable (cosine across different models is meaningless). The
  matcher only compares vectors with the same `embedding_model`. Cheap insurance; no behavior change for
  existing rows.
- **`SpeakersRepository::upsert`** (`database/repositories/speaker.rs`) grows
  `embedding: Option<&[u8]>, embedding_dim: Option<i64>, embedding_model: Option<&str>`. New read
  `get_identified_with_embeddings(pool) -> Vec<(meeting_id, speaker_key, display_name, email,
  embedding, embedding_model)>` for the matcher (filters to rows with both `embedding` and `email`).
- `transcripts.speaker` (the per-segment key) and the P2 join are unchanged; live just resolves names
  earlier.

### Diarization module deltas (`frontend/src-tauri/src/diarization/`)

| File | Change |
|---|---|
| `mod.rs` | Extend the `Diarizer` trait (or add a sibling) to optionally return per-cluster embeddings: e.g. `SpeakerTurn` stays, add `fn diarize_with_embeddings(...) -> Result<(Vec<SpeakerTurn>, HashMap<String, Vec<f32>>)>` with a default that calls `diarize` + empty map. Keeps `speakrs` swap cheap. |
| `sherpa.rs` | Surface per-cluster centroid embeddings (sherpa-rs API if available; else via `embedding.rs`). Expose the config knobs (`threshold`, `min_duration_on/off`) for the accuracy spike, sourced from settings. |
| `embedding.rs` (new) | Helpers: run the loaded embedding ONNX over pooled cluster audio if centroids aren't exposed; L2-normalize; cosine; f32↔LE-bytes (de)serialization for the BLOB. Pure + unit-tested. |
| `live.rs` (new) | `LiveDiarizer`: accumulating 16 kHz system buffer fed from the pipeline tap; periodic `spawn_blocking` pass over the growing buffer; the **session stable-id registry** (embedding-anchored) and label-stability logic; emits `live-diarization-update`. |
| `identity.rs` (new) | Cross-meeting matcher: cosine match of current speakers vs `get_identified_with_embeddings`, margin/threshold guard, suggestion struct; the email-keyed-vs-voice ranking that feeds the attendee pick-list. **No network.** |
| `pipeline.rs` | Offline pass also computes + persists embeddings (via the extended trait) and, after persisting speakers, runs `identity.rs` to attach name suggestions to the `diarization-complete` payload. |
| `commands.rs` | New: `api_start_live_diarization(meeting_id)` / `api_stop_live_diarization` (or auto-driven by the recording lifecycle); `api_get_speaker_suggestions(meeting_id) -> Vec<SpeakerSuggestion>`. |
| `settings.rs` | Add `live_diarization_enabled: bool` (default false) alongside `diarization_enabled`; optional `diarization_threshold` override for the tuning outcome. |

### Pipeline tap (`audio/pipeline.rs`, `audio/channel_writer.rs`)

`pipeline.rs` already calls `ChannelWavWriter::write_window(system_window)` during recording. Add an
optional fan-out: when live diarization is enabled, the same system window is also pushed into the
`LiveDiarizer`'s buffer (a `tokio::sync::mpsc` send or a shared `Mutex<Vec<f32>>`), tapped at the same
pre-mix point so it's the clean separated channel. Mic windows are not forwarded (mic = `You`). This is
additive and best-effort, matching `channel_writer`'s "never break the primary recording" contract.

### Tauri IPC (register in `frontend/src-tauri/src/lib.rs`)

Commands (frontend → Rust):
- `api_start_live_diarization(meeting_id)` / `api_stop_live_diarization()` — lifecycle for the live pass
  (or wired into the existing record start/stop in `audio/recording_commands.rs` so the frontend doesn't
  manage it). Gated on `diarization_enabled && live_diarization_enabled && models_present`.
- `api_get_speaker_suggestions(meeting_id) -> Vec<SpeakerSuggestion>` — cross-meeting + calendar name
  suggestions for the meeting's speakers (the ranked pick-list source).
- `api_get/set_live_diarization_enabled(bool)`.

Events (Rust → frontend):
- Extend `transcript-update` with `speaker: Option<String>` (resolved display name, live).
- `live-diarization-update { meeting_id, segments: [{ sequence_id, speaker }] }` — retroactive labels
  for already-emitted live segments.
- `diarization-complete` payload extended with `suggestions: Vec<SpeakerSuggestion>` so the
  meeting-details view can surface "Looks like Priya" right after the offline pass.

### UI (`frontend/src/`)

- **Live transcript panel** (recording view, `app/_components/TranscriptPanel.tsx` /
  `components/RecordingControls.tsx`): render per-segment speaker labels + colors live, fed by
  `transcript-update.speaker` and patched by `live-diarization-update`, reusing `lib/speaker-colors.ts`
  and the legend component. A subtle "labels are provisional, finalized when you stop" affordance.
- **Speaker suggestions** (`MeetingDetails/SpeakerLegend.tsx`, `hooks/useSpeakers.ts`): on the speaker
  row, show a dismissible suggestion chip ("Looks like Priya · from calendar" / "· matched 2 meetings")
  that fills the existing rename/assign control on click — same affordance as P2's attendee pick-list,
  just with a ranked default.
- **Settings** ("Speakers & diarization"): add a "Label speakers live while recording" sub-toggle (under
  the existing enable), noting it uses extra CPU.

## Tasks (ordered, phased — owner agents in bold)

**P3-0 — Accuracy gate (precedes the live build)**
1. [ ] **audio-engineer** — diarization benchmark + DER/attribution harness: extend `specs/0009`'s
   `say`-based fixtures into a multi-speaker ground-truth set under `frontend/src-tauri/tests/`; add a
   `cargo test` that reports attribution accuracy for the current `sherpa.rs` config. Capture 2–3 real
   Zoom clips for manual DER.
2. [ ] **audio-engineer** — tune `DiarizeConfig` (`threshold`, `min_duration_on/off`, Auto-vs-Fixed) in
   `sherpa.rs` against the benchmark; record the chosen values + accuracy. **Gate:** report whether
   tuned sherpa clears the agreed bar; if not, do the `speakrs` evaluation behind the `Diarizer` trait
   and recommend swap/no-swap.

**P3-A — Cross-meeting identity (Task 11; ships value on its own, on the offline path)**
3. [ ] **audio-engineer** — `diarization/embedding.rs`: representative per-cluster embedding extraction
   (centroid via sherpa-rs, or pooled re-embedding using the loaded ONNX), L2-normalize, cosine,
   BLOB (de)serialization. Extend the `Diarizer` trait (`mod.rs`) + `sherpa.rs` to surface embeddings.
4. [ ] **rust-core-engineer** — migration `…_add_speaker_embedding_meta.sql` (`embedding_dim`,
   `embedding_model`); extend `SpeakersRepository::upsert` to persist the embedding + meta; add
   `get_identified_with_embeddings`. Write embeddings from `pipeline.rs::persist` (remote speakers only).
5. [ ] **rust-core-engineer** — `diarization/identity.rs`: cosine match + margin/threshold guard;
   `api_get_speaker_suggestions`; rank calendar (email) vs voice suggestions; attach `suggestions` to
   `diarization-complete`. Unit test the no-network/no-leak invariant.
6. [ ] **frontend-engineer** — suggestion chips in `SpeakerLegend`/`useSpeakers` that pre-fill the
   existing rename/assign control; surface match basis; dismiss = not re-offered this meeting.

**P3-B — Live diarization (Task 10; built only after the gate clears)**
7. [ ] **audio-engineer** — `diarization/live.rs` `LiveDiarizer`: growing-buffer accumulation from the
   pipeline tap, periodic `spawn_blocking` re-diarization, embedding-anchored session stable-id registry
   + label-stability (stable-once-shown). Mic = `You` live short-circuit.
8. [ ] **audio-engineer** — pipeline tap fan-out in `audio/pipeline.rs` (system window → `LiveDiarizer`),
   gated, best-effort; wire start/stop into `audio/recording_commands.rs`.
9. [ ] **rust-core-engineer** — extend `TranscriptUpdate` with `speaker: Option<String>`; emit
   `live-diarization-update`; `api_start/stop_live_diarization`, `api_get/set_live_diarization_enabled`,
   settings field; register in `lib.rs`. Reconcile-at-stop: offline pass result applied as one update.
10. [ ] **frontend-engineer** — live speaker labels + colors in the recording transcript panel;
    retroactive patching from `live-diarization-update`; "provisional, finalized at stop" affordance;
    live sub-toggle in settings.

**Cross-cutting**
11. [ ] **spec-architect** — `docs/decisions/ADR-0006-live-diarization-approach.md` (this decision) +
    `specs/0012` for Task 12 (People + role-weighted summaries). Both drafted alongside this spec.

## Acceptance criteria

Testable; tie back to the Definition of Done in `/CLAUDE.md` (`cargo check`/`clippy` clean in
`frontend/src-tauri`; `pnpm lint` clean in `frontend`; app launches via `./clean_run.sh`;
record → live transcript → summary smoke unchanged).

- **Accuracy gate:** the benchmark `cargo test` runs without a mic and reports segment-attribution
  accuracy; the chosen engine/config clears the agreed bar (proposed ≥ 85% on the benchmark) and the
  number is recorded in the spec/ADR before P3-B starts.
- **Live labels appear:** in a recorded multi-person call with live diarization ON, speaker labels
  appear on live transcript segments within ~T+a few seconds of being spoken; `You` appears immediately
  for mic-dominant speech.
- **Label stability:** during a live session, a label shown for a segment is **never** observed to flip
  to a different speaker on screen (only `None → name` transitions occur live); the single authoritative
  relabel at stop is applied atomically.
- **No STT regression:** live transcript end-to-end latency with live diarization ON is within an agreed
  bound of OFF (proposed ≤ 15% slower) on a reference machine; `cargo test` STT fixtures
  (`specs/0009`) still pass.
- **Cross-meeting suggestion:** after diarizing a meeting that includes a previously-named speaker (same
  voice, prior `email`), `api_get_speaker_suggestions` returns that name with its basis; a wrong-but-
  plausible voice does **not** produce a confident suggestion (margin/threshold respected).
- **Embeddings persisted, owner excluded:** remote speakers get a non-null `embedding` +
  `embedding_model`; `local`/`You` stays NULL.
- **Privacy:** with diarization (live and offline) running, a packet capture shows zero egress except
  the one-time model download; a unit test asserts the summary payload and any IPC suggestion payload
  contain **no** embedding bytes.
- **Opt-in / no regression:** with live diarization disabled (default), behavior is identical to v0.4.0;
  the offline P1/P2 path is unchanged.

## Risks / open questions

- **Recompute cost late in long calls (Task 10).** Re-clustering a growing buffer every T s is O(call
  length). *Mitigation:* debounce + single in-flight + CPU-only; *fallback:* cap the live window to the
  last M minutes (offline-at-stop stays full). **Default: full buffer; profile before capping.**
- **Embedding extraction path (Task 11).** Whether sherpa-rs exposes per-cluster centroids or we must
  re-embed pooled audio is **unverified** — needs a short spike against `sherpa-rs 0.6.8`. The re-embed
  fallback is fully in our control (the ONNX model is loaded), so this is a "which path," not a blocker.
- **Match threshold & false positives (Task 11).** τ_match is engine/model-specific; a too-low threshold
  produces confident wrong suggestions (the worst UX). *Mitigation:* tune τ on the benchmark, require a
  runner-up margin, always make suggestions confirmable.
- **Embedding model stability.** If we ever change the embedding model, old BLOBs become incomparable —
  hence `embedding_model` on the row and matcher filtering by it. Old, unmatched speakers simply stop
  being suggestion sources (graceful).
- **Voiceprinting the owner.** We deliberately do **not** embed `You`/`local`. If a future feature wants
  "recognize me on a new machine," that's an explicit-enrollment 0012+ decision, not a silent default.
- **Two ONNX runtimes + CPU pressure live.** ADR-0005's static-ORT-1.22 + dynamic-ORT-1.17.1 coexistence
  was proven for the offline path; running diarization `compute()` *concurrently* with live STT in the
  same process is a new stress point — covered by the no-STT-regression acceptance criterion and a
  manual smoke.
- **speakrs adoption (if gate fails).** Pulls in BLAS + new dylib packaging (ADR-0005). Contained by the
  trait, but it's real work; the gate decides whether we pay it.

## Verification

- **Accuracy:** the new benchmark `cargo test` (mic-free, `say`-generated + a couple of real clips)
  reports attribution accuracy for the chosen config; record the number.
- **Live smoke:** record a real 2–3 person Zoom call with live diarization ON → confirm labels appear
  within a few seconds, `You` is immediate, and no on-screen label flips; stop → confirm the offline pass
  produces the final labels in one update.
- **Cross-meeting smoke:** name a speaker via calendar in meeting A (writes `email`+`embedding`); record
  meeting B with the same person → confirm "Looks like <name>" suggestion appears and one-click applies.
- **Privacy:** packet-capture during a live+offline run (models cached) shows zero egress; the unit test
  asserts no embedding bytes in summary/suggestion payloads.
- **Engine parity:** repeat the live smoke once with Whisper and once with Parakeet selected (same
  `transcript-update`/alignment path).
- Run `/check` (cargo check/clippy, pnpm lint, `./clean_run.sh`, record→live transcript→summary smoke).

## Sources

- Parent spec & decisions: `specs/0010-speaker-diarization.md` (Tasks 10–12; "Future enhancement (P3+)
  — persistent People & roles"); `docs/decisions/ADR-0005-diarization-engine.md` (sherpa-onnx; `Diarizer`
  trait retained for a `speakrs` swap).
- Implementation grounding (this repo): `frontend/src-tauri/src/diarization/{mod,sherpa,pipeline,align,
  models,settings,commands}.rs`; `frontend/src-tauri/src/audio/channel_writer.rs`;
  `frontend/src-tauri/src/audio/transcription/worker.rs` (`TranscriptUpdate`);
  `frontend/src-tauri/src/summary/processor.rs` (speaker-attributed transcript, P2 Task 9);
  migrations `20260624000000_add_speakers_table.sql`, `20260625000000_add_speaker_email.sql`.
- Calendar attendees (the pick-list this ranks): `specs/0008-calendar-zoom-integration.md`;
  `diarization/commands.rs::api_get_meeting_attendees`.
- Test-harness pattern for fixtures: `specs/0009-audio-test-harness.md`.
- sherpa-onnx speaker diarization (offline; fixed/auto count; threshold/min-duration knobs):
  https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/index.html
- speakrs (pure-Rust pyannote `community-1`, ONNX/CoreML, ~7% DER; the P3 accuracy-gate alternative,
  needs BLAS): https://github.com/avencera/speakrs
