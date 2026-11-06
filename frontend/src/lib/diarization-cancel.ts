import { invoke } from '@tauri-apps/api/core';

/**
 * specs/0084 W2 — cancel a running speaker-identification pass (the queue row's and the
 * meeting page's Cancel). Rust stops the pass at its next stage boundary and ends it with a
 * quiet `diarization-error` carrying `cancelled: true`; a pass already inside its compute step
 * stops when that step returns, so callers show "Cancelling…" until the pass ends.
 *
 * Resolves `false` when nothing was running for the meeting (it already finished).
 */
export async function cancelSpeakerIdentification(meetingId: string): Promise<boolean> {
  const result = await invoke<{ cancelled: boolean } | null>('api_cancel_diarization', { meetingId });
  return result?.cancelled === true;
}

/** The label a Cancel control shows between the click and the pass ending. */
export const CANCELLING_LABEL = 'Cancelling…';
