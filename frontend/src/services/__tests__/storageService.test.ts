import { describe, it, expect, beforeEach, vi } from 'vitest';

// specs/0023 L3 — Tauri IPC contract test. vi.hoisted lets the mock factory
// reference the spy (vi.mock is hoisted above imports). This locks the save
// contract that specs/0019 WS6.7 hinges on: the existing-meeting id is threaded
// through to the backend exactly as given (or null when omitted).
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { storageService } from '@/services/storageService';

describe('storageService.saveMeeting', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ meeting_id: 'meeting-new' });
  });

  it('calls api_save_transcript with meetingId=null and resumed defaults when no id is provided', async () => {
    await storageService.saveMeeting('My Meeting', [], '/recordings/abc');

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith('api_save_transcript', {
      meetingTitle: 'My Meeting',
      transcripts: [],
      folderPath: '/recordings/abc',
      meetingId: null,
      // specs/0037 — a normal save defaults to a non-resumed append (false/0), so the
      // backend takes its unchanged new-row path.
      resumed: false,
      audioOffsetSeconds: 0,
    });
  });

  it('threads an explicit existing meetingId through unchanged', async () => {
    await storageService.saveMeeting('My Meeting', [], null, 'meeting-existing');

    expect(invokeMock).toHaveBeenCalledWith('api_save_transcript', {
      meetingTitle: 'My Meeting',
      transcripts: [],
      folderPath: null,
      meetingId: 'meeting-existing',
      resumed: false,
      audioOffsetSeconds: 0,
    });
  });

  it('routes a resumed save with the append flag + audio offset (specs/0037)', async () => {
    await storageService.saveMeeting('My Meeting', [], '/recordings/prev', 'meeting-existing', true, 42.5);

    expect(invokeMock).toHaveBeenCalledWith('api_save_transcript', {
      meetingTitle: 'My Meeting',
      transcripts: [],
      folderPath: '/recordings/prev',
      meetingId: 'meeting-existing',
      resumed: true,
      audioOffsetSeconds: 42.5,
    });
  });

  it('returns the backend response (the authoritative saved id)', async () => {
    const res = await storageService.saveMeeting('My Meeting', [], null, 'stale-id');
    // Even if a stale id is passed, the frontend adopts whatever the backend returns
    // (the WS6.7 backend net may mint a fresh row and return its id).
    expect(res).toEqual({ meeting_id: 'meeting-new' });
  });
});
