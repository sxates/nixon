import { describe, it, expect } from 'vitest';
import { shouldRedirectToActiveRecording } from '@/lib/recording-redirect';

// specs/0019 WS6.5 (note 24) — opening a PAST meeting while recording must NOT bounce you
// to /record. The redirect fires only when the viewed meeting IS the active recording,
// identified by activeRecordingMeetingId (not the view-tracking currentMeeting.id).

const base = {
  isRecording: true,
  isMeetingActive: true,
  meetingId: 'A',
  activeRecordingMeetingId: 'A',
};

describe('shouldRedirectToActiveRecording (WS6.5)', () => {
  it('redirects when viewing the meeting that is actively recording', () => {
    expect(shouldRedirectToActiveRecording(base)).toBe(true);
  });

  it('does NOT redirect when viewing a different (past) meeting mid-recording', () => {
    // The regression: recording A, opening B. activeRecordingMeetingId stays 'A'.
    expect(
      shouldRedirectToActiveRecording({ ...base, meetingId: 'B' }),
    ).toBe(false);
  });

  it('does not redirect when nothing is recording', () => {
    expect(
      shouldRedirectToActiveRecording({
        ...base,
        isRecording: false,
        activeRecordingMeetingId: null,
      }),
    ).toBe(false);
  });

  it('does not redirect for the intro-call placeholder or a missing id', () => {
    expect(
      shouldRedirectToActiveRecording({ ...base, meetingId: 'intro-call', activeRecordingMeetingId: 'intro-call' }),
    ).toBe(false);
    expect(shouldRedirectToActiveRecording({ ...base, meetingId: null })).toBe(false);
  });

  it('requires the meeting to be active (guards a stale recording flag)', () => {
    expect(
      shouldRedirectToActiveRecording({ ...base, isMeetingActive: false }),
    ).toBe(false);
  });
});
