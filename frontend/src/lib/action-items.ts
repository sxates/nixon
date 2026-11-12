/**
 * Pure helpers for the action-items feature (specs/0034, 0088): assignee IPC args and
 * display resolution, drag reordering, and bulk-dismiss selectors. Kept free of
 * React/Tauri so the hub's core logic is unit-testable (Vitest, no IPC).
 */

import type { ActionItem } from '@/types';

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

/**
 * Who an adopted item came from (specs/0088), or null. Adopting keeps the original
 * assignee, so a listed item that is assigned to someone else is one the user took on.
 */
export function adoptedFromName(
  item: ActionItem,
  nameById: ReadonlyMap<string, string>,
): string | null {
  if (item.triage !== 'listed' || item.assigneeIsSelf) return null;
  if (!item.assigneePersonId && !item.assigneeRaw?.trim()) return null;
  return assigneeLabel(item, nameById);
}
