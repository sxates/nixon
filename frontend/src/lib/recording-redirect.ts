/**
 * specs/0019 WS6.5 (note 24) — should opening a meeting-details route redirect to the
 * live recording screen?
 *
 * Only when the meeting being VIEWED is the one actually RECORDING. The active recording
 * is identified by `activeRecordingMeetingId` (set solely at recording start/stop), NOT by
 * `currentMeeting.id` — which is also overwritten to whatever meeting you open, so using it
 * made opening a *past* meeting mid-recording wrongly redirect you to /record.
 */
export interface RecordingRedirectInput {
  isRecording: boolean;
  isMeetingActive: boolean;
  /** The meeting id from the route (?id=…). */
  meetingId: string | null | undefined;
  /** The authoritative in-progress recording id (null when not recording). */
  activeRecordingMeetingId: string | null | undefined;
}

export function shouldRedirectToActiveRecording({
  isRecording,
  isMeetingActive,
  meetingId,
  activeRecordingMeetingId,
}: RecordingRedirectInput): boolean {
  return (
    isRecording &&
    isMeetingActive &&
    !!meetingId &&
    meetingId !== 'intro-call' &&
    activeRecordingMeetingId === meetingId
  );
}
