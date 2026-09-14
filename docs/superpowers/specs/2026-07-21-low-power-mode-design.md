# Low Power Mode + Attendee-Removal Persistence — Design

**Date:** 2026-07-21
**Status:** Approved

## Problem

Real-time transcription during meetings meaningfully shortens MacBook battery life
(on top of Zoom's own drain). The user wants recording to continue on battery while
transcription and summary generation are deferred until back on AC power, with
per-meeting manual control in both directions.

Separately: attendees removed from the participants list *while recording* reappear
after the meeting ends, which also inflates the diarization speaker cap and hurts
accuracy.

## Decisions (from brainstorming)

- **Trigger:** low-power mode engages automatically on battery, disengages on AC.
- **Backlog:** when back on AC, the app *prompts* before processing deferred meetings.
- **Mid-meeting enable:** "go live + backfill" — live transcription starts at toggle
  time; the earlier span is transcribed and stitched at meeting end.
- **Global switch default:** on.

## 1. Power detection (new `power` module, Rust)

- IOKit power-source APIs (`IOPSGetProvidingPowerSourceType` +
  `IOPSNotificationCreateRunLoopSource`) — event-driven, no polling.
- Exposes current source (battery/AC) to backend consumers and emits a
  `power-source-changed` Tauri event to the frontend.
- macOS-only, matching the app.

## 2. Global switch

- New preference `low_power_on_battery: bool` (default **true**) in
  `RecordingPreferences` (`audio/recording_preferences.rs`, tauri-plugin-store).
- Effective low-power state = `low_power_on_battery && on_battery`.
- UI: toggle in Settings → Recordings ("Defer transcription & summaries while on
  battery"); indicator in the recording header when the current meeting is
  recording in deferred mode.
- Interaction with the existing global `live_transcription_enabled`: if that is
  already false, everything is deferred regardless; low-power mode only *adds*
  deferral when live transcription is otherwise on.

## 3. Per-meeting override

- Migration: add `processing_mode TEXT` to `meetings`
  (`NULL` = follow global, `'live'` = force full processing, `'defer'` = force defer).
- IPC to get/set; control in the recording header (and meeting details) to flip
  mid-meeting either direction.
- **Defer → Live mid-meeting:** load Whisper model, spawn the transcription worker
  (`audio/transcription/worker.rs`), attach VAD/STT to the running pipeline; live
  transcript starts from toggle time. At recording end the meeting is FULLY
  retranscribed via the existing replace path (`audio/retranscription.rs`
  `replace_meeting_transcripts` already does delete-and-insert), then diarization
  and summary run automatically even on battery (the override means "process this
  one now"). Full-replace supersedes range-stitching: the live segments serve the
  in-meeting display; the final transcript comes from one uniform pass.
- **Live → Defer mid-meeting:** the VAD/STT stage detaches (near-zero CPU); the
  engine stays resident but idle — resident memory is not the battery cost, CPU
  is, and unloading mid-recording risks lifecycle races. The partial transcript
  is kept for display; backlog processing later retranscribes the whole meeting.

## 4. Recording in low-power mode

Reuses the existing record-only path (`live_transcription_enabled = false` branch
in `audio/recording_commands.rs` / `recording_manager.rs` / `pipeline.rs`): audio
capture, mixing, per-channel WAVs, `audio.mp4`, checkpoints all unchanged; VAD/STT
and the live diarizer are skipped and the model never loads. The only new logic is
computing the effective mode at recording start (global switch + power source +
per-meeting override).

## 5. Backlog processing

- **Detection:** meetings with saved audio but missing/partial transcripts
  (reuse the sparse-transcript heuristic, plus partially-transcribed deferred
  meetings marked for stitching).
- **Trigger:** power returns to AC (debounced ~90 s, frontend-driven off the
  `power-source-changed` event) or app launch on AC. Never starts while a
  recording is live. The backend contributes two query commands
  (`api_get_power_state`, `api_list_deferred_meetings`); the debounce, prompt,
  and sequential processing live in the frontend — no backend watcher task.
- **Flow:** frontend prompt — "N meetings were recorded in low-power mode —
  process them now?" On confirm, process sequentially per meeting:
  transcribe → diarize → summarize, with progress UI, using existing machinery
  (`start_retranscription`, `api_diarize_meeting`, summary service). On decline,
  a badge remains and each meeting's existing "Transcribe now" affordance still
  works.
- The retention sweeper already exempts sparse-transcript meetings, which covers
  fully deferred recordings. Partially transcribed meetings (Live → Defer
  mid-meeting) can exceed the sparse threshold, so the sweep
  (`audio/retention.rs::decide_sweep`) must additionally exempt any meeting with
  pending deferred processing.

## 6. Attendee-removal fix (tombstone)

- Root cause: `api_get_meeting_participants` re-seeds calendar attendees on every
  roster read (`diarization/commands.rs:1175`), and removal is a hard `DELETE`
  with no memory of the user's intent — so removals are resurrected on the next
  read (meeting-details mount after recording, popover reopen, speaker events).
- Fix: migration adds `removed_at` to `meeting_participants`;
  `api_remove_meeting_participant` sets it instead of deleting. The seed's
  `INSERT OR IGNORE` then naturally respects the tombstone while still
  self-healing genuinely new attendees.
- All roster reads and the diarization speaker-count queries
  (`remote_person_ids`, `count_for_meeting` in `meeting_participant.rs`, feeding
  `roster_speaker_cap` in `diarization/pipeline.rs`) filter `removed_at IS NULL`,
  so removals actually lower the speaker cap as intended.
- Re-adding a removed person (via the add-participant UI) clears `removed_at`.

## 7. Testing

- Rust: unit tests for effective-mode decision logic, backlog detection query,
  tombstone repository behavior (remove → seed → still removed; re-add clears).
- Frontend: Vitest for new hooks/UI states (low-power indicator, override control,
  backlog prompt).
- Known pre-existing failure: the `vad_filter` adversarial-fixtures test on
  macOS 15.x — unrelated, stays as-is.

## Out of scope

- Windows/Linux power detection.
- Thermal or CPU-load-based throttling.
- Changing summary provider behavior or auto-summary policy beyond deferral.
