import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';

type Handler = (event: { payload?: { muted?: boolean } }) => void;
const { listeners, safeListenMock } = vi.hoisted(() => {
  const listeners = new Map<string, Handler>();
  return {
    listeners,
    safeListenMock: vi.fn((event: string, handler: Handler) => {
      listeners.set(event, handler);
      return () => listeners.delete(event);
    }),
  };
});
vi.mock('@/lib/safe-listen', () => ({ safeListen: safeListenMock }));

import { useMicGate } from '@/hooks/useMicGate';

const emit = (event: string, payload?: { muted?: boolean }) =>
  act(() => {
    listeners.get(event)?.({ payload });
  });

beforeEach(() => {
  listeners.clear();
  vi.clearAllMocks();
});

// specs/0049 + 0057 §3.1 — `zoom-mute-changed` is edge-triggered and mute_monitor emits
// nothing (not even `false`) when it stops, so the gate needs a second, terminal edge.
describe('useMicGate', () => {
  it('follows zoom-mute-changed', () => {
    const { result } = renderHook(() => useMicGate());
    expect(result.current).toBe(false);
    emit('zoom-mute-changed', { muted: true });
    expect(result.current).toBe(true);
    emit('zoom-mute-changed', { muted: false });
    expect(result.current).toBe(false);
  });

  it('resets to false on recording-stopped so a muted session cannot stick', () => {
    const { result } = renderHook(() => useMicGate());
    emit('zoom-mute-changed', { muted: true });
    expect(result.current).toBe(true);
    emit('recording-stopped');
    expect(result.current).toBe(false);
  });
});
