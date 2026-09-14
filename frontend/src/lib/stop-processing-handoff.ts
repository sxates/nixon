/**
 * What the stop path does after trying to hand a meeting to the deferred backlog
 * (spec 0051 WS2).
 *
 * Before this, `stopAction === 'process-now'` fired a `window.dispatchEvent` and
 * assumed it landed. That assumption suppressed BOTH of the normal fallbacks —
 * `shouldAutoDiarizeAtStop` returns false for 'process-now', and the meeting-details
 * auto-summary gate returns 'deferred-processing' and skips *silently* — so a single
 * missed dispatch produced a meeting with no speakers, no summary, and no explanation.
 * It also stranded the meeting at `processing_mode='live'`, which
 * `list_deferred_candidates` never matches, making the state permanent.
 *
 * The fix inverts the marker: the stop path writes the durable 'defer' marker BEFORE
 * attempting the handoff, so an unaccepted handoff degrades into the ordinary
 * deferred-meeting case the system already recovers from — on the next launch (via the
 * Rust startup reconciliation), on the next AC connection, or via "Process now".
 */

import { shouldAutoDiarizeAtStop, type StopAction } from '@/lib/processing-mode';

/** Whether the deferred backlog took ownership of the meeting. */
export type HandoffOutcome =
  | { accepted: true }
  /** `no-folder-path`: the backlog could not resolve the meeting's audio folder.
   *  `threw`: the enqueue call itself failed. */
  | { accepted: false; reason: 'no-folder-path' | 'threw' };

export interface StopFollowUp {
  /** Run the best-effort auto-diarization pass at stop. */
  autoDiarize: boolean;
  /** Leave `processing_mode='defer'` in place so a later pass can retry. */
  deferMarker: boolean;
  /** User-facing warning, or null when there is nothing to say. */
  toast: { title: string; description: string } | null;
}

const HANDOFF_FAILED_TOAST = {
  title: 'Couldn\'t start processing this meeting',
  description:
    'Your recording is saved. Nixon will retry the next time it starts, or use "Process now" on the transcript.',
};

/**
 * `handoff` is `null` for any action other than 'process-now' (no handoff was
 * attempted). For 'process-now' a `null` handoff means the attempt produced no
 * answer, which is treated as a failure — silence is the bug being fixed.
 */
export function stopFollowUp(
  action: StopAction,
  handoff: HandoffOutcome | null,
): StopFollowUp {
  if (action !== 'process-now') {
    return {
      autoDiarize: shouldAutoDiarizeAtStop(action),
      deferMarker: action === 'mark-defer',
      toast: null,
    };
  }

  // The marker stays either way: on success the backlog's own pipeline clears it as
  // its last step; on failure it is exactly what makes the meeting recoverable.
  if (handoff?.accepted) {
    return { autoDiarize: false, deferMarker: true, toast: null };
  }

  // Nothing is going to run on its own — give the user speakers now and say so.
  return { autoDiarize: true, deferMarker: true, toast: HANDOFF_FAILED_TOAST };
}
