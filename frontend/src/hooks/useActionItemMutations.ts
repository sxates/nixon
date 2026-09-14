'use client';

/**
 * useActionItemMutations (specs/0034) — the action-item mutation handlers shared by
 * the per-meeting section (`components/MeetingDetails/ActionItemsSection.tsx`) and
 * the task hub (`app/tasks/page.tsx`). Each handler does the optimistic local update,
 * the IPC call, and — on failure — a **per-item functional revert** plus an error
 * toast.
 *
 * Per-item reverts matter: the list can be replaced underneath an in-flight mutation
 * by an `action-items-updated` reload (a background extraction commit), so restoring
 * a whole-list snapshot would clobber freshly loaded items and other rows' committed
 * changes. Instead, a failed mutation touches only the mutated row:
 *   - status/edit: merge the pre-mutation item back over the matching row;
 *   - delete: re-insert the item only if a concurrent reload didn't already restore it.
 *
 * Surface-specific behavior (the hub dropping rows that leave its status filter, the
 * meeting page's dismiss toast, re-scoping person-filtered views, "to-do added"
 * toasts) plugs in via the option callbacks — the hook itself never re-queries.
 */

import type { Dispatch, SetStateAction } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import {
  updateAssigneeArgs,
  type AssigneeSelection,
  type BulkFilter,
} from '@/lib/action-items';
import type { ActionItem, ActionItemStatus } from '@/types';

export interface UseActionItemMutationsOptions<T extends ActionItem> {
  setItems: Dispatch<SetStateAction<T[]>>;
  /** Meeting the add row creates items on; null = standalone hub to-do. */
  meetingId: string | null;
  /**
   * Lift a bare IPC `ActionItem` into the surface's row type (default: identity).
   * The hub uses this to supply its meeting join fields for created/reverted rows.
   */
  toRow?: (item: ActionItem) => T;
  /** Runs after a status change commits (e.g. dismiss toast, hub filter scoping). */
  onStatusChanged?: (updated: ActionItem) => void;
  /** Runs after an assignee change commits (e.g. re-scope a person-filtered view). */
  onAssigned?: (updated: ActionItem) => void | Promise<void>;
  /** Runs after a manual add commits (e.g. success toast). */
  onAdded?: (created: ActionItem) => void;
  /** Whether a just-created item belongs in the current view (default: always). */
  appendCreated?: (created: ActionItem) => boolean;
}

export interface ActionItemMutations {
  setStatus: (item: ActionItem, status: ActionItemStatus) => Promise<void>;
  editDescription: (item: ActionItem, description: string) => Promise<void>;
  assign: (item: ActionItem, selection: AssigneeSelection) => Promise<void>;
  deleteItem: (item: ActionItem) => Promise<void>;
  /** Returns false on failure so `AddActionItemRow` keeps the typed draft. */
  add: (description: string) => Promise<boolean>;
  /** Set (ISO `YYYY-MM-DD`) or clear (`null`) an item's structured due date (WS1.a). */
  setDueDate: (item: ActionItem, dueDate: string | null) => Promise<void>;
  /**
   * Persist a manual drag order (WS1.b). Optimistically stamps dense `sortOrder` on the
   * given ids (so a `sortOrder`-keyed Manual sort re-renders in the new order), then
   * writes it via `api_reorder_action_items`; reverts the whole list on failure.
   */
  reorder: (orderedIds: string[]) => Promise<void>;
  /**
   * Bulk status change (WS1.e). Thin IPC wrapper returning the affected count — the
   * caller owns the optimistic list change, id snapshot, and Undo toast (only it knows
   * the current view). Throws on failure so the caller can revert.
   */
  bulkSetStatus: (filter: BulkFilter, status: ActionItemStatus) => Promise<number>;
}

