# 0010 — Speaker diarization (who-said-what, on-device)

- **Status:** Done — shipped (diarization P1/P2, 0.3.0–0.4.0).
- **Owner agent(s):** audio-engineer (lead) + rust-core-engineer (DB/IPC) + frontend-engineer (UI)
- **Roadmap phase:** Phase 3 — Speaker diarization

## Context / Problem
Vinyl transcribes a meeting into a flat, speaker-less wall of text. The reader can't tell who said
what, which makes both the live transcript and the generated summary noticeably worse than the
granola.ai quality bar — action items can't be attributed, and "Alice asked / Bob agreed" framing
is impossible.

**This is our flagship differentiator.** Upstream meetily has *no* real diarization: it only labels
audio by *source* ("mic" vs "system"), and full diarization is paywalled in their PRO tier. Doing it
**on-device** is squarely on-brand for Vinyl's privacy-first thesis (`CLAUDE.md`): audio never leaves
the machine, so the diarization models must run locally too.

### Grounding (verified in this repo)
- **The pipeline MIXES mic + system into one stream before transcription.** `audio/pipeline.rs`
  `AudioPipeline::run()` pulls per-device `AudioChunk`s into `AudioMixerRingBuffer`, mixes a window
  (`ProfessionalAudioMixer::mix_window`, ~line 826), runs VAD on the *mixed* signal, and sends the
  resulting `AudioChunk` to transcription with a placeholder `device_type: DeviceType::Microphone`
  (it's actually mixed — see the `// Mixed audio` comment at ~line 849). **Channel identity is
  destroyed before STT.** This is the single most important architectural fact for this spec.
- **The transcript-update event carries no real speaker.** `audio/transcription/worker.rs`
  `TranscriptUpdate` has `source: "Audio"` hardcoded and recording-relative timestamps
  (`audio_start_time`, `audio_end_time`, `duration`) that we can align diarization against.
- **A `speaker` column already exists but is dead.** Migration
  `20251110000001_add_speaker_field.sql` added `transcripts.speaker TEXT` ("'mic' | 'system'"), but
  the insert in `database/repositories/transcript.rs` (`insert_segments`, ~line 20) never writes it,
  the `Transcript` model (`database/models.rs`, ~line 40) doesn't read it, and `TranscriptSegment`
  (`api/api.rs`, ~line 195) has no speaker field. We can repurpose / extend this column.
- **We already ship ONNX Runtime.** `Cargo.toml` depends on `ort = "2.0.0-rc.10"` for the Parakeet
  engine (`parakeet_engine/model.rs` builds `ort` sessions via `commit_from_file`). Diarization
  models (pyannote segmentation, speaker-embedding) are ONNX — they run on the runtime we already
  bundle, no new inference engine required.
- **There is dead diarization reference code.** `audio/stt.rs` is orphaned screenpipe code (it
  `use`s a nonexistent `crate::pyannote::{embedding::EmbeddingExtractor, identify::EmbeddingManager}`
  and a `segmentation_model`; it's in **no** `mod` tree and does not compile into the app). It is
  *not* a usable foundation, but it confirms the intended shape: **VAD → segmentation → speaker
  embeddings → clustering**, with a `TranscriptionResult.speaker_embedding: Vec<f32>` per segment.
- **Mixed audio is saved to a WAV.** `audio/recording_saver.rs` finalizes one mixed recording under
  `~/Movies/meetily-recordings/` (`recording_preferences::get_default_recordings_folder`). A
  post-meeting pass can diarize from this file (or, better, from the separated streams — see
  Approach).
- **Two STT engines, one event contract.** `audio/transcription/engine.rs` dispatches
  Whisper (`whisper_engine/`) vs Parakeet (`parakeet_engine/`) vs trait `Provider`. Both ultimately
  emit the *same* `transcript-update` event with the same timestamp fields, so diarization integrates
  **once**, at the segment/timestamp layer, and works for both engines.
- **Model-download precedent exists.** Parakeet models load from a `model_dir` (ONNX files +
  `vocab.txt`); Whisper models download on demand. We follow the same on-demand pattern for the
  diarization models, located via Tauri path APIs (never hardcoded).

## Goals
- **Who-said-what labels** on the transcript: every saved transcript segment is attributed to a
  stable speaker label for the meeting (e.g. `You`, `Speaker 1`, `Speaker 2`).
- **100% on-device.** No audio or embeddings leave the machine; models are local ONNX.
- **Exploit the mic/system split we already have.** The local user is *already* identifiable — they
  are the microphone channel. Diarization's real job is separating the *remote* participants in the
  system-audio channel. Treat "mic = the known local speaker (`You`)" as a first-class shortcut.
- **Engine-agnostic:** works identically for the Whisper and Parakeet transcription paths.
- **Editable labels:** the user can rename a speaker ("Speaker 2" → "Priya") and have it stick for
  that meeting.
- **Off by default initially / opt-in**, with a clear settings toggle, so we can validate accuracy
  before making it the default (the owner decides — see Open decisions).
- **Feed speakers into the summary** so the notes-aware summary (`specs/0003`) can attribute points.

## Non-goals
- **Cross-meeting speaker identity / a global voiceprint database** ("this is always Priya across all
  meetings") — designed for later (P3), explicitly out of P1/P2.
- **Live/streaming diarization in P1** — P1 is post-meeting (offline) diarization; live is P3.
- **Separating overlapping simultaneous speakers within the *system* channel into clean streams**
  (source separation). We label the dominant speaker per segment; true overlap handling is best-effort.
- **Windows/Linux** — macOS-only, like the rest of the audio stack.
- **Cloud diarization providers** (Deepgram/AssemblyAI speaker labels) — violates the privacy thesis;
  not in scope even as a fallback.
- Reviving the orphaned `audio/stt.rs` / screenpipe `pyannote` path — we build fresh against sherpa.

## Approach

### Primary recommendation: **sherpa-onnx offline diarization (pyannote segmentation + speaker
embedding + clustering), run post-meeting, with the mic channel pre-assigned as `You`.** ✅

**Why sherpa-onnx.** It is the lowest-friction fit for *this* codebase:
- **Reuses our existing inference stack.** Its models are ONNX and run on `onnxruntime` — we already
  bundle `ort` for Parakeet. No second ML runtime, no `candle`, no Python.
- **Purpose-built diarization pipeline** (VAD/segmentation → embedding → clustering) with a **Rust
  API**, so we don't hand-roll spectral clustering. Supports **either a fixed `num_speakers` or an
  automatic clustering threshold**, and ingests **16 kHz mono** — exactly what our VAD stage already
  produces.
- **Small, permissively-licensed models we can download on demand:** pyannote `segmentation-3.0`
  ONNX ≈ **6 MB (MIT)**; a speaker-embedding model from sherpa's release set — **3D-Speaker
  (Apache-2.0, ~28 MB)** recommended, or wespeaker ResNet34 (CC-BY-4.0). Total footprint ~35 MB,
  comparable to a Whisper model.
- **Offline (batch) mode is the right first target.** We diarize *after* the meeting on the captured
  audio, where we have the whole conversation for stable clustering — far more accurate than online,
  and it never competes with real-time STT for CPU/GPU during the call.

**The architectural simplification that makes this tractable: diarize the SEPARATED streams, not the
mix.** Today the pipeline throws away channel identity before STT. We change that so diarization gets
the streams *unmixed*:
- **Microphone channel = the local user.** Any speech that is dominantly on the mic is `You` (the
  device owner). No clustering needed for the local speaker — we *know* who it is. (Edge case: people
  in the same physical room bleeding into the mic; acceptable to lump as `You`/local in v1, refine
  later.)
- **System channel = the remote participants.** Diarization (segmentation + embedding + clustering)
  runs **only on the system-audio stream**, where the genuinely-unknown speakers live. This both
  improves accuracy (no local-mic contamination to confuse clustering) and roughly halves the audio
  we must cluster.
- This means we must **persist (or re-derive) the per-channel audio**, not only the mix. Cheapest
  path: have `recording_saver` also write a `system.wav` (and `mic.wav`) alongside the mixed file
  during recording, OR diarize from in-memory per-channel buffers at stop. (Decision deferred — see
  Open decisions; writing two extra WAVs is simplest and also enables future re-diarization.)

**Alignment (the known hard problem).** Diarization yields *speaker turns* `(start, end, speaker)`;
STT yields *segments* `(audio_start_time, audio_end_time, text)`. We already have recording-relative
timestamps on both sides (the VAD pipeline timestamps every segment). Assignment rule:
**each transcript segment is labeled with the diarization turn whose time overlap with the segment is
greatest** (max temporal IoU / overlap-duration). Because we diarize per-channel, mic-channel
segments short-circuit to `You`; only system-channel segments consult the clustering result. Segments
with no confident overlap fall back to the channel-level label (`You` for mic, `Speaker ?` for
system). This is robust to the small boundary differences between VAD segmentation and pyannote
segmentation.

### Spike findings (2026-06-24) — VIABLE, coexistence proven
A feasibility spike (audio-engineer, isolated worktree) validated the approach with an actual build:
- **Crate:** `sherpa-rs` **0.6.8** (`thewh1teagle/sherpa-rs`; the de-facto Rust binding the k2-fsa repo
  now redirects to). Offline diarization is **always compiled in** (no feature flag): use
  `sherpa_rs::diarize::{Diarize, DiarizeConfig, Segment}`. `Segment { start: f32, end: f32, speaker: i32 }`
  (recording-relative seconds + cluster id) feeds `align.rs` directly. `compute(samples_16k_mono_f32,
  Option<progress_cb>)`; `DiarizeConfig { num_clusters: Option<i32>, threshold, min_duration_on/off,
  provider: Some("cpu"), .. }` (auto count = `num_clusters: None` + `threshold`).
- **onnxruntime coexistence: CLEARED.** A crate pinning `ort = "=2.0.0-rc.10"` **and** `sherpa-rs 0.6.8`
  compiled, linked, and ran in one process — no duplicate-symbol error. They link in different modes:
  `ort` rc.10 **statically** links ORT 1.22; `sherpa-rs-sys` **dynamically** links `libonnxruntime.1.17.1.dylib`
  + `libsherpa-onnx-c-api.dylib`. Both ORTs initialized in the same process at runtime.
- **Packaging caveat (the real P1 work item, not a blocker):** the two sherpa dylibs must be shipped in
  the `.app` with a correct rpath (`@executable_path/…`), mirroring the `ffmpeg`/`llama-helper` sidecar
  handling. Bare-binary launch fails with `dyld: Library not loaded: @rpath/libonnxruntime.1.17.1.dylib`;
  `cargo run` / `DYLD_LIBRARY_PATH` work. **Verify with `./build-gpu.sh` + a launched-app smoke**, not
  just `cargo build`. Also try `default-features = false` to drop sherpa-rs's `tts` default and slim the build.
- **Models (verified URLs):** segmentation `sherpa-onnx-pyannote-segmentation-3-0.tar.bz2` (~6.6 MB, MIT)
  from the `speaker-segmentation-models` release; embedding `3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx`
  (~28 MB, Apache-2.0) from the `speaker-recongition-models` release. ~35 MB total; download on demand.
- **Provider:** CPU for P1 (batch, post-meeting; don't compete with live STT). CoreML only if 1-hour-call
  latency is unacceptable.
- **Unverified, for P1:** real-model runtime smoke with a Parakeet session AND `compute()` live in the
  same process; DER accuracy on real Zoom system audio.

### Alternatives considered (briefly)
- **speakrs** (pure-Rust pyannote `community-1` pipeline; CoreML on Apple Silicon; ~7% DER at 529×
  realtime). Likely *higher accuracy* than sherpa and very fast, but it pulls in **BLAS (MKL/OpenBLAS)**
  and a heavier model set, and it's a younger single-maintainer crate. **Strong P3 upgrade candidate**
  if sherpa accuracy disappoints; we keep the diarizer behind a trait so swapping is cheap. Not
  primary for P1 because of the extra native-dep surface.
- **whisper-only "diarization" via channel attribution** (just label mic vs system, the meetily
  approach). Free and instant, but it can't separate the multiple *remote* people on the system
  channel — which is the whole point. We *do* use it as the mic-side shortcut and as a graceful
  fallback, but not as the headline feature.
- **pyannote via Python sidecar** — rejected: reintroduces a Python runtime dependency, which
  `CLAUDE.md` forbids.
- **Cloud diarization** — rejected on privacy grounds.

## Design

### Data model
Reuse and extend the existing (currently-dead) `transcripts.speaker` column rather than inventing a
new table. Two forward-only migrations (per `CLAUDE.md`, migrations are forward-only):

1. **Repurpose `transcripts.speaker` for a per-meeting speaker key** (the column already exists; no
   schema change needed there — we just start *writing* it). Values are stable per-meeting keys, e.g.
   `local` for the mic/local user and `spk_0`, `spk_1`, … for clustered remote speakers. Keep raw
   keys here (not display names) so renames don't require rewriting every row.

2. **New migration `…_add_speakers_table.sql`** — a `speakers` table mapping a per-meeting speaker key
   to an editable display name (and reserved fields for future cross-meeting identity):
   ```sql
   CREATE TABLE IF NOT EXISTS speakers (
       id           TEXT PRIMARY KEY,            -- "speaker-<uuid>"
       meeting_id   TEXT NOT NULL,
       speaker_key  TEXT NOT NULL,               -- matches transcripts.speaker ("local","spk_0",…)
       display_name TEXT NOT NULL,               -- "You","Speaker 1", user-renamed e.g. "Priya"
       is_local     INTEGER NOT NULL DEFAULT 0,  -- 1 for the mic/local user
       embedding    BLOB,                        -- reserved for P3 cross-meeting identity (nullable)
       created_at   TEXT NOT NULL,
       updated_at   TEXT NOT NULL,
       UNIQUE(meeting_id, speaker_key),
       FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
   );
   ```
   Rationale for a separate table over a `transcripts.speaker_name` column: a meeting has few speakers
   but many segments; renaming a speaker is one `UPDATE` here, not N updates across segments; and the
   `embedding` BLOB gives P3 cross-meeting identity a home without another migration.

3. **Surface `speaker` end-to-end:** add `pub speaker: Option<String>` (the key) to the `Transcript`
   model (`database/models.rs`) and to `TranscriptSegment` (`api/api.rs`); write it in
   `insert_segments` (`database/repositories/transcript.rs`). The API/IPC layer joins `transcripts` →
   `speakers` to return a resolved `display_name` to the frontend.

### Diarization module (`frontend/src-tauri/src/diarization/` — new)
- `mod.rs` — public `Diarizer` trait (`diarize(samples_16k_mono, sample_rate) -> Result<Vec<SpeakerTurn>>`,
  `anyhow::Result`) so sherpa can be swapped for speakrs later.
- `sherpa.rs` — sherpa-onnx-backed implementation (segmentation + embedding + clustering), `ort`-based.
- `models.rs` — on-demand download + cache of the two ONNX models into an app-data subdir resolved via
  **Tauri path APIs** (e.g. `app_data_dir()/models/diarization/`), mirroring the Parakeet/Whisper
  download pattern; verify checksums; emit progress events.
- `align.rs` — `SpeakerTurn`/`TranscriptSegment` overlap alignment (max-overlap rule above), plus the
  mic-channel `local` short-circuit and channel-fallback labels.
- `pipeline.rs` — orchestration: gather per-channel audio for the finished meeting → run system-channel
  diarization → derive `local` for mic-channel segments → align → produce `speaker_key` per segment →
  upsert `speakers` rows (default names `You` / `Speaker N`) → persist `transcripts.speaker`.

### Capturing per-channel audio for diarization
The pipeline currently mixes before STT. To diarize the **system** channel we need its audio retained.
Recommended (least invasive): in `audio/recording_saver.rs`, additionally finalize a `system.wav`
(and optionally `mic.wav`) next to the mixed WAV during recording. P1 diarizes from `system.wav` at
stop. (Alt: keep per-channel ring buffers in memory and hand them to the diarizer at stop — lower
disk use but no re-diarization later. See Open decisions.)

### Tauri IPC (register in `frontend/src-tauri/src/lib.rs`)
Commands (frontend → Rust):
- `diarize_meeting(meeting_id) -> DiarizationResult` — run/re-run offline diarization for a saved
  meeting; writes `transcripts.speaker` + `speakers` rows. Idempotent (re-run clears prior keys).
- `get_meeting_speakers(meeting_id) -> Vec<Speaker>` — speaker keys + display names for the meeting.
- `rename_speaker(meeting_id, speaker_key, display_name)` — `UPDATE speakers.display_name`.
- `merge_speakers(meeting_id, from_key, into_key)` — fix over-segmentation (P2); reassign segments.
- `get_diarization_settings() / set_diarization_settings(...)` — enabled, auto-run-on-stop,
  num_speakers (auto vs fixed), model choice.
- `download_diarization_models() -> ()` — explicit first-run model fetch (with progress events).

Events (Rust → frontend):
- `diarization-progress` `{ meeting_id, stage, pct }` — model download + diarization passes.
- `diarization-complete` `{ meeting_id, speaker_count }` — refresh the transcript view.
- (Live P3 only: extend `transcript-update` with a `speaker` field so labels appear in real time.)

### UI (`frontend/src/`)
- **Transcript view** (`components/MeetingDetails/TranscriptPanel.tsx` + `VirtualizedTranscriptView`,
  `usePaginatedTranscripts.ts`, `services/transcriptService.ts`, `types/index.ts`): render a speaker
  label/avatar/color per segment; group consecutive same-speaker segments. Add `speaker`/`speakerName`
  to `TranscriptSegmentData`.
- **Speaker rename UI:** click a speaker label → rename; a small "Speakers" legend on the meeting page
  for rename/merge (P2).
- **Settings:** a "Speakers & diarization" section — enable, run automatically after each meeting,
  fixed vs auto speaker count, "Download models" + status.
- **Summary integration (`specs/0003`):** include speaker attribution in the transcript text passed to
  the summary prompt so the model can attribute points/action items.

## Tasks (ordered, phased — see Phasing)

**P1 — Offline diarization on the system channel; mic = `You` (highest value/effort ratio)** ✅ DONE 2026-06-24 (`vinyl-v0.3.0`); live multi-person smoke is the user's manual test.
1. [x] **audio-engineer** — per-channel audio: `system.wav` + `mic.wav` (16k mono) written into the
   meeting folder, tapped pre-mix in `pipeline.rs`; resolvers in `audio/channel_writer.rs`.
2. [x] **audio-engineer** — `diarization/` module: `Diarizer` trait + `sherpa.rs` (sherpa-rs 0.6.8),
   `models.rs` on-demand download (Tauri paths), `align.rs` overlap alignment + `local` fallback.
   Plus dylib bundling + `disable-library-validation` entitlement (ADR-0005).
3. [x] **rust-core-engineer** — DB: `speakers` migration; `speaker` on `Transcript`/`TranscriptSegment`
   (LEFT JOIN for `speaker_name`); `speakers` repository.
4. [x] **rust-core-engineer** — `diarization/pipeline.rs` orchestration; `api_diarize_meeting`,
   `api_get_meeting_speakers`, `api_download_diarization_models`, `api_diarization_models_present`,
   `api_get/set_diarization_enabled` (opt-in, default OFF) + events; registered in `lib.rs`.
5. [x] **frontend-engineer** — speaker labels in `VirtualizedTranscriptView`; "Identify speakers"
   action + model-download UX (`useDiarization`); diarization settings toggle; gated auto-run on stop.

**P2 — Speaker labeling/renaming + attendee association + summary integration**
6. [ ] **rust-core-engineer** — `rename_speaker`, `merge_speakers` commands; join speakers into
   transcript reads so the frontend gets `display_name`. Migration: add nullable `email TEXT` to
   `speakers`.
7. [ ] **rust-core-engineer** — expose the linked calendar event's attendees (name + email) for a
   meeting (reuse `specs/0008` EventKit data); a `assign_speaker_to_attendee` command that sets
   `display_name` + `email` on a speaker row; auto-suggest the obvious 1:1 mapping.
8. [ ] **frontend-engineer** — rename/merge UI + speaker legend (colors/avatars per speaker); the
   rename control offers the meeting's **attendees as a pick-list** so `Speaker 1` → a real person
   in one click.
9. [ ] **llm-pipeline-engineer** — feed speaker-attributed transcript into the notes-aware summary
   (`summary/` + `specs/0003`); eval that attribution improves action-item ownership.

**P3 — Live diarization + cross-meeting identity + People/roles** → broken out into **`specs/0011`**
(Tasks 10–11) and **`specs/0012`** (Task 12); design + accuracy gate live there.
10. [ ] **audio-engineer** — streaming/online diarization (or low-latency incremental re-clustering)
    so labels appear during recording; extend `transcript-update` with `speaker`. Evaluate **speakrs**
    here if sherpa accuracy/latency is insufficient. → **`specs/0011`** (chosen: periodic re-diarization
    on the growing buffer + offline-pass-authoritative, `docs/decisions/ADR-0006`).
11. [ ] **audio-engineer + rust-core-engineer** — cross-meeting identity: store the per-speaker
    `embedding` (BLOB) + `email` and match against prior meetings/People to suggest names
    ("Looks like Priya"). → **`specs/0011`**.
12. [ ] **rust-core-engineer + llm-pipeline-engineer** — persistent **People** entity (keyed by email,
    optional voice embedding) with role/seniority attributes; weight speaker contributions by role in
    the notes-aware summary (e.g. CEO directives carry more signal). → **split into `specs/0012`**;
    schema pre-designed for it (`speakers.email`, `speakers.embedding`, a future `people` table).

**Cross-cutting**
13. [ ] **spec-architect** — ADR `docs/decisions/ADR-00NN-diarization-engine.md` (sherpa-onnx over
    speakrs/Python; per-channel diarization decision; model licensing). Write alongside P1.

## Acceptance criteria
Testable; tie back to the Definition of Done in `/CLAUDE.md`.
- **Attribution:** for a recorded multi-person Zoom call, ≥ ~80% of transcript segments are assigned
  to the correct speaker on a small hand-labeled fixture; the local user's segments are labeled `You`.
- **Speaker count:** a 3-person remote call (system channel) yields ~3 distinct remote speakers +
  `You` (off-by-one tolerated in P1; fixable via merge in P2).
- **Persistence:** `transcripts.speaker` and `speakers` rows are written; reopening the meeting shows
  the same labels; renaming a speaker persists and updates every segment's displayed name via the join.
- **Engine parity:** diarization labels appear correctly whether the meeting was transcribed with
  Whisper or Parakeet (same alignment path).
- **Privacy:** with diarization on, Vinyl makes **no** outbound network calls except the *one-time*
  model download from the configured host; verify with a network monitor.
- **Opt-in / no regression:** with diarization disabled, behavior is identical to today; the
  record → live transcript → summary smoke path is unchanged.
- **Gate (`/check`):** `cargo check` + `cargo clippy` clean (`frontend/src-tauri`); `pnpm lint` clean
  (`frontend`); app launches via `./clean_run.sh`; record → transcript → summary smoke still works.

## Risks / open questions
- **Per-channel audio retention.** Diarizing the system channel requires keeping it un-mixed. Adds
  disk (two extra WAVs) or memory (per-channel buffers). *Open:* WAV-on-disk (enables re-diarization,
  simpler) vs in-memory at stop (less disk). **Default lean: write `system.wav` + `mic.wav`.**
- **Alignment boundary drift.** pyannote turn boundaries won't match VAD segment boundaries exactly;
  the max-overlap rule mitigates this, but very short interjections may mis-attribute. Mitigation:
  per-channel diarization (mic short-circuit removes half the ambiguity) + merge UI (P2).
- **Over/under-clustering.** Auto threshold may split one person into two or merge two quiet speakers.
  Mitigation: expose fixed `num_speakers`, and `merge_speakers` (P2). *Open:* default auto vs prompt
  for expected count?
- **Mic bleed / same-room speakers.** Multiple people on one mic get lumped as `You` in v1.
  Acceptable for the all-day-1:1/Zoom user; note as a known limitation.
- **Model licensing/distribution.** segmentation-3.0 = MIT; 3D-Speaker = Apache-2.0; wespeaker =
  CC-BY-4.0 (attribution). *Open:* which embedding model is default, and do we bundle vs download?
  (Lean: **download on demand** to keep the app small, like Whisper/Parakeet.)
- **sherpa-onnx Rust binding maturity.** Confirm the Rust crate exposes offline diarization (not just
  ASR) and links against the same `onnxruntime` we ship; a short build spike is warranted before P1.
- **Performance.** Offline diarization on a 1-hour call must finish in a reasonable post-meeting
  window and not block the UI; run it in a background `tauri::async_runtime::spawn` task with progress.
- **Re-runs after edits.** Re-diarizing should not clobber user renames keyed to the same speaker; key
  stability across re-runs is non-trivial. *Open:* re-run preserves names by best-effort embedding
  match, or warns that renames reset?

## Decisions (resolved by the owner, 2026-06-24)
All recommendations accepted:
1. **Offline (post-meeting) diarization in P1**; live deferred to P3.
2. **Default off / opt-in** until accuracy is validated on real calls, then flip.
3. **Per-channel audio on disk** — write `system.wav` + `mic.wav` alongside the mixed recording.
4. **Embedding model 3D-Speaker (Apache-2.0), downloaded on demand** (~35 MB total with segmentation);
   not bundled.
5. **Auto speaker count** by default, with a manual "expected speakers" override in settings.
6. **Local user labeled `You`** (NOT substituted with the account name), remotes `Speaker 1..N`, all
   renamable.
7. **Merge/split speakers: yes, in P2.**
8. **sherpa-onnx for P1**, `Diarizer` trait retained so speakrs can be adopted in P3.

### Added scope (owner, 2026-06-24) — calendar-attendee → speaker association
Tie diarization to the calendar integration (`specs/0008`): when a meeting is linked to a calendar
event, we already have its **attendees (display name + email)** via EventKit. Use that roster to make
naming a *pick-list*, not free text:
- **P2:** in the rename UI, offer the linked event's attendees as suggestions so the user replaces
  `Speaker 1` with a real person in one click (stores the chosen `display_name` and the attendee
  **email** as a stable identity key on the `speakers` row). Add a nullable `email TEXT` column to the
  `speakers` table (forward-only migration) for this. If exactly one obvious mapping exists (e.g. a
  1:1 where the only remote attendee is the only remote speaker), pre-fill it.
- **Why it's powerful:** email is a stable cross-meeting key — far better than a raw voice embedding
  for *identity* — and it bridges "who was invited" with "who actually spoke."

### Future enhancement (P3+) — persistent People & roles
Promote per-meeting speakers to a first-class, app-wide **People** entity keyed by email (with an
optional voice `embedding` for recognition when there's no calendar event). A person carries
attributes useful to summarization — notably **role/seniority** (e.g. "CEO", "Eng lead"). The
notes-aware summary (`specs/0003`) can then **weight contributions by role** — decisions or directives
from a higher-authority speaker carry more signal in the summary and action-item ownership. This turns
diarization from "who spoke" into "whose input matters," and reuses the reserved `speakers.embedding`
BLOB + the new `email` key. Likely its own spec once P1/P2 land; captured here so the schema
(`email`, `embedding`, a future `people` table) is designed with it in mind.

## Verification
- **Fixtures:** generate a synthetic multi-speaker system-channel clip (e.g. concatenate distinct
  macOS `say` voices, as `specs/0009`'s harness does) with a known ground-truth turn timeline; add a
  `cargo test` that asserts the diarizer recovers the right speaker count and ≥ overlap-accuracy
  threshold, and that `align.rs` assigns segments correctly. Runs without a mic.
- **Manual smoke:** record a real 2–3 person Zoom call → run diarization → confirm `You` + distinct
  remote speakers in the transcript view; rename a speaker and confirm it persists across reopen.
- **Privacy check:** packet-capture during a diarization run (after models are cached) shows zero
  egress.
- **Engine parity:** repeat the smoke once with Whisper and once with Parakeet selected.
- Run `/check` (cargo check/clippy, pnpm lint, `./clean_run.sh`, record→transcript→summary smoke).

## Sources
- sherpa-onnx speaker diarization (segmentation + embedding + clustering; offline; fixed/auto speaker
  count; Rust API): https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/index.html and repo
  https://github.com/k2-fsa/sherpa-onnx (segmentation models:
  https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-segmentation-models ; embedding models:
  https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-recongition-models )
- pyannote `segmentation-3.0` (MIT; 10s/16kHz mono input; powerset speaker classes):
  https://huggingface.co/pyannote/segmentation-3.0 ; ONNX build (~6 MB):
  https://huggingface.co/onnx-community/pyannote-segmentation-3.0
- wespeaker embedding (CC-BY-4.0 via VoxCeleb):
  https://huggingface.co/pyannote/wespeaker-voxceleb-resnet34-LM
- speakrs (pure-Rust pyannote `community-1` pipeline; ONNX/CoreML; ~7% DER, 529× realtime on Apple
  Silicon; Apache-2.0; needs BLAS) — the P3 alternative: https://github.com/avencera/speakrs
