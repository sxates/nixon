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

import {
  formatDecisionDate,
  getTopic,
  unlinkTopic,
  setActionItemTopic,
  updateTopic,
  reorderTopics,
  reorderActionItemsWithin,
} from '../topics';
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

// specs/0090 wire contract. Exact objects (toHaveBeenLastCalledWith fails on an extra or
// missing key), checked against the Rust command signatures, not against this file: 0089
// shipped a `topicId` shape bug because the test mirrored the caller.
describe('0090 wire shape', () => {
  it('api_update_topic sends every field, null when untouched, camelCase hideFromTodo', async () => {
    h.invokeMock.mockResolvedValue({});
    await updateTopic('t1', { starred: true });
    expect(h.invokeMock).toHaveBeenLastCalledWith('api_update_topic', {
      id: 't1', name: null, description: null, status: null, starred: true, hideFromTodo: null,
    });
    await updateTopic('t1', { hideFromTodo: false });
    expect(h.invokeMock).toHaveBeenLastCalledWith('api_update_topic', {
      id: 't1', name: null, description: null, status: null, starred: null, hideFromTodo: false,
    });
  });

  it('api_reorder_topics takes the full id list as orderedIds', async () => {
    h.invokeMock.mockResolvedValue(undefined);
    await reorderTopics(['b', 'a', 'c']);
    expect(h.invokeMock).toHaveBeenLastCalledWith('api_reorder_topics', { orderedIds: ['b', 'a', 'c'] });
  });

  it('api_reorder_action_items_within takes the subset as orderedIds', async () => {
    h.invokeMock.mockResolvedValue(undefined);
    await reorderActionItemsWithin(['t3', 't1']);
    expect(h.invokeMock).toHaveBeenLastCalledWith('api_reorder_action_items_within', {
      orderedIds: ['t3', 't1'],
    });
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