function errorDescription(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function useActionItemMutations<T extends ActionItem>({
  setItems,
  meetingId,
  toRow,
  onStatusChanged,
  onAssigned,
  onAdded,
  appendCreated,
}: UseActionItemMutationsOptions<T>): ActionItemMutations {
  const lift = toRow ?? ((item: ActionItem) => item as T);

  /** Merge base `ActionItem` fields into the matching row, keeping surface-only fields. */
  const mergeItem = (patch: ActionItem) =>
    setItems((prev) => prev.map((i) => (i.id === patch.id ? { ...i, ...patch } : i)));

  const setStatus = async (item: ActionItem, status: ActionItemStatus) => {
    // Optimistic; the IPC return value re-syncs, errors revert this item only.
    setItems((prev) => prev.map((i) => (i.id === item.id ? { ...i, status } : i)));
    try {
      const updated = await invoke<ActionItem>('api_set_action_item_status', {
        id: item.id,
        status,
      });
      mergeItem(updated);
      onStatusChanged?.(updated);
    } catch (err) {
      console.error('Failed to update action item status:', err);
      mergeItem(item);
      toast.error('Could not update the item', { description: errorDescription(err) });
    }
  };

  const editDescription = async (item: ActionItem, description: string) => {
    setItems((prev) => prev.map((i) => (i.id === item.id ? { ...i, description } : i)));
    try {
      const updated = await invoke<ActionItem>('api_update_action_item', {
        id: item.id,
        description,
      });
      mergeItem(updated);
    } catch (err) {
      console.error('Failed to edit action item:', err);
      mergeItem(item);
      toast.error('Could not save the change', { description: errorDescription(err) });
    }
  };

  const assign = async (item: ActionItem, selection: AssigneeSelection) => {
    try {
      const updated = await invoke<ActionItem>(
        'api_update_action_item',
        updateAssigneeArgs(item.id, selection),
      );
      mergeItem(updated);
      await onAssigned?.(updated);
    } catch (err) {
      console.error('Failed to assign action item:', err);
      toast.error('Could not update the assignee', { description: errorDescription(err) });
    }
  };

  const deleteItem = async (item: ActionItem) => {
    setItems((prev) => prev.filter((i) => i.id !== item.id));
    try {
      await invoke('api_delete_action_item', { id: item.id });
    } catch (err) {
      console.error('Failed to delete action item:', err);
      // Re-insert only if a concurrent reload didn't already restore the row.
      const row = lift(item);
      setItems((prev) => (prev.some((i) => i.id === item.id) ? prev : [...prev, row]));
      toast.error('Could not delete the item', { description: errorDescription(err) });
    }
  };

  const setDueDate = async (item: ActionItem, dueDate: string | null) => {
    const next = dueDate || null;
    // Optimistic; `""` clears server-side, an ISO date sets (per WS1.a contract).
    setItems((prev) => prev.map((i) => (i.id === item.id ? { ...i, dueDate: next } : i)));
    try {
      const updated = await invoke<ActionItem>('api_update_action_item', {
        id: item.id,
        dueDate: next ?? '',
      });
      mergeItem(updated);
    } catch (err) {
      console.error('Failed to set action item due date:', err);
      mergeItem(item);
      toast.error('Could not update the due date', { description: errorDescription(err) });
    }
  };

  const reorder = async (orderedIds: string[]) => {
    const rank = new Map(orderedIds.map((id, i) => [id, i]));
    let snapshot: T[] = [];
    setItems((prev) => {
      snapshot = prev;
      return prev.map((i) =>
        rank.has(i.id) ? { ...i, sortOrder: rank.get(i.id)! } : i,
      );
    });
    try {
      await invoke('api_reorder_action_items', { orderedIds });
    } catch (err) {
      console.error('Failed to reorder action items:', err);
      setItems(() => snapshot);
      toast.error('Could not save the new order', { description: errorDescription(err) });
    }
  };

  const bulkSetStatus = async (
    filter: BulkFilter,
    status: ActionItemStatus,
  ): Promise<number> => {
    // Thin wrapper — the caller owns optimistic removal + Undo (view-specific). Rethrows
    // so the caller can revert its optimistic change.
    return invoke<number>('api_bulk_set_action_item_status', { filter, status });
  };

  const add = async (description: string): Promise<boolean> => {
    try {
      const created = await invoke<ActionItem>('api_create_action_item', {
        meetingId,
        description,
      });
      if (!appendCreated || appendCreated(created)) {
        const row = lift(created);
        setItems((prev) => (prev.some((i) => i.id === row.id) ? prev : [...prev, row]));
      }
      onAdded?.(created);
      return true;
    } catch (err) {
      console.error('Failed to add action item:', err);
      toast.error('Could not add the item', { description: errorDescription(err) });
      return false;
    }
  };

  return { setStatus, editDescription, assign, deleteItem, add, setDueDate, reorder, bulkSetStatus };
}
