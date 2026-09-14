import { describe, it, expect, beforeEach, vi } from 'vitest';

// specs/0037 — the resume hand-off stash plus the review-2 FIX D additions:
// the armed/in-flight signal the /record page's legacy IndexedDB recovery check consults
// (so it can't offer to "recover" the meeting being resumed), and the folder-matched
// reconciliation that marks the crashed session's IndexedDB entries saved.

const { getAllMeetings, markMeetingSaved } = vi.hoisted(() => ({
  getAllMeetings: vi.fn(),
  markMeetingSaved: vi.fn(),
}));
vi.mock('@/services/indexedDBService', () => ({
  indexedDBService: { getAllMeetings, markMeetingSaved },
}));

import {
  armResumeRecording,
  consumeResumeRecording,
  isResumeArmedOrInFlight,
  markRecoveryEntriesSavedForFolder,
  RESUME_RECORDING_KEY,
  setResumeInFlight,
} from '@/lib/resume-recording';

describe('resume stash (arm/consume)', () => {
  beforeEach(() => {
    sessionStorage.clear();
    setResumeInFlight(false);
  });

  it('consume is read-and-clear and round-trips the descriptor', () => {
    armResumeRecording({ meetingId: 'm1', folderPath: '/recordings/m1', meetingName: 'Sync' });
    expect(consumeResumeRecording()).toEqual({
      meetingId: 'm1',
      folderPath: '/recordings/m1',
      meetingName: 'Sync',
    });
    expect(consumeResumeRecording()).toBeNull();
  });

  it('returns null for a malformed stash', () => {
    sessionStorage.setItem(RESUME_RECORDING_KEY, '{not json');
    expect(consumeResumeRecording()).toBeNull();
    expect(sessionStorage.getItem(RESUME_RECORDING_KEY)).toBeNull(); // still cleared
  });
});

describe('isResumeArmedOrInFlight (review-2 FIX D)', () => {
  beforeEach(() => {
    sessionStorage.clear();
    setResumeInFlight(false);
  });

  it('is false when nothing is armed and no resume is starting', () => {
    expect(isResumeArmedOrInFlight()).toBe(false);
  });

  it('is true while the stash is armed, and false again once consumed', () => {
    armResumeRecording({ meetingId: 'm1', folderPath: '/r', meetingName: null });
    expect(isResumeArmedOrInFlight()).toBe(true);
    consumeResumeRecording();
    expect(isResumeArmedOrInFlight()).toBe(false);
  });

  it('is true while a consumed resume start is in flight (the recovery-check race window)', () => {
    // Mirrors useRecordingStart: consume, then flag in-flight synchronously — the page's
    // startup recovery check runs after and must see true even though the stash is gone.
    armResumeRecording({ meetingId: 'm1', folderPath: '/r', meetingName: null });
    consumeResumeRecording();
    setResumeInFlight(true);
    expect(isResumeArmedOrInFlight()).toBe(true);
    setResumeInFlight(false);
    expect(isResumeArmedOrInFlight()).toBe(false);
  });
});

describe('markRecoveryEntriesSavedForFolder (review-2 FIX D)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    markMeetingSaved.mockResolvedValue(undefined);
  });

  it('marks only the unsaved entries whose folder matches the resumed folder', async () => {
    getAllMeetings.mockResolvedValue([
      { meetingId: 'meeting-100', folderPath: '/recordings/prev', savedToSQLite: false },
      { meetingId: 'meeting-101', folderPath: '/recordings/other', savedToSQLite: false },
      { meetingId: 'meeting-102', savedToSQLite: false }, // no folder recorded
    ]);

    await markRecoveryEntriesSavedForFolder('/recordings/prev');

    expect(markMeetingSaved).toHaveBeenCalledTimes(1);
    expect(markMeetingSaved).toHaveBeenCalledWith('meeting-100');
  });

  it('is best-effort: an IndexedDB failure resolves without throwing', async () => {
    getAllMeetings.mockRejectedValue(new Error('indexeddb unavailable'));
    await expect(markRecoveryEntriesSavedForFolder('/recordings/prev')).resolves.toBeUndefined();
    expect(markMeetingSaved).not.toHaveBeenCalled();
  });
});
