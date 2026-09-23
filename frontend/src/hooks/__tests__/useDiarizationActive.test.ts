import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, Array<(event: { payload: unknown }) => void>>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn((eventName: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(eventName, [...(listeners.get(eventName) ?? []), handler]);
    return () => {
      listeners.set(eventName, (listeners.get(eventName) ?? []).filter((h) => h !== handler));
    };
  }),
}));

import { useDiarizationActive } from '@/hooks/useDiarizationActive';

const emit = (name: string, payload: unknown) =>
  act(() => (listeners.get(name) ?? []).forEach((h) => h({ payload })));

describe('useDiarizationActive', () => {
  beforeEach(() => {
    listeners.clear();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
  });

  it('is active from this meeting’s progress until it completes', () => {
    const { result } = renderHook(() => useDiarizationActive('m1'));
    expect(result.current).toBe(false);

    emit('diarization-progress', { meeting_id: 'm1', stage: 'clustering' });
    expect(result.current).toBe(true);

    emit('diarization-complete', { meeting_id: 'm1' });
    expect(result.current).toBe(false);
  });

  it('an error ends it too', () => {
    const { result } = renderHook(() => useDiarizationActive('m1'));
    emit('diarization-progress', { meeting_id: 'm1' });
    emit('diarization-error', { meeting_id: 'm1' });
    expect(result.current).toBe(false);
  });

  it('ignores other meetings and meeting-less (model download) progress', () => {
    const { result } = renderHook(() => useDiarizationActive('m1'));
    emit('diarization-progress', { meeting_id: 'm2' });
    emit('diarization-progress', { stage: 'downloading' });
    expect(result.current).toBe(false);
  });

  it('picks up a pass already running when the page opens', async () => {
    invokeMock.mockResolvedValue({ running: true, stage: 'embedding', progressPct: 40 });
    const { result } = renderHook(() => useDiarizationActive('m1'));
    await waitFor(() => expect(result.current).toBe(true));
    expect(invokeMock).toHaveBeenCalledWith('api_diarization_status', { meetingId: 'm1' });
  });
});
