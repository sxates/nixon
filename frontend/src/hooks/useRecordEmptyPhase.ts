import { useEffect, useState } from 'react';
import { RecordingStatus, useRecordingState } from '@/contexts/RecordingStateContext';
import { isResumeArmedOrInFlight } from '@/lib/resume-recording';

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

  if (status === RecordingStatus.STARTING) return 'starting';
  if (startPending && status === RecordingStatus.IDLE) return 'starting';
  if (SAVING_STATUSES.has(status)) return 'saving';
  return undefined;
}
