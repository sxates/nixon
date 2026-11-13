import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor, act } from '@testing-library/react';

const h = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  handler: null as null | ((e: { payload: { topic_id: string | null } }) => void),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invokeMock }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (_name: string, cb: (e: { payload: { topic_id: string | null } }) => void) => {
    h.handler = cb;
    return () => {};
  },
}));

import { formatDecisionDate, getTopic, unlinkTopic, setActionItemTopic } from '../topics';
import { useTopic, useTopics } from '@/hooks/useTopics';

beforeEach(() => {
  vi.clearAllMocks();
  h.handler = null;
});

describe('formatDecisionDate', () => {
  it('formats an RFC 3339 time as "Mon D"', () => {
    expect(formatDecisionDate(new Date(2026, 10, 3, 12).toISOString())).toBe('Nov 3');
  });
  it('returns an empty string for junk', () => {
    expect(formatDecisionDate('not a date')).toBe('');
  });
});

describe('wrappers', () => {
  it('map to the command names and camelCase args', async () => {
    h.invokeMock.mockResolvedValue(undefined);
    await getTopic('t1');
    expect(h.invokeMock).toHaveBeenLastCalledWith('api_get_topic', { id: 't1' });
    await unlinkTopic('t1', 'series', 'k');
    expect(h.invokeMock).toHaveBeenLastCalledWith('api_unlink_topic', { topicId: 't1', kind: 'series', ref: 'k' });
    await setActionItemTopic('a1', null);
    expect(h.invokeMock).toHaveBeenLastCalledWith('api_set_action_item_topic', { id: 'a1', topicId: null });
  });
});

describe('useTopic', () => {
  it('re-fetches on a matching event and ignores another topic', async () => {
    h.invokeMock.mockResolvedValue({ topic: { id: 't1' } });
    const { result } = renderHook(() => useTopic('t1'));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(h.invokeMock).toHaveBeenCalledTimes(1);

    await act(async () => h.handler!({ payload: { topic_id: 't2' } }));
    expect(h.invokeMock).toHaveBeenCalledTimes(1);

    await act(async () => h.handler!({ payload: { topic_id: 't1' } }));
    await waitFor(() => expect(h.invokeMock).toHaveBeenCalledTimes(2));

    await act(async () => h.handler!({ payload: { topic_id: null } }));
    await waitFor(() => expect(h.invokeMock).toHaveBeenCalledTimes(3));
  });

  it('reports a user-facing error', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    h.invokeMock.mockRejectedValue('Topic not found.');
    const { result } = renderHook(() => useTopic('t1'));
    await waitFor(() => expect(result.current.error).toBe('Topic not found.'));
    expect(result.current.data).toBeNull();
  });
});

describe('useTopics', () => {
  it('passes the status filter and re-fetches on any event', async () => {
    h.invokeMock.mockResolvedValue([]);
    const { result } = renderHook(() => useTopics('open'));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(h.invokeMock).toHaveBeenCalledWith('api_list_topics', { status: 'open' });
    await act(async () => h.handler!({ payload: { topic_id: 'tX' } }));
    await waitFor(() => expect(h.invokeMock).toHaveBeenCalledTimes(2));
  });
});
