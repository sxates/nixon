import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { useState } from 'react';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
const toastError = vi.fn();
vi.mock('sonner', () => ({ toast: { error: (...a: unknown[]) => toastError(...a), success: vi.fn() } }));

import { useTodoGroupActions } from '@/hooks/useTodoGroupActions';
import { reorderActionItemsWithin } from '@/lib/topics';
import type { TodoRow, TopicSummary } from '@/types';

// specs/0090 — the grouped To-Do moves: cross-section re-tag and topic-section reorder.

const topic = (id: string, sortOrder: number, status: 'open' | 'closed' = 'open'): TopicSummary => ({
  id, name: `Topic ${id}`, description: '', status, lastActivity: null, pendingCount: 0,
  meetingCount: 0, sortOrder, starred: false, hideFromTodo: false,
});
const row = (id: string, topicId: string | null, sortOrder: number) =>
  ({ id, topicId, sortOrder, status: 'open' }) as unknown as TodoRow;

const TOPICS = [topic('a', 0), topic('quiet', 1), topic('b', 2), topic('old', 3, 'closed')];

function setup(rows: TodoRow[]) {
  return renderHook(() => {
    const [items, setItems] = useState(rows);
    const actions = useTodoGroupActions({
      setItems,
      allTopics: TOPICS,
      reorderWithin: (ids) => reorderActionItemsWithin(ids),
    });
    return { items, ...actions };
  });
}

beforeEach(() => {
  invokeMock.mockReset().mockResolvedValue(undefined);
  toastError.mockReset();
});

describe('moveToTopic', () => {
  it('re-tags, then reorders the target section (moved item included), in that order', async () => {
    const { result } = setup([row('x', null, 0), row('t1', 'a', 1), row('y', null, 2), row('t2', 'a', 3)]);
    await act(() => result.current.moveToTopic('x', 'a', ['t1', 'x', 't2']));
    expect(invokeMock.mock.calls.map((c) => c[0])).toEqual([
      'api_set_action_item_topic',
      'api_reorder_action_items_within',
    ]);
    expect(invokeMock.mock.calls[0][1]).toEqual({ id: 'x', topicId: 'a' });
    expect(invokeMock.mock.calls[1][1]).toEqual({ orderedIds: ['t1', 'x', 't2'] });
    const moved = result.current.items.find((i) => i.id === 'x')!;
    expect(moved.topicId).toBe('a');
    // The target's slots were 0 (x's own), 1, 3: t1 takes 0, x 1, t2 3.
    expect(result.current.items.map((i) => [i.id, i.sortOrder])).toEqual([['x', 1], ['t1', 0], ['y', 2], ['t2', 3]]);
  });

  it('sends null for No Topic', async () => {
    const { result } = setup([row('t1', 'a', 0), row('n', null, 1)]);
    await act(() => result.current.moveToTopic('t1', null, ['n', 't1']));
    expect(invokeMock.mock.calls[0]).toEqual(['api_set_action_item_topic', { id: 't1', topicId: null }]);
  });

  it('a refused re-tag restores the list, toasts, and skips the reorder', async () => {
    invokeMock.mockRejectedValueOnce(new Error('closed topic'));
    const rows = [row('x', null, 0), row('t1', 'a', 1)];
    const { result } = setup(rows);
    await act(() => result.current.moveToTopic('x', 'a', ['x', 't1']));
    expect(result.current.items).toEqual(rows);
    expect(toastError).toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});

describe('reorderTopics', () => {
  it('moves the section topics within their own places; others keep theirs', () => {
    const { result } = setup([]);
    act(() => result.current.reorderTopics(['b', 'a']));
    expect(invokeMock).toHaveBeenCalledWith('api_reorder_topics', { orderedIds: ['b', 'quiet', 'a', 'old'] });
    expect(result.current.topics.find((t) => t.id === 'b')!.sortOrder).toBe(0);
  });

  it('a failed save reverts and toasts', async () => {
    invokeMock.mockRejectedValueOnce(new Error('nope'));
    const { result } = setup([]);
    await act(async () => result.current.reorderTopics(['b', 'a']));
    expect(result.current.topics.find((t) => t.id === 'b')!.sortOrder).toBe(2);
    expect(toastError).toHaveBeenCalled();
  });
});
