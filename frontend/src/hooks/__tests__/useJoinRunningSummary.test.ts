import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0083 W2b — the meeting page follows the summary the Rust wrap-up started instead of
// starting a second one: it joins the shared poll on mount when a run is pending/processing,
// and on `meeting-wrap-up` `summarizing` for the meeting it shows.

const h = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  invoke: vi.fn(),
}));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (event: string, handler: (event: { payload: unknown }) => void) => {
    h.listeners.set(event, handler);
    return () => h.listeners.delete(event);
  },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));

import { useJoinRunningSummary, joinedSummaryResult } from '@/hooks/meeting-details/useJoinRunningSummary';

const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

beforeEach(() => {
  vi.clearAllMocks();
  h.listeners.clear();
});

function mount(status: string) {
  h.invoke.mockResolvedValue({ status, data: null });
  const poll = vi.fn();
  const handlers = { onJoin: vi.fn(), onResult: vi.fn() };
  renderHook(() => useJoinRunningSummary('m-1', poll, handlers));
  return { poll, handlers };
}

describe('useJoinRunningSummary', () => {
  it.each(['pending', 'PENDING', 'processing'])('joins a %s run on mount', async (status) => {
    const { poll, handlers } = mount(status);
    await flush();
    expect(h.invoke).toHaveBeenCalledWith('api_get_summary', { meetingId: 'm-1' });
    expect(handlers.onJoin).toHaveBeenCalledTimes(1);
    expect(poll).toHaveBeenCalledWith('m-1', 'm-1', expect.any(Function));
  });

  it.each(['completed', 'idle', 'failed'])('leaves a %s summary alone', async (status) => {
    const { poll } = mount(status);
    await flush();
    expect(poll).not.toHaveBeenCalled();
  });

  it('joins when the wrap-up starts summarizing this meeting, and only this one', async () => {
    const { poll } = mount('idle');
    await flush();
    act(() => h.listeners.get('meeting-wrap-up')!({ payload: { stage: 'summarizing', meetingId: 'other' } }));
    act(() => h.listeners.get('meeting-wrap-up')!({ payload: { stage: 'diarizing', meetingId: 'm-1' } }));
    expect(poll).not.toHaveBeenCalled();
    act(() => h.listeners.get('meeting-wrap-up')!({ payload: { stage: 'summarizing', meetingId: 'm-1' } }));
    expect(poll).toHaveBeenCalledTimes(1);
  });

  it('never starts a summary itself', async () => {
    mount('pending');
    await flush();
    const commands = h.invoke.mock.calls.map(([cmd]) => cmd);
    expect(commands).toEqual(['api_get_summary']);
  });
});

describe('joinedSummaryResult', () => {
  it('shows a completed markdown summary and its title', () => {
    const deps = { showSummary: vi.fn(), setStatus: vi.fn(), setError: vi.fn(), applyTitle: vi.fn() };
    joinedSummaryResult(deps)({ status: 'completed', data: { markdown: '# Notes', MeetingName: 'Pricing' } });
    expect(deps.showSummary).toHaveBeenCalledWith({ markdown: '# Notes', summary_status: undefined });
    expect(deps.setStatus).toHaveBeenCalledWith('completed');
    expect(deps.applyTitle).toHaveBeenCalledWith('Pricing');
  });

  it('reports a failure', () => {
    const deps = { showSummary: vi.fn(), setStatus: vi.fn(), setError: vi.fn(), applyTitle: vi.fn() };
    joinedSummaryResult(deps)({ status: 'failed', error: 'Ollama is not running' });
    expect(deps.setError).toHaveBeenCalledWith('Ollama is not running');
    expect(deps.setStatus).toHaveBeenCalledWith('error');
  });
});
