/**
 * "Group by Topic" on the To-do list (specs/0090): pure section building plus the
 * per-viewer toggle in `localStorage` (a convenience, so every access is guarded).
 */

import type { TopicSummary } from '@/types';
import { sortTopics } from './topic-order';

export interface TodoGroup<T> {
  /** The topic id, or `null` for the "No Topic" section. */
  topicId: string | null;
  /** The heading: the topic's name, or "No Topic". */
  name: string;
  rows: T[];
}

/**
 * One section per topic in manual order (open topics, then archived), then "No Topic".
 * Rows keep their given order inside a section; empty sections are left out. A row whose
 * topic isn't in `topics` (deleted under us) falls into "No Topic".
 */
export function groupByTopic<T extends { topicId?: string | null }>(
  rows: readonly T[],
  topics: readonly TopicSummary[],
): TodoGroup<T>[] {
  const sorted = sortTopics(topics);
  const ordered = [
    ...sorted.filter((t) => t.status === 'open'),
    ...sorted.filter((t) => t.status !== 'open'),
  ];
  const known = new Set(ordered.map((t) => t.id));
  const byTopic = new Map<string | null, T[]>();
  for (const row of rows) {
    const key = row.topicId && known.has(row.topicId) ? row.topicId : null;
    const list = byTopic.get(key) ?? [];
    list.push(row);
    byTopic.set(key, list);
  }
  const groups: TodoGroup<T>[] = ordered
    .filter((t) => byTopic.has(t.id))
    .map((t) => ({ topicId: t.id, name: t.name, rows: byTopic.get(t.id)! }));
  const loose = byTopic.get(null);
  if (loose) groups.push({ topicId: null, name: 'No Topic', rows: loose });
  return groups;
}

export const GROUP_BY_TOPIC_KEY = 'nixon.todo.groupByTopic';

export function readGroupByTopic(): boolean {
  try {
    return window.localStorage.getItem(GROUP_BY_TOPIC_KEY) === '1';
  } catch {
    return false;
  }
}

export function writeGroupByTopic(on: boolean): void {
  try {
    window.localStorage.setItem(GROUP_BY_TOPIC_KEY, on ? '1' : '0');
  } catch {
    // Storage blocked (private window, cleared site data): the toggle just won't stick.
  }
}
