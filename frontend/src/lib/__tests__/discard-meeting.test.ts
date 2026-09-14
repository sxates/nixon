import { describe, it, expect, beforeEach, vi } from 'vitest';

// specs/0037 review-2 — the shared discard safety gate used by BOTH the stop path's
// abandoned-recording cleanup and the failed-start orphan cleanup. The contract:
// answer true ONLY when the meeting provably has nothing worth keeping (no typed
// notes AND the backend confirms nothing durable); any doubt or error answers false.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { canDiscardMeeting } from '@/lib/discard-meeting';

function route({
  notes = null as { notesMarkdown: string | null; notesJson: string | null } | null,
  safeToDiscard = true as unknown,
}: {
  notes?: { notesMarkdown: string | null; notesJson: string | null } | null;
  safeToDiscard?: unknown;
} = {}) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_get_meeting_notes':
        return Promise.resolve(notes);
      case 'api_recording_is_safe_to_discard':
        return Promise.resolve(safeToDiscard);
      default:
        return Promise.resolve(undefined);
    }
  });
}

describe('canDiscardMeeting', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('returns true only when there are no notes AND the backend confirms safe', async () => {
    route({ notes: null, safeToDiscard: true });
    await expect(canDiscardMeeting('m1', '/recordings/m1')).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_notes', { meetingId: 'm1' });
    expect(invokeMock).toHaveBeenCalledWith('api_recording_is_safe_to_discard', {
      meetingId: 'm1',
      folderPath: '/recordings/m1',
    });
  });

  it('forwards a null folderPath to the backend check', async () => {
    route();
    await canDiscardMeeting('m1', null);
    expect(invokeMock).toHaveBeenCalledWith('api_recording_is_safe_to_discard', {
      meetingId: 'm1',
      folderPath: null,
    });
  });

  it('typed markdown notes veto the discard (backend never even asked)', async () => {
    route({ notes: { notesMarkdown: 'important decision', notesJson: null } });
    await expect(canDiscardMeeting('m1', null)).resolves.toBe(false);
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_recording_is_safe_to_discard',
      expect.anything(),
    );
  });

  it('a non-empty notes blocks array vetoes the discard', async () => {
    route({ notes: { notesMarkdown: null, notesJson: '[{"type":"paragraph"}]' } });
    await expect(canDiscardMeeting('m1', null)).resolves.toBe(false);
  });

  it('empty notes payloads do not veto', async () => {
    route({ notes: { notesMarkdown: '   ', notesJson: '[]' } });
    await expect(canDiscardMeeting('m1', null)).resolves.toBe(true);
  });

  it('keeps the meeting when the notes check fails', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'api_get_meeting_notes'
        ? Promise.reject(new Error('db locked'))
        : Promise.resolve(true),
    );
    await expect(canDiscardMeeting('m1', null)).resolves.toBe(false);
  });

  it('keeps the meeting when the backend says not safe', async () => {
    route({ safeToDiscard: false });
    await expect(canDiscardMeeting('m1', null)).resolves.toBe(false);
  });

  it('keeps the meeting when the safe-to-discard check fails or returns a non-boolean', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'api_recording_is_safe_to_discard'
        ? Promise.reject(new Error('backend gone'))
        : Promise.resolve(null),
    );
    await expect(canDiscardMeeting('m1', null)).resolves.toBe(false);

    // An undefined answer (older backend / deserialization miss) must not read as "safe".
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'api_recording_is_safe_to_discard'
        ? Promise.resolve(undefined)
        : Promise.resolve(null),
    );
    await expect(canDiscardMeeting('m1', null)).resolves.toBe(false);
  });
});
