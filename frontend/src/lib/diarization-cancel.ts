import { invoke } from '@tauri-apps/api/core';

/**
 * specs/0084 W2 — cancel a running speaker-identification pass (the queue row's and the
 * meeting page's Cancel). When this resolves `true`, Rust has already ended the pass for the
 * user: its queue row is finished as skipped and the quiet `diarization-error`
 * (`cancelled: true`) has gone out. A pass inside sherpa's compute step finishes unseen and
 * writes nothing, so "Cancelling…" only shows for the length of this call.
 *
 * Resolves `false` when nothing was running for the meeting (it already finished).
 */
export async function cancelSpeakerIdentification(meetingId: string): Promise<boolean> {
  const result = await invoke<{ cancelled: boolean } | null>('api_cancel_diarization', { meetingId });
  return result?.cancelled === true;
}

/** The label a Cancel control shows between the click and the pass ending. */
export const CANCELLING_LABEL = 'Cancelling…';

/**
 * Whether a refused start is the "a cancelled pass is still finishing" answer
 * (`diarization/detach.rs` `STILL_STOPPING`): shown as a plain notice, not a failure.
 */
export function isStillStopping(message: string): boolean {
  return message.startsWith('Still stopping the previous run');
}
