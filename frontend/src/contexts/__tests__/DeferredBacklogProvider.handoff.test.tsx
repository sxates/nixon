import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, waitFor, act } from '@testing-library/react';

// specs/0083 W2b (spec 0051 WS2 invariant) — the wrap-up asks the backlog to take a
// 'process-now' meeting and waits up to 30 s for the answer. The backlog must always answer,
// a refusal or a throw included, so Rust either leaves it to the backlog or degrades to
// diarizing it and telling the user.

const h = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  invoke: vi.fn(),
  enqueueMeeting: vi.fn(),
}));

vi.mock('@/lib/safe-listen', () => ({
  safeListen: (event: string, handler: (event: { payload: unknown }) => void) => {
    h.listeners.set(event, handler);
    return () => h.listeners.delete(event);
  },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@/hooks/useDeferredBacklog', () => ({
  useDeferredBacklog: () => ({
    view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 },
    enqueueMeeting: h.enqueueMeeting,
    stop: vi.fn(),
    dismissDone: vi.fn(),
    startNow: vi.fn(),
  }),
}));

import { DeferredBacklogProvider } from '@/contexts/DeferredBacklogProvider';

const wrapUp = (stage: string, meetingId = 'm-1') => ({
  id: 3,
  meetingId,
  source: 'user',
  stage,
  title: 'Standup',
  transcriptionIncomplete: false,
  reason: null,
  message: null,
  toast: null,
  failedStage: null,
});

async function emit(payload: unknown) {
  await act(async () => {
    h.listeners.get('meeting-wrap-up')!({ payload });
    await Promise.resolve();
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  h.listeners.clear();
  h.invoke.mockResolvedValue(undefined);
});

describe('DeferredBacklogProvider answers the wrap-up handoff (specs/0083)', () => {
  it('enqueues the meeting forced and reports it accepted', async () => {
    h.enqueueMeeting.mockResolvedValue({ accepted: true });
    render(<DeferredBacklogProvider><div /></DeferredBacklogProvider>);
    await emit(wrapUp('handoff'));

    expect(h.enqueueMeeting).toHaveBeenCalledWith('m-1', { force: true });
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('api_wrap_up_handoff_result', {
        meetingId: 'm-1',
        accepted: true,
      }),
    );
  });

  it('reports a refusal', async () => {
    h.enqueueMeeting.mockResolvedValue({ accepted: false, reason: 'no-folder-path' });
    render(<DeferredBacklogProvider><div /></DeferredBacklogProvider>);
    await emit(wrapUp('handoff'));
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('api_wrap_up_handoff_result', {
        meetingId: 'm-1',
        accepted: false,
      }),
    );
  });

  it('reports a throw as a refusal instead of leaving Rust to time out', async () => {
    h.enqueueMeeting.mockRejectedValue(new Error('backlog not ready'));
    render(<DeferredBacklogProvider><div /></DeferredBacklogProvider>);
    await emit(wrapUp('handoff'));
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('api_wrap_up_handoff_result', {
        meetingId: 'm-1',
        accepted: false,
      }),
    );
  });

  it('ignores every other stage', async () => {
    render(<DeferredBacklogProvider><div /></DeferredBacklogProvider>);
    for (const stage of ['saving', 'saved', 'diarizing', 'summarizing', 'done']) {
      await emit(wrapUp(stage));
    }
    expect(h.enqueueMeeting).not.toHaveBeenCalled();
    expect(h.invoke).not.toHaveBeenCalled();
  });
});
