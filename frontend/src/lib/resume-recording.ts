/**
 * Resume / continue a recording (specs/0037).
 *
 * The three UI surfaces that can start a recording in *resume* mode — the relaunch
 * "Unfinished recording" prompt, the meeting-details "Continue recording" action, and
 * (via those) the `/record` page — hand off through a single `sessionStorage` key. This
 * mirrors the existing `autoStartRecording` flag used by the dashboard Record button:
 * the initiator writes the key + navigates to `/record`, and `useRecordingStart` consumes
 * it on mount and starts a session that REUSES an existing `meeting_id` + folder instead
 * of minting a fresh meeting row.
 */

import { indexedDBService } from '@/services/indexedDBService';

export const RESUME_RECORDING_KEY = 'resumeRecording';

/**
 * Fired on `window` right after a resume is armed. `useRecordingStart` listens for this so
 * an arm-while-already-on-`/record` still triggers the resume: `router.push('/record')` does
 * not remount an already-mounted recorder, so its mount effect never re-runs
 * `consumeResumeRecording`. The event lets the same consume+start path fire without a remount.
 * The stash is still the source of truth (read-and-clear), so this only ever starts one resume.
 */
export const RESUME_ARMED_EVENT = 'nixon:resume-armed';

export interface ResumeRecordingDescriptor {
  /** The existing meeting row to append to (never create a new one for a resume). */
  meetingId: string;
  /**
   * The meeting's recording folder, if known. When null, `useRecordingStart` resolves it
   * from the meeting row (`api_get_meeting_metadata`) before starting, and ABORTS the
   * resume with a user-facing toast if the meeting has no folder. It must NEVER thread a
   * null `resumeFolderPath` into the start invoke: the backend cannot infer resume-intent
   * from `meetingId` alone (normal starts pass one too), so a missing folder silently
   * degrades into a fresh recording whose stop saves a DUPLICATE meeting row — the exact
   * outcome specs/0037 exists to prevent.
   */
  folderPath: string | null;
  /** The existing meeting's title, so the live UI shows it without a round-trip. */
  meetingName?: string | null;
}

/** Arm a resume: the `/record` page's `useRecordingStart` consumes this on mount. */
export function armResumeRecording(descriptor: ResumeRecordingDescriptor): void {
  if (typeof window === 'undefined') return;
  sessionStorage.setItem(RESUME_RECORDING_KEY, JSON.stringify(descriptor));
  // Notify an already-mounted `/record` page (whose mount effect won't re-run on a
  // no-op `router.push('/record')`) that a resume is now armed. Dispatched AFTER the
  // stash write so the listener's `consumeResumeRecording` finds it.
  window.dispatchEvent(new CustomEvent(RESUME_ARMED_EVENT));
}

/**
 * Read + clear the resume descriptor. Returns null when nothing is armed (the normal
 * fresh-recording case) or the stash is malformed. Clearing on read guarantees a resume
 * fires exactly once, exactly like `autoStartRecording`.
 */
export function consumeResumeRecording(): ResumeRecordingDescriptor | null {
  if (typeof window === 'undefined') return null;
  const raw = sessionStorage.getItem(RESUME_RECORDING_KEY);
  if (!raw) return null;
  sessionStorage.removeItem(RESUME_RECORDING_KEY);
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed.meetingId === 'string' && parsed.meetingId) {
      return {
        meetingId: parsed.meetingId,
        folderPath: typeof parsed.folderPath === 'string' ? parsed.folderPath : null,
        meetingName: typeof parsed.meetingName === 'string' ? parsed.meetingName : null,
      };
    }
  } catch {
    /* malformed stash — ignore and fall through to null */
  }
  return null;
}

// ── Resume-in-flight tracking (specs/0037 review-2, FIX D) ─────────────────────────
//
// The `/record` page's legacy IndexedDB recovery startup check races the async resume
// start: both observe isRecording === false at mount, so without a guard the check can
// offer to "recover" the very meeting the user just chose to Resume — and that recovery
// saves WITHOUT a meetingId (a duplicate row) and runs recover_audio_from_checkpoints
// against the folder the live resumed session is writing into. `useRecordingStart` sets
// this flag synchronously when it consumes the stash (its mount effect runs BEFORE the
// page's startup effect) and clears it when the resume start settles; the startup check
// consults `isResumeArmedOrInFlight()` before offering recovery.

let resumeInFlight = false;

/** Set by `useRecordingStart` around a consumed resume's start attempt. */
export function setResumeInFlight(value: boolean): void {
  resumeInFlight = value;
}

/** True when a resume is armed (stash present) or its start is currently in flight. */
export function isResumeArmedOrInFlight(): boolean {
  if (resumeInFlight) return true;
  if (typeof window === 'undefined') return false;
  return sessionStorage.getItem(RESUME_RECORDING_KEY) !== null;
}

/**
 * Mark the crashed session's IndexedDB recovery entries as saved once a resume has
 * successfully adopted the meeting (specs/0037 review-2, FIX D). The entries are keyed
 * by TranscriptContext's fabricated `meeting-<timestamp>` ids — NOT the SQLite meeting
 * id — so we match them by the recording folder the resume is appending into. Without
 * this, the stale entry re-offers "recovering" the same meeting every session.
 * Best-effort: failures only log (recovery hygiene must never break a live resume).
 */
export async function markRecoveryEntriesSavedForFolder(folderPath: string): Promise<void> {
  try {
    const entries = await indexedDBService.getAllMeetings(); // unsaved entries only
    await Promise.all(
      entries
        .filter((entry) => !!entry.folderPath && entry.folderPath === folderPath)
        .map((entry) => indexedDBService.markMeetingSaved(entry.meetingId)),
    );
  } catch (error) {
    console.warn('Could not mark resumed recovery entries as saved:', error);
  }
}
