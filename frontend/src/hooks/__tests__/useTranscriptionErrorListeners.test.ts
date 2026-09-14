import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0057 Plan 2 residual — the `transcript-error` listener had no emitter anywhere (no Rust
// `emit`, no frontend producer), so it was dead code. These tests pin the live wiring:
// `transcription-error` still stops the recording, and nothing subscribes to `transcript-error`.

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

  it('stops the recording when transcription-error fires', async () => {
    const onRecordingStop = vi.fn();
    renderHook(() => useTranscriptionErrorListeners({ onRecordingStop }));
    await flush();

    const handler = handlers.get('transcription-error');
    expect(handler).toBeDefined();
    act(() => handler!({ payload: { message: 'model failed to load' } }));
    expect(onRecordingStop).toHaveBeenCalledWith(false);
  });

  it('does not subscribe to the emitter-less transcript-error event', async () => {
    renderHook(() => useTranscriptionErrorListeners({ onRecordingStop: vi.fn() }));
    await flush();

    expect(listenMock.mock.calls.map((c) => c[0])).toEqual([
      'transcription-error',
      'speech-detected',
    ]);
    expect(handlers.has('transcript-error')).toBe(false);
  });
});
