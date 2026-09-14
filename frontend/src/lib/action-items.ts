/**
 * Pure helpers for the action-items feature (specs/0034): the task hub's filter →
 * IPC-args mapping, meeting grouping, and assignee display resolution. Kept free of
 * React/Tauri so the hub's core logic is unit-testable (Vitest, no IPC).
 */

import type { ActionItem, ActionItemStatus, ActionItemWithMeeting } from '@/types';

/** The hub's status filter — one of the three persisted statuses (default 'open'). */
export type StatusFilter = ActionItemStatus;

/** The hub's person filter: everyone, the app owner ("Me"), or a specific person. */
export type PersonFilter =
  | { kind: 'anyone' }
  | { kind: 'me' }
  | { kind: 'person'; personId: string };

/** Invoke args for `api_list_action_items` from the hub's filter state. */
export function buildListArgs(
  status: StatusFilter,
  person: PersonFilter,
): { status: StatusFilter; personId: string | null; mineOnly: boolean } {
  return {
    status,
    personId: person.kind === 'person' ? person.personId : null,
    mineOnly: person.kind === 'me',
  };
}

/** What the assignee picker resolved to. `raw` = free-text name (kept verbatim). */
export type AssigneeSelection =
  | { kind: 'me' }
  | { kind: 'person'; personId: string }
  | { kind: 'raw'; name: string }
  | { kind: 'clear' };

/** Invoke args for `api_update_action_item` from an assignee-picker selection. */
export function updateAssigneeArgs(
  id: string,
  selection: AssigneeSelection,
): Record<string, unknown> {
  switch (selection.kind) {
    case 'me':
      return { id, assigneeIsSelf: true };
    case 'person':
      return { id, assigneePersonId: selection.personId };
    case 'raw':
      return { id, assigneeRaw: selection.name };
    case 'clear':
      return { id, assigneeClear: true };
  }
}

/**
 * Hub ordering (specs/0038 WS1.b). `meeting` is the grouped default (by source meeting,
 * newest-first — `groupActionItems`); `manual` and `due` are flat orders computed
 * client-side over `sortOrder` / `dueDate` (the server list order is unchanged).
 */
export type HubSort = 'manual' | 'due' | 'meeting';

/** The two flat (ungrouped) sorts — `meeting` renders grouped instead. */
export type FlatSort = Exclude<HubSort, 'meeting'>;

/**
 * Flat client-side ordering for the hub's Manual / Due sorts (specs/0038 WS1.b):
 *   - `manual`: by `sortOrder` ascending, NULLs last, then `createdAt` (oldest first);
 *   - `due`:    by `dueDate` ascending (soonest first), NULLs last, then `createdAt`.
 * Pure and stable; never mutates the input.
 */
export function sortActionItems(
  items: ActionItemWithMeeting[],
  sort: FlatSort,
): ActionItemWithMeeting[] {
  const byCreatedAt = (a: ActionItem, b: ActionItem) =>
    (a.createdAt || '').localeCompare(b.createdAt || '');
  const copy = [...items];
  if (sort === 'manual') {
    copy.sort((a, b) => {
      const ao = a.sortOrder;
      const bo = b.sortOrder;
      if (ao != null && bo != null && ao !== bo) return ao - bo;
      if (ao != null && bo == null) return -1;
      if (ao == null && bo != null) return 1;
      return byCreatedAt(a, b);
    });
  } else {
    copy.sort((a, b) => {
      const ad = a.dueDate;
      const bd = b.dueDate;
      if (ad && bd && ad !== bd) return ad.localeCompare(bd);
      if (ad && !bd) return -1;
      if (!ad && bd) return 1;
      return byCreatedAt(a, b);
    });
  }
  return copy;
}

/**
 * Move `draggingId` to `targetId`'s slot within an id list (specs/0038 WS1.b drag).
 * Returns a new array; a no-op (same id, or either id missing) returns the input order.
 */
export function reorderIds(
  ids: readonly string[],
  draggingId: string,
  targetId: string,
): string[] {
  if (draggingId === targetId) return [...ids];
  const from = ids.indexOf(draggingId);
  const to = ids.indexOf(targetId);
  if (from < 0 || to < 0) return [...ids];
  const next = [...ids];
  next.splice(from, 1);
  next.splice(to, 0, draggingId);
  return next;
}

