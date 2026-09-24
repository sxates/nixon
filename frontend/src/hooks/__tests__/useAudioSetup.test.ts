import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0078 W3 — the "Who was on the mic?" hook: fetch per meeting, re-fetch when a
// pass for THIS meeting completes (that is when `resolved` changes), and set = store the
// override through the command that also starts the re-run.

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn((event: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(event, cb);
    return () => listeners.delete(event);
  }),
}));

import { useAudioSetup, isRoomSetup } from '@/hooks/useAudioSetup';

let stored: { override: string; resolved: string | null };

beforeEach(() => {
  vi.clearAllMocks();
  listeners.clear();
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

  it('re-fetches on diarization-complete for this meeting only', async () => {
    const { result } = renderHook(() => useAudioSetup('m1'));
    await waitFor(() => expect(result.current.setup).not.toBeNull());
    const before = getCalls();

    stored = { override: 'auto', resolved: 'room' };
    act(() => listeners.get('diarization-complete')!({ payload: { meeting_id: 'other' } }));
    expect(getCalls()).toBe(before);

    act(() => listeners.get('diarization-complete')!({ payload: { meeting_id: 'm1' } }));
    await waitFor(() => expect(result.current.setup?.resolved).toBe('room'));
    expect(getCalls()).toBe(before + 1);
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
