# 0037 — Resume / continue a recording

- **Status:** Implemented 2026-07-05 (automated gate green; `/code-review` correctness fixes
  applied — see BACKLOG "0037 code-review follow-ups" for deferred items; native manual smoke
  pending — see Verification)
- **Owner agent(s):** rust-core-engineer (start/stop reuse path, DB append, launch scan, IPC) + audio-engineer (segment audio + WAV concat for diarization) + frontend-engineer (relaunch prompt, Continue-recording action, resume start path)
- **Roadmap phase:** Later → graduates the BACKLOG item "Resume / continue an existing recording (crash recovery + keep-recording)" (reported 2026-07-05).

## Context / Problem

Today a recording can only ever be started *fresh* — the start command takes a meeting
*name* (never a `meeting_id`), lazily creates a new timestamped folder
(`create_meeting_folder`, `audio_processing.rs:35`), and the DB row is minted in parallel by
the frontend (`api_create_meeting`, `useRecordingStart.ts:203`). There is **no way to add to
a recording that has already stopped**, and two distinct real situations suffer:

1. **Crash / force-quit mid-meeting.** The app has no boot-time recovery. `metadata.json`
   carries `status: "recording"` (`recording_saver.rs:298`) that is never flipped to
   `"completed"` on an unclean exit, and `.checkpoints/audio_chunk_*.mp4` survive — but
   nothing reads them at launch. The only existing recovery (`useTranscriptRecovery.ts`) is
   IndexedDB-driven and fires only when the user happens to visit `/record`. So a crash can
   silently strand a half-captured meeting.
2. **Stopped too early.** If you stop, then the meeting keeps going (or you step away and
   come back), the only option is a brand-new recording = a **separate** meeting row + folder.
   You end up with two records for one conversation, split notes, and two partial summaries.

The owner hit this while testing 0036 and asked for it directly.

## Goals

- **Crash/quit recovery:** on app launch, detect an interrupted recording and **always prompt**
  ("Unfinished recording from <time> — Resume / Discard"). Never silently reopen the mic.
- **Continue recording:** an explicit action on an already-recorded meeting that resumes
  capture **into the same meeting** (same `meeting_id`, same folder, one continuous object).
- Both converge on one core capability: **start a recording that reuses an existing
  `meeting_id` + folder and appends** — transcripts, audio, and a diarization pass that covers
  the *whole* meeting, not just the new tail.
- Correctness first: no duplicate rows, no colliding audio timestamps, no speaker labels wiped
  for the earlier portion.

## Non-goals

- **No live-stitch of a single continuous WAV mid-stream.** Each recording session (initial +
  each resume) writes its own segment audio; segments are concatenated at final stop. (Simpler,
  and it matches how `.checkpoints` already concat at finalize.)
- **No cross-device / cross-machine resume.** Single machine, single install.
- **No auto-resume.** Always prompt (owner decision 2026-07-05).
- **No "resume a meeting recorded before this feature shipped"** guarantee — pre-0037 meetings
  lack the folder/meeting_id link written at start; they fall back to `folder_match` best-effort
  and may decline to resume. Acceptable.
- Not touching the live diarization path or the summary prompt — only ensuring they run over the
  combined set at stop.

## Approach

One shared mechanism — **"open a recording session against an existing meeting"** — serves both
features. The initial recording is just session #0; a resume is session #1+. Sessions live as
numbered **segments inside the meeting's existing folder**; at final stop we concatenate segment
audio into the meeting's `audio.mp4` and the per-channel `system.wav`/`mic.wav` into single
continuous files so the offline diarization pass (which full-recomputes over one `system.wav`)
sees the entire meeting. Transcript rows are **appended** to the same `meeting_id` with audio
timestamps offset by the meeting's prior total duration, so the transcript timeline is continuous
and stays aligned with the concatenated WAV.

Two enabling changes make resume robust (both also help recovery):

1. **Link the folder to the DB row at *start*, not just at stop.** Write `meeting_id` into
   `metadata.json` (the field already exists, `recording_saver.rs:282`, just unused) and set
   `meetings.folder_path` at start. A folder can then self-identify its meeting for both resume
   and boot recovery, instead of relying on timestamp/title `folder_match`.
2. **Allow additive transcript inserts.** The 0019 WS6.7 `count > 0` guard
   (`transcript.rs:148`) exists to stop a *race* from minting duplicate rows; resume is an
   *intentional* append. Add an explicit append path that bypasses the guard and offsets audio
   timestamps.

