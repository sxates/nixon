/**
 * specs/0075 W4 — the recovery dialog must never offer the meeting being recorded.
 *
 * The recoverable list comes from IndexedDB, keyed `meeting-<ms>` (minted by
 * TranscriptContext on `recording-started`). Nothing else in the app shares that key: the
 * sidebar's `activeRecordingMeetingId` is the SQLite row id and is lost on a reload, and
 * `get_recording_state` carries no meeting id at all. So the live meeting is recognised by
 * what survives a webview reload, and only while the backend says a recording is live:
 *   1. its IndexedDB id — TranscriptContext keeps it in sessionStorage
 *      (`indexeddb_current_meeting_id`), which survives a reload of the same window;
 *   2. its recording folder — `get_meeting_folder_path` (Some only while the backend's
 *      RecordingManager exists, i.e. between start and stop), matched against the
 *      IndexedDB row's `folderPath`;
 *   3. its start time — the backend's `recording_duration` (wall time since start,
 *      pauses included) puts the live start at `now - duration`; a row whose `startTime`
 *      is within START_TOLERANCE_MS of that is the live one (covers the window before the
 *      folder path has been written to the row).
 */
import { invoke } from '@tauri-apps/api/core';

export const LIVE_MEETING_SESSION_KEY = 'indexeddb_current_meeting_id';
const START_TOLERANCE_MS = 10_000;

export interface LiveRecording {
  indexedDbId: string | null;
  folderPath: string | null;
  startedAtMs: number | null;
}

interface RecordingStateReply {
  is_recording?: boolean;
  recording_duration?: number | null;
}

/** The live recording's identifiers, or null when the backend reports nothing recording. */
export async function fetchLiveRecording(): Promise<LiveRecording | null> {
  let state: RecordingStateReply;
  try {
    state = await invoke<RecordingStateReply>('get_recording_state');
  } catch {
    return null;
  }
  if (!state?.is_recording) return null;

  let folderPath: string | null = null;
  try {
    folderPath = (await invoke<string | null>('get_meeting_folder_path')) ?? null;
  } catch {
    folderPath = null;
  }

  let indexedDbId: string | null = null;
  try {
    indexedDbId = sessionStorage.getItem(LIVE_MEETING_SESSION_KEY);
  } catch {
    indexedDbId = null;
  }

  const duration = state.recording_duration;
  const startedAtMs =
    typeof duration === 'number' && Number.isFinite(duration) ? Date.now() - duration * 1000 : null;

  return { indexedDbId, folderPath, startedAtMs };
}

export function isLiveMeeting(
  meeting: { meetingId: string; folderPath?: string; startTime?: number },
  live: LiveRecording | null,
): boolean {
  if (!live) return false;
  if (live.indexedDbId && meeting.meetingId === live.indexedDbId) return true;
  if (live.folderPath && meeting.folderPath && meeting.folderPath === live.folderPath) return true;
  if (
    live.startedAtMs !== null &&
    typeof meeting.startTime === 'number' &&
    Math.abs(meeting.startTime - live.startedAtMs) <= START_TOLERANCE_MS
  ) {
    return true;
  }
  return false;
}
