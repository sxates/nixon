import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0078 W3 — the "Who was on the mic?" hook: fetch per meeting, take `resolved` from
// a completed pass for THIS meeting (from the payload, re-fetching only when the payload
// can't be trusted), and set = store the override through the command that also starts
// the re-run. The hook never subscribes itself: the speakers controller owns the one
// `diarization-complete` listener and feeds it `applyDiarizationComplete`.

const { invokeMock, safeListenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  safeListenMock: vi.fn(() => () => {}),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: safeListenMock }));

import { useAudioSetup, isRoomSetup } from '@/hooks/useAudioSetup';

let stored: { override: string; resolved: string | null };

beforeEach(() => {
  vi.clearAllMocks();
  stored = { override: 'auto', resolved: null };
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_get_meeting_audio_setup') return { ...stored };
    if (cmd === 'api_set_meeting_audio_setup') return { started: true, alreadyRunning: false };
    return null;
  });
});

const getCalls = () =>
  invokeMock.mock.calls.filter(([c]) => c === 'api_get_meeting_audio_setup').length;

describe('useAudioSetup (specs/0078)', () => {
  it('fetches the setup for the meeting on mount', async () => {
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(result.current.setup).toEqual({ override: 'auto', resolved: null }));
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_audio_setup', { meetingId: 'm1' });
  });

  it('does not subscribe to any event itself', async () => {
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(result.current.setup).not.toBeNull());
    expect(safeListenMock).not.toHaveBeenCalled();
  });

  it('takes `resolved` from a completion payload for this meeting, without a re-fetch', async () => {
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(result.current.setup).not.toBeNull());
    const before = getCalls();

    act(() =>
      result.current.applyDiarizationComplete({
        meeting_id: 'other',
        audioSetup: 'room',
        audioSetupSource: 'detected',
      }),
    );
    expect(result.current.setup?.resolved).toBeNull();

    act(() =>
      result.current.applyDiarizationComplete({
        meeting_id: 'm1',
        audioSetup: 'room',
        audioSetupSource: 'detected',
      }),
    );
    expect(result.current.setup).toEqual({ override: 'auto', resolved: 'room' });
    await act(async () => {});
    expect(getCalls()).toBe(before);
  });

  it('falls back to a re-fetch when the payload has no audio setup', async () => {
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(result.current.setup).not.toBeNull());
    const before = getCalls();

    stored = { override: 'auto', resolved: 'hybrid' };
    act(() => result.current.applyDiarizationComplete({ meeting_id: 'm1' }));
    await waitFor(() => expect(result.current.setup?.resolved).toBe('hybrid'));
    expect(getCalls()).toBe(before + 1);
  });

  it('re-fetches when the payload source disagrees with the override it holds', async () => {
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(result.current.setup?.override).toBe('auto'));

    // Another window forced "room"; this view still holds "auto".
    stored = { override: 'room', resolved: 'room' };
    act(() =>
      result.current.applyDiarizationComplete({
        meeting_id: 'm1',
        audioSetup: 'room',
        audioSetupSource: 'override',
      }),
    );
    // `resolved` lands at once; the override follows from the re-read.
    expect(result.current.setup?.resolved).toBe('room');
    await waitFor(() => expect(result.current.setup?.override).toBe('room'));
  });

  it('setOverride stores the choice through the set command and reflects it', async () => {
    stored = { override: 'auto', resolved: 'call' };
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(result.current.setup?.resolved).toBe('call'));

    let out: unknown;
    await act(async () => {
      out = await result.current.setOverride('room');
    });
    expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_audio_setup', {
      meetingId: 'm1',
      setup: 'room',
    });
    expect(out).toEqual({ started: true, alreadyRunning: false });
    // The override updates now; `resolved` waits for the pass.
    expect(result.current.setup).toEqual({ override: 'room', resolved: 'call' });
  });

  it('a failed set re-reads the setup and rethrows', async () => {
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(result.current.setup).not.toBeNull());
    const before = getCalls();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_set_meeting_audio_setup') throw new Error('model missing');
      return { override: 'room', resolved: null };
    });

    await act(async () => {
      await expect(result.current.setOverride('room')).rejects.toThrow('model missing');
    });
    await waitFor(() => expect(result.current.setup?.override).toBe('room'));
    expect(getCalls()).toBe(before + 1);
  });

  it('a failed fetch leaves setup null (no caption, no crash)', async () => {
    invokeMock.mockRejectedValue(new Error('boom'));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(result.current.setup).toBeNull();
    warn.mockRestore();
  });

  it('isRoomSetup is true for room and hybrid only', () => {
    expect(isRoomSetup('room')).toBe(true);
    expect(isRoomSetup('hybrid')).toBe(true);
    expect(isRoomSetup('call')).toBe(false);
    expect(isRoomSetup(null)).toBe(false);
  });
});
