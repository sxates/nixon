/**
 * Shared safety gate for auto-deleting a meeting row the recording flow believes is
 * disposable (specs/0037 review-2, extracted from useRecordingStop's abandoned-recording
 * cleanup so the failed-start orphan cleanup in useRecordingStart runs the SAME checks).
 *
 * `canDiscardMeeting` answers "is it provably safe to delete this row?": true ONLY when
 * the meeting has no typed notes AND the backend confirms there is nothing durable to
 * keep (no transcripts, no on-disk audio, not calendar-linked —
 * `api_recording_is_safe_to_discard`). On ANY error it answers false: when in doubt,
 * keep the meeting — typed notes must never be destroyed by a cleanup heuristic.
 */

import { invoke } from '@tauri-apps/api/core';

export async function canDiscardMeeting(
  meetingId: string,
  folderPath: string | null,
): Promise<boolean> {
  // Typed notes are user data — their presence always vetoes a delete.
  try {
    const notes = await invoke<{ notesMarkdown: string | null; notesJson: string | null } | null>(
      'api_get_meeting_notes',
      { meetingId },
    );
    const json = notes?.notesJson;
    // Treat a non-empty markdown or a non-empty blocks array as real notes.
    const hasNotes =
      (!!notes?.notesMarkdown && notes.notesMarkdown.trim().length > 0) ||
      (!!json && json.trim().length > 0 && json.trim() !== '[]');
    if (hasNotes) return false;
  } catch (error) {
    // If we can't tell, err on the side of keeping the meeting (don't destroy data).
    console.warn('Could not check notes before discarding meeting; keeping it:', error);
    return false;
  }

  // "Zero transcripts in the UI" can be a false abandonment signal (specs/0019 WS6.1):
  // only the backend can confirm nothing durable exists. Require an explicit true.
  try {
    const safe = await invoke<boolean>('api_recording_is_safe_to_discard', {
      meetingId,
      folderPath: folderPath ?? null,
    });
    return safe === true;
  } catch (error) {
    console.warn('Could not verify meeting is safe to discard; keeping it:', error);
    return false;
  }
}
