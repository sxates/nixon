import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0057 §3.2 (Plan 3 residual): per-channel levels for the CH1 MIC / CH2 SYS needles.
type Handler = (e: { payload: unknown }) => void;
const { listeners } = vi.hoisted(() => ({ listeners: new Map<string, Handler>() }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (name: string, cb: Handler) => {
    listeners.set(name, cb);
    return () => listeners.delete(name);
  },
}));

import { parseLevelPayload, useRecordingLevel } from '@/hooks/useRecordingLevel';

const emit = (payload: unknown) => act(() => listeners.get('recording-level')?.({ payload }));

beforeEach(() => {
  vi.useFakeTimers();
  listeners.clear();
});
afterEach(() => vi.useRealTimers());

describe('parseLevelPayload', () => {
  it('reads the mixed pair and both channel pairs', () => {
    expect(
      parseLevelPayload({ rms: 0.3, peak: 0.5, mic: { rms: 0.1, peak: 0.2 }, sys: { rms: 0.6, peak: 0.9 } }),
    ).toEqual({ rms: 0.3, peak: 0.5, mic: { rms: 0.1, peak: 0.2 }, sys: { rms: 0.6, peak: 0.9 } });
  });

  it('falls back to the mixed level when a channel object is missing (older emitter)', () => {
    expect(parseLevelPayload({ rms: 0.3, peak: 0.5 })).toEqual({
      rms: 0.3,
      peak: 0.5,
      mic: { rms: 0.3, peak: 0.5 },
      sys: { rms: 0.3, peak: 0.5 },
    });
  });

  it('treats garbage as silence', () => {
    expect(parseLevelPayload(undefined)).toEqual({ rms: 0, peak: 0, mic: { rms: 0, peak: 0 }, sys: { rms: 0, peak: 0 } });
    expect(parseLevelPayload({ rms: 'x' as unknown as number, mic: { rms: NaN } }).mic).toEqual({ rms: 0, peak: 0 });
  });
});

describe('useRecordingLevel', () => {
  it('exposes per-channel levels and latches PEAK on a clipping channel even when the mix is clean', () => {
    const { result } = renderHook(() => useRecordingLevel(true));
    emit({ rms: 0.4, peak: 0.6, mic: { rms: 0.05, peak: 0.99 }, sys: { rms: 0.4, peak: 0.6 } });
    expect(result.current.mic).toEqual({ rms: 0.05, peak: 0.99 });
    expect(result.current.sys).toEqual({ rms: 0.4, peak: 0.6 });
    expect(result.current.peakLatched).toBe(true);
    act(() => vi.advanceTimersByTime(800));
    expect(result.current.peakLatched).toBe(false);
  });

  it('decays every channel to silence when events stop', () => {
    const { result } = renderHook(() => useRecordingLevel(true));
    emit({ rms: 0.4, peak: 0.6, mic: { rms: 0.3, peak: 0.3 }, sys: { rms: 0.2, peak: 0.2 } });
    act(() => vi.advanceTimersByTime(500));
    expect(result.current.rms).toBe(0);
    expect(result.current.mic).toEqual({ rms: 0, peak: 0 });
    expect(result.current.sys).toEqual({ rms: 0, peak: 0 });
  });

  it('is idle and unsubscribed when disabled', () => {
    const { result } = renderHook(() => useRecordingLevel(false));
    expect(listeners.has('recording-level')).toBe(false);
    expect(result.current.mic).toEqual({ rms: 0, peak: 0 });
  });
});