Why not the alternatives: a truly continuous single WAV would require rewinding the audio writers
mid-stream (they restart `checkpoint_count` at 0, `incremental_saver.rs:48`, and would overwrite
`audio_chunk_000`); segment-then-concat reuses the existing FFmpeg concat and is far less risky in
the crown-jewel audio path. Re-deriving diarization per-tail is impossible given the full-recompute
design (§4 of the lifecycle map), so concatenating to one WAV at stop is the natural fit.

## Design

### Data model

- **`metadata.json` (folder-local truth), extended** (`MeetingMetadata`, `recording_saver.rs:34`):
  - Actually write `meeting_id` at start (currently `None`).
  - Add `segments: [{ index, started_at, completed_at, duration_seconds, audio_file,
    system_wav, mic_wav }]`. Session #0 seeds `segments[0]`; each resume appends one. `status`
    stays `"recording"` until the final clean stop flips it to `"completed"`.
- **`meetings` table:** set `folder_path` at start (already the column used by diarization,
  `pipeline.rs:169`). No schema migration needed — reuse existing `folder_path`.
- **No new SQLite table.** The segment list is folder-local (metadata.json); the DB stays the
  meeting + its appended `transcripts` rows. (Revisit only if a query needs per-segment data.)

### Recording session (Rust)

- Thread `Option<String> meeting_id` + `Option<PathBuf> resume_folder` from the start commands
  (`start_recording_with_meeting_name` `recording_commands.rs:282`,
  `start_recording_with_devices_and_meeting` `:588`) down through
  `RecordingManager::start_recording` (`recording_manager.rs:104`) →
  `RecordingSaver::start_accumulation` / `initialize_meeting_folder` (`recording_saver.rs:161/256`).
  - When `resume_folder` is `Some`: **reuse** the folder (skip `create_meeting_folder`), read its
    metadata, allocate the next `segment.index`, and write segment audio under a fresh range
    (`.checkpoints/segNN_audio_chunk_*.mp4`, or seed `IncrementalAudioSaver.checkpoint_count`
    past existing) so nothing overwrites session #0's chunks.
  - Per-channel WAVs written as `system_segNN.wav` / `mic_segNN.wav` for the resumed session.
- Respect the single-session invariant: `IS_RECORDING` / `RECORDING_MANAGER`
  (`recording_commands.rs:36/39`) — resume still errors if a recording is already live.

### Stop / finalize with segments

- `RecordingSaver::stop_and_save` (`recording_saver.rs:409`): finalize this segment, then if the
  meeting has >1 segment, **FFmpeg-concat** all `segNN` audio into the meeting `audio.mp4` and all
  `system_segNN.wav` / `mic_segNN.wav` into single `system.wav` / `mic.wav`. Flip `status` to
  `"completed"`, write cumulative duration.
- DB append: new `TranscriptsRepository::append_transcripts_for_meeting` (or an `append: bool`
  param on `save_transcripts_for_meeting`, `transcript.rs:132`) that **skips the `count>0` guard**
  and offsets each new segment's `audio_start_time`/`audio_end_time` by the meeting's current
  `MAX(audio_end_time)`. Frontend stop path (`useRecordingStop.ts:391`) passes `append=true` when
  the session resumed an existing meeting.
- Diarization (`api_diarize_meeting`) runs as today over the now-continuous `system.wav` +
  full DB segment set — no change needed beyond ensuring the concat happened first.

### Launch recovery scan (Rust)

- New `scan_interrupted_recordings()` — scans the recordings root for folders whose
  `metadata.json.status == "recording"`, resolves `meeting_id` (now written at start; fallback
  `folder_match::best_folder_match`, `folder_match.rs:121`), returns
  `[{ meeting_id, folder_path, meeting_name, started_at, segment_count }]`.
- Called from a new command `api_list_interrupted_recordings`; discard via
  `api_discard_interrupted_recording` (flip metadata `status`, or finalize-and-keep the partial
  — see open questions).

### Tauri IPC

