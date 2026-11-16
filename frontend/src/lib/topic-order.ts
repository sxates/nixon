/**
 * Ordering helpers for specs/0090: the Topics page's manual order, and the "slot" rule a
 * subset reorder follows. Pure (no React, no IPC) so it is unit-testable.
 *
 * The slot rule: a subset of a list (a topic's to-dos, one section on To-do, the open
 * topics among all topics) is reordered by handing the subset the positions it already
 * holds, so nothing outside the subset moves. The backend applies the same rule in
 * `reorder_within`; these mirror it for the optimistic update.
 */

import type { TopicSummary } from '@/types';

interface Ordered {
  id: string;
  name: string;
  sortOrder: number | null;
}

/** Manual order: `sortOrder` ascending, unordered last, then name (as `Topics::list`). */
export function sortTopics<T extends Ordered>(topics: readonly T[]): T[] {
  return [...topics].sort(
    (a, b) =>
      (a.sortOrder ?? Infinity) - (b.sortOrder ?? Infinity) ||
      a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
  );
}

/** The sidebar's topics (owner decision 2026-11-16): starred and open, in manual order. */
export function sidebarTopics(topics: readonly TopicSummary[] | null): TopicSummary[] {
  return sortTopics((topics ?? []).filter((t) => t.starred && t.status === 'open'));
}

/**
 * Put `subset` (in its new order) into the slots its ids hold in `all`; every other id
 * keeps its place. Ids in `subset` that aren't in `all` are ignored.
 */
export function placeInSlots(all: readonly string[], subset: readonly string[]): string[] {
  const members = new Set(subset.filter((id) => all.includes(id)));
  const queue = subset.filter((id) => members.has(id));
  let next = 0;
  return all.map((id) => (members.has(id) ? queue[next++] : id));
}

/**
 * The new `sortOrder` per id when `orderedIds` is reordered within its own slots: the
 * subset's current values, ascending, handed out in the new order. An item with no
 * `sortOrder` first gets `max + 1, max + 2, …` in the given order (the backend's rule).
 */
export function slotSortOrders(
  items: readonly { id: string; sortOrder?: number | null }[],
  orderedIds: readonly string[],
): Map<string, number> {
  const byId = new Map(items.map((i) => [i.id, i.sortOrder ?? null]));
  let max = Math.max(-1, ...items.map((i) => i.sortOrder ?? -1));
  const slots = orderedIds
    .filter((id) => byId.has(id))
    .map((id) => byId.get(id) ?? ++max)
    .sort((a, b) => a - b);
  const ids = orderedIds.filter((id) => byId.has(id));
  return new Map(ids.map((id, i) => [id, slots[i]]));
}

/** True when two id lists are the same order (a drop that changed nothing). */
export const sameOrder = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((id, i) => id === b[i]);
