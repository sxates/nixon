import { afterEach, describe, it, expect, vi } from 'vitest';
import { GROUP_BY_TOPIC_KEY, groupByTopic, readGroupByTopic, writeGroupByTopic } from '../todo-groups';
import type { TopicSummary } from '@/types';

const topic = (id: string, sortOrder: number, status: 'open' | 'closed' = 'open'): TopicSummary => ({
  id, name: `Topic ${id}`, description: '', status, lastActivity: null, pendingCount: 0,
  meetingCount: 0, sortOrder, starred: false, hideFromTodo: false,
});

describe('groupByTopic', () => {
  it('sections in topic order (open, then archived), then No Topic; empty ones left out', () => {
    const topics = [topic('a', 2), topic('b', 0, 'closed'), topic('c', 1), topic('empty', 3)];
    const rows = [
      { id: '1', topicId: 'b' }, { id: '2', topicId: null }, { id: '3', topicId: 'a' },
      { id: '4', topicId: 'c' }, { id: '5', topicId: 'a' }, { id: '6', topicId: 'gone' },
    ];
    const groups = groupByTopic(rows, topics);
    expect(groups.map((g) => [g.topicId, g.rows.map((r) => r.id)])).toEqual([
      ['c', ['4']],
      ['a', ['3', '5']],
      ['b', ['1']],
      [null, ['2', '6']],
    ]);
    expect(groups.at(-1)!.name).toBe('No Topic');
  });
});

describe('group-by-topic preference', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.removeItem(GROUP_BY_TOPIC_KEY);
  });

  it('round-trips through localStorage under a namespaced key', () => {
    expect(readGroupByTopic()).toBe(false);
    writeGroupByTopic(true);
    expect(window.localStorage.getItem(GROUP_BY_TOPIC_KEY)).toBe('1');
    expect(readGroupByTopic()).toBe(true);
  });

  it('falls back to off when storage throws', () => {
    vi.spyOn(window, 'localStorage', 'get').mockImplementation(() => {
      throw new Error('blocked');
    });
    expect(() => writeGroupByTopic(true)).not.toThrow();
    expect(readGroupByTopic()).toBe(false);
  });
});