/**
 * Target selector for `api_bulk_set_action_item_status` (specs/0038 WS1.e). Tagged to
 * mirror the Rust `BulkFilterArg` (serde `tag = "mode"`, camelCase): dismiss everyone but
 * me, a specific person, or an explicit id set (the Undo primitive).
 */
export type BulkFilter =
  | { mode: 'person'; personId: string }
  | { mode: 'notSelf' }
  | { mode: 'ids'; ids: string[] };

/**
 * The ids a bulk filter would affect in the CURRENT view — snapshotted client-side
 * before the bulk call so an Undo can re-`open` exactly them (`{ mode: 'ids' }`). Mirrors
 * the backend: person/notSelf only touch OPEN items and never my own (`assigneeIsSelf`).
 */
export function affectedByBulk(items: ActionItem[], filter: BulkFilter): string[] {
  switch (filter.mode) {
    case 'person':
      return items
        .filter((i) => i.status === 'open' && i.assigneePersonId === filter.personId)
        .map((i) => i.id);
    case 'notSelf':
      return items
        .filter((i) => i.status === 'open' && !i.assigneeIsSelf)
        .map((i) => i.id);
    case 'ids':
      return [...filter.ids];
  }
}

/** One hub group: a source meeting (or the standalone "Unattached" bucket). */
export interface ActionItemGroup {
  /** Stable render key: the meeting id, or 'unattached'. */
  key: string;
  meetingId: string | null;
  title: string;
  /** Meeting creation time (ISO) — null for the unattached group. */
  createdAt: string | null;
  items: ActionItemWithMeeting[];
}

export const UNATTACHED_GROUP_TITLE = 'Unattached';

/**
 * Group hub items by source meeting: meetings newest-first (by meeting `created_at`),
 * standalone items (`meetingId === null`) in a trailing "Unattached" group. Within a
 * group, items keep oldest-first order (by item `createdAt`) so lists read stably.
 */
export function groupActionItems(items: ActionItemWithMeeting[]): ActionItemGroup[] {
  const byMeeting = new Map<string, ActionItemGroup>();
  const unattached: ActionItemWithMeeting[] = [];

  for (const item of items) {
    if (!item.meetingId) {
      unattached.push(item);
      continue;
    }
    let group = byMeeting.get(item.meetingId);
    if (!group) {
      group = {
        key: item.meetingId,
        meetingId: item.meetingId,
        title: item.meetingTitle?.trim() || 'Untitled meeting',
        createdAt: item.meetingCreatedAt ?? null,
        items: [],
      };
      byMeeting.set(item.meetingId, group);
    }
    group.items.push(item);
  }

  const byCreatedAtAsc = (a: ActionItem, b: ActionItem) =>
    (a.createdAt || '').localeCompare(b.createdAt || '');

  const groups = Array.from(byMeeting.values());
  for (const group of groups) group.items.sort(byCreatedAtAsc);
  // Meetings newest-first; ISO-8601 UTC strings compare lexicographically.
  groups.sort((a, b) => (b.createdAt || '').localeCompare(a.createdAt || ''));

  if (unattached.length > 0) {
    unattached.sort(byCreatedAtAsc);
    groups.push({
      key: 'unattached',
      meetingId: null,
      title: UNATTACHED_GROUP_TITLE,
      createdAt: null,
      items: unattached,
    });
  }
  return groups;
}

/**
 * Resolve an item's assignee to a display label, or null when unassigned.
 * Priority: "Me" (owner flag) → people-directory name → raw extracted name.
 * A dangling `assigneePersonId` (person deleted) falls back to `assigneeRaw`.
 */
export function assigneeLabel(
  item: ActionItem,
  nameById: ReadonlyMap<string, string>,
): string | null {
  if (item.assigneeIsSelf) return 'Me';
  if (item.assigneePersonId) {
    return nameById.get(item.assigneePersonId) ?? item.assigneeRaw?.trim() ?? 'Unknown';
  }
  const raw = item.assigneeRaw?.trim();
  return raw ? raw : null;
}
