import { useEffect, useState } from 'react';
import { RecordingStatus, useRecordingState } from '@/contexts/RecordingStateContext';
import { isResumeArmedOrInFlight } from '@/lib/resume-recording';

/** Dispatched on `window` by useRecordingStart when a requested auto-start gives up before
 *  STARTING (e.g. no transcription model yet) — status never leaves IDLE in that case. */
export const AUTO_START_ABANDONED_EVENT = 'nixon:auto-start-abandoned';

/** Longest a requested start may read as "starting" without the status moving: a backstop
 *  for any early return that doesn't announce itself. A real start reaches STARTING in
 *  well under a second. */
export const START_PENDING_MAX_MS = 10_000;

/** What the record page's empty transcript should say while it has no segments. */
export type RecordEmptyPhase = 'starting' | 'saving' | undefined;

const SAVING_STATUSES = new Set<RecordingStatus>([
  RecordingStatus.STOPPING,
  RecordingStatus.PROCESSING_TRANSCRIPTS,
  RecordingStatus.SAVING,
  RecordingStatus.COMPLETED,
]);

/** A start requested before this page mounted: REC from another page, Join & Record, or
 *  a resume. Each sets its sessionStorage flag and then navigates to /record. */
function startWasRequested(): boolean {
  try {
    return window.sessionStorage.getItem('autoStartRecording') === 'true' || isResumeArmedOrInFlight();
  } catch {
    return false;
  }
}

/**
 * The record page between "idle" and "recording" (owner feedback 2026-09-23): pressing REC
 * lands on /record before the recording status flips, and stopping leaves it there while
 * the meeting saves, so both used to flash the idle "Welcome to Nixon!" copy. A requested
 * start reads as `'starting'` from the first render; stop through save reads `'saving'`.
 */
export function useRecordEmptyPhase(): RecordEmptyPhase {
  const { status } = useRecordingState();
  const [startPending, setStartPending] = useState(startWasRequested);
  useEffect(() => {
    // The flag has done its job once the start is under way (or failed).
    if (status !== RecordingStatus.IDLE) setStartPending(false);
  }, [status]);
  useEffect(() => {
    // A start abandoned before STARTING leaves status at IDLE, so the effect above never
    // fires: without this the page read "Listening for speech…" with nothing recording
    // (whole-branch review, 2026-09-23).
    if (!startPending) return;
    const clear = () => setStartPending(false);
    window.addEventListener(AUTO_START_ABANDONED_EVENT, clear);
    const backstop = window.setTimeout(clear, START_PENDING_MAX_MS);
    return () => {
      window.removeEventListener(AUTO_START_ABANDONED_EVENT, clear);
      window.clearTimeout(backstop);
    };
  }, [startPending]);

  if (status === RecordingStatus.STARTING) return 'starting';
  if (startPending && status === RecordingStatus.IDLE) return 'starting';
  if (SAVING_STATUSES.has(status)) return 'saving';
  return undefined;
}
