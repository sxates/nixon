import { describe, it, expect } from 'vitest';
import { placeInSlots, sameOrder, sidebarTopics, slotSortOrders, sortTopics } from '../topic-order';
import type { TopicSummary } from '@/types';

const topic = (id: string, over: Partial<TopicSummary> = {}): TopicSummary => ({
  id, name: id.toUpperCase(), description: '', status: 'open', lastActivity: null,
  pendingCount: 0, meetingCount: 0, sortOrder: null, starred: false, hideFromTodo: false,
  ...over,
});

describe('slotSortOrders (the spec 0090 acceptance case)', () => {
  it('X1, T1, Y1, T2, T3 with T reordered to T3, T1, T2 gives X1, T3, Y1, T1, T2', () => {
    const items = [
      { id: 'x1', sortOrder: 0 }, { id: 't1', sortOrder: 1 }, { id: 'y1', sortOrder: 2 },
      { id: 't2', sortOrder: 3 }, { id: 't3', sortOrder: 4 },
    ];
    const slots = slotSortOrders(items, ['t3', 't1', 't2']);
    const next = items.map((i) => ({ ...i, sortOrder: slots.get(i.id) ?? i.sortOrder }));
    expect(next.sort((a, b) => a.sortOrder - b.sortOrder).map((i) => i.id)).toEqual([
      'x1', 't3', 'y1', 't1', 't2',
    ]);
  });

  it('gives an unordered item max + 1 before swapping, and ignores unknown ids', () => {
    const items = [{ id: 'a', sortOrder: 5 }, { id: 'b', sortOrder: null }, { id: 'c', sortOrder: 2 }];
    const slots = slotSortOrders(items, ['b', 'zzz', 'a']);
    expect([...slots.entries()]).toEqual([['b', 5], ['a', 6]]);
  });
});

describe('placeInSlots', () => {
  it('reorders a subset inside the places it holds; others stay put', () => {
    expect(placeInSlots(['o1', 'a1', 'o2', 'a2'], ['o2', 'o1'])).toEqual(['o2', 'a1', 'o1', 'a2']);
  });
});

describe('sortTopics / sidebarTopics', () => {
  it('sorts by sortOrder, unordered last by name', () => {
    const sorted = sortTopics([topic('b'), topic('c', { sortOrder: 1 }), topic('a'), topic('d', { sortOrder: 0 })]);
    expect(sorted.map((t) => t.id)).toEqual(['d', 'c', 'a', 'b']);
  });

  it('lists only starred, open topics', () => {
    const list = sidebarTopics([
      topic('a', { starred: true, sortOrder: 2 }),
      topic('b', { starred: true, sortOrder: 1, status: 'closed' }),
      topic('c', { sortOrder: 0 }),
      topic('d', { starred: true, sortOrder: 0 }),
    ]);
    expect(list.map((t) => t.id)).toEqual(['d', 'a']);
    expect(sidebarTopics(null)).toEqual([]);
  });

  it('sameOrder', () => {
    expect(sameOrder(['a', 'b'], ['a', 'b'])).toBe(true);
    expect(sameOrder(['a', 'b'], ['b', 'a'])).toBe(false);
  });
});
