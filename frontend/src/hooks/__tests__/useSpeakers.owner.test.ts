import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0078 W3 — "This is me" / "This isn't me" through the shared speakers controller:
// the right command, then the same refresh an assign does (speakers + the transcript via
// onMutated), and a friendly toast on failure.

const { invokeMock, toastMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  toastMock: { success: vi.fn(), error: vi.fn() },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: vi.fn(() => () => {}) }));
vi.mock('sonner', () => ({ toast: toastMock }));

import { useSpeakers } from '@/hooks/useSpeakers';

let failMark = false;

beforeEach(() => {
  vi.clearAllMocks();
  failMark = false;
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case 'api_get_meeting_speakers':
        return [{ speakerKey: 'spk_0', displayName: 'Speaker 1', isLocal: false }];
      case 'api_get_meeting_audio_setup':
        return { override: 'auto', resolved: 'room' };
      case 'api_get_meeting_attendees':
        return { attendees: [], suggestion: null };
      case 'api_mark_speaker_as_me':
        if (failMark) throw 'That speaker is no longer in this meeting';
        return null;
      default:
        return [];
    }
  });
});

const speakerFetches = () =>
  invokeMock.mock.calls.filter(([c]) => c === 'api_get_meeting_speakers').length;

describe('useSpeakers owner actions (specs/0078)', () => {
  it('exposes the meeting audio setup', async () => {
    const { result } = renderHook(() => useSpeakers({ meetingId: 'm1' }));
    await waitFor(() => expect(result.current.audioSetup?.resolved).toBe('room'));
  });

  it('markAsMe calls the command per key, then refreshes speakers and the transcript', async () => {
    const onMutated = vi.fn();
    const { result } = renderHook(() => useSpeakers({ meetingId: 'm1', onMutated }));
    await waitFor(() => expect(result.current.speakers).toHaveLength(1));
    const before = speakerFetches();

    await act(async () => {
      await result.current.markAsMe(['spk_0', 'spk_2']);
    });

    expect(invokeMock).toHaveBeenCalledWith('api_mark_speaker_as_me', { meetingId: 'm1', speakerKey: 'spk_0' });
    expect(invokeMock).toHaveBeenCalledWith('api_mark_speaker_as_me', { meetingId: 'm1', speakerKey: 'spk_2' });
    expect(speakerFetches()).toBe(before + 1);
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(toastMock.success).toHaveBeenCalledWith('Marked as you');
  });

  it('unmarkMe calls the command, then refreshes', async () => {
    const onMutated = vi.fn();
    const { result } = renderHook(() => useSpeakers({ meetingId: 'm1', onMutated }));
    await waitFor(() => expect(result.current.speakers).toHaveLength(1));
    const before = speakerFetches();

    await act(async () => {
      await result.current.unmarkMe();
    });

    expect(invokeMock).toHaveBeenCalledWith('api_unmark_speaker_as_me', { meetingId: 'm1' });
    expect(speakerFetches()).toBe(before + 1);
    expect(onMutated).toHaveBeenCalledTimes(1);
  });

  it('a refused mark shows a friendly error toast with the backend reason', async () => {
    failMark = true;
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { result } = renderHook(() => useSpeakers({ meetingId: 'm1' }));
    await waitFor(() => expect(result.current.speakers).toHaveLength(1));

    await act(async () => {
      await result.current.markAsMe('spk_0');
    });

    expect(toastMock.error).toHaveBeenCalledWith('Could not mark this speaker as you', {
      description: 'That speaker is no longer in this meeting',
    });
    expect(toastMock.success).not.toHaveBeenCalled();
    err.mockRestore();
  });
});