- `api_list_interrupted_recordings() -> Vec<InterruptedRecording>` — boot recovery.
- `api_discard_interrupted_recording(meetingId)` — dismiss without resuming.
- Extend start commands to accept optional `meetingId` + `resumeFolderPath` (backward-compatible
  Option params → missing = fresh recording, today's behavior).
- Reuse existing `recording-stopped` event (already carries `folder_path`).

### UI (frontend/src)

- **Relaunch prompt:** a launch-level check (in a provider mounted by `app/layout.tsx`, not
  `/record`) calls `api_list_interrupted_recordings`; if non-empty, show a modal —
  "Unfinished recording — <title> · started <time> — [Resume recording] [Discard]". Resume →
  `/record` in resume mode.
- **Continue recording:** a "Continue recording" button on `meeting-details` for a recorded
  meeting (near Summarize/Identify), → `/record` in resume mode for that `meeting_id`.
- **Resume start path:** `useRecordingStart.ts` gains a resume mode that **skips
  `api_create_meeting`** (`:203`), adopts the existing `meeting_id` (`setActiveRecordingMeetingId`),
  and passes `{ meetingId, resumeFolderPath }` through `recordingService.startRecordingWithDevices`
  (`recordingService.ts:69`). `meetingCreatedRef`/`createdMeetingIdRef` point at the resumed id.

## Tasks

1. [ ] **Metadata link + segments** — write `meeting_id` into `metadata.json` at start; set
   `meetings.folder_path` at start; add `segments[]` to `MeetingMetadata`. (rust-core + audio)
2. [ ] **Reuse-folder start path** — thread `meeting_id`/`resume_folder` through start command →
   manager → saver; segment-scoped checkpoint + WAV filenames; no overwrite of prior chunks.
   (audio-engineer)
3. [ ] **Segment concat at stop** — FFmpeg-concat segment audio → `audio.mp4` and per-channel
   WAVs → `system.wav`/`mic.wav`; cumulative duration; flip `status`. (audio-engineer)
4. [ ] **DB append** — `append_transcripts_for_meeting` bypassing the WS6.7 guard with audio-time
   offset; unit test append preserves ordering + offsets. (rust-core)
5. [ ] **Launch scan + IPC** — `scan_interrupted_recordings`, `api_list_interrupted_recordings`,
   `api_discard_interrupted_recording`; register in `lib.rs`. (rust-core)
6. [ ] **Resume start (frontend)** — resume mode in `useRecordingStart`/`recordingService` that
   skips create and adopts the existing id. (frontend)
7. [ ] **Relaunch prompt UI** — launch-level interrupted-recording modal. (frontend)
8. [ ] **Continue-recording action** — button on `meeting-details`. (frontend)
9. [ ] **Tests + gate** — Rust append/offset + scan unit tests; frontend resume-path + prompt
   tests; `/check`.

## Acceptance criteria

- Recording → stop → **Continue recording** → stop again yields **one** meeting whose transcript
  spans both sessions in order, whose audio plays end-to-end, and whose diarization labels cover
  the whole meeting (earlier speakers not wiped). No second `meetings` row.
- Kill the app mid-recording → relaunch → prompt appears with the right title/time → **Resume** →
  continue → stop → same single-meeting outcome. **Discard** → no resume, partial handled per
  decision.
- A normal (non-resumed) recording is byte-for-byte behavior-identical to today (Option params
  absent). Regression-guarded by existing `useRecordingStart` tests + `db_lifecycle`.
- `cargo check`/`clippy`/`test` and `pnpm lint`/`test` clean (Definition of Done, `/CLAUDE.md`).

## Risks / open questions

- **Audio-timestamp offset correctness** is the crux — the concatenated WAV timeline must match
  the offset transcript times or diarization mis-aligns. Cover with a fixture test.
- **Checkpoint collision** (`IncrementalAudioSaver.checkpoint_count` restarts at 0,
  `incremental_saver.rs:48`) — segment-scoped filenames or a seeded counter; verify no overwrite.
- **Discard semantics:** discard = throw away the partial, or finalize-and-keep it as a normal
  (shorter) recording? Leaning **finalize-and-keep** (never lose captured audio); confirm.
- **Time-gap cutoff:** should a very old interrupted folder still prompt, or auto-expire? Leaning
  always-prompt (owner chose always-prompt) with a "Discard" escape; revisit if noisy.
- **0036 interaction:** a resumed occurrence must remain the same meeting object — resume reuses
  the existing `meeting_id`, so scheduled-row adoption is untouched. Verify no new scheduled row is
  minted on resume.
- Live diarization during a resumed session is per-session; the authoritative offline pass at stop
  reconciles — same contract as today.

## Verification

- **Rust:** `cd frontend/src-tauri && cargo test --features metal` for the new append/offset +
  scan unit tests; existing `--test db_lifecycle` regression.
- **Frontend:** `pnpm test` for resume-path + relaunch-prompt suites; `pnpm lint`.
- **Manual smoke (native, owner machine):** (1) record 20s → stop → Continue recording → 20s →
  stop; open the meeting: one transcript spanning both, audio plays through, speakers labeled
  across both halves. (2) record 20s → force-quit (Activity Monitor) → relaunch → prompt → Resume
  → 20s → stop; same single-meeting outcome. (3) Discard path. (4) confirm a plain record→stop is
  unchanged.
