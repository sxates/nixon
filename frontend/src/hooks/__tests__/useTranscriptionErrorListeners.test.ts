import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0057 Plan 2 residual — the `transcript-error` listener had no emitter anywhere (no Rust
// `emit`, no frontend producer), so it was dead code. These tests pin the live wiring. Since
// specs/0083 a `transcription-error` stops nothing from the webview (a per-chunk engine failure
// leaves the recording running; stopping is Rust's), and nothing subscribes to `transcript-error`.

const { listenMock, handlers } = vi.hoisted(() => {
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  return {
    handlers,
    listenMock: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
      handlers.set(event, handler);
      return () => handlers.delete(event);
    }),
  };
});
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

import { useTranscriptionErrorListeners } from '@/hooks/useTranscriptionErrorListeners';

const flush = () => act(async () => { await Promise.resolve(); });

describe('useTranscriptionErrorListeners', () => {
  beforeEach(() => {
    handlers.clear();
    listenMock.mockClear();
  });

  it('handles transcription-error without throwing', async () => {
    renderHook(() => useTranscriptionErrorListeners());
    await flush();

    const handler = handlers.get('transcription-error');
    expect(handler).toBeDefined();
    expect(() => act(() => handler!({ payload: { message: 'model failed to load' } }))).not.toThrow();
  });

  it('does not subscribe to the emitter-less transcript-error event', async () => {
    renderHook(() => useTranscriptionErrorListeners());
    await flush();

    expect(listenMock.mock.calls.map((c) => c[0])).toEqual([
      'transcription-error',
      'speech-detected',
    ]);
    expect(handlers.has('transcript-error')).toBe(false);
  });
});
