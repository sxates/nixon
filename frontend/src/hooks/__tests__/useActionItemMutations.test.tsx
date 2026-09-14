import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { useState } from 'react';
import {
  useActionItemMutations,
  type UseActionItemMutationsOptions,
} from '@/hooks/useActionItemMutations';
import type { ActionItem } from '@/types';

// specs/0034 — the mutation hook shared by the per-meeting section and the task
// hub. Locks the per-item revert semantics: a failed mutation restores ONLY the
// mutated row, so it can never clobber a concurrent `action-items-updated` reload
// (the old whole-list snapshot revert did exactly that).

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';

const invokeMock = vi.mocked(invoke);
const toastError = vi.mocked(toast.error);

function makeItem(overrides: Partial<ActionItem> = {}): ActionItem {
  return {
    id: 'a',
    meetingId: 'm1',
    description: 'Item A',
    assigneePersonId: null,
    assigneeIsSelf: false,
    assigneeRaw: null,
    dueHint: null,
    dueDate: null,
    status: 'open',
    source: 'extracted',
    userEdited: false,
    contentKey: 'ck-a',
    createdAt: '2026-07-01T10:00:00Z',
    updatedAt: '2026-07-01T10:00:00Z',
    completedAt: null,
    sortOrder: null,
    ...overrides,
  };
}

const A = makeItem();
const B = makeItem({ id: 'b', description: 'Item B', contentKey: 'ck-b' });

function setup(
  initial: ActionItem[],
  options: Partial<UseActionItemMutationsOptions<ActionItem>> = {},
) {
  return renderHook(() => {
    const [items, setItems] = useState<ActionItem[]>(initial);
    const mutations = useActionItemMutations<ActionItem>({
      setItems,
      meetingId: 'm1',
      ...options,
    });
    return { items, setItems, mutations };
  });
}

/** A controllable pending invoke for interleaving tests. */
function pendingInvoke() {
  let resolve!: (value: unknown) => void;
  let reject!: (err: unknown) => void;
  invokeMock.mockImplementation(
    () =>
      new Promise((res, rej) => {
        resolve = res;
        reject = rej;
      }),
  );
  return { resolve: (v: unknown) => resolve(v), reject: (e: unknown) => reject(e) };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('useActionItemMutations (specs/0034)', () => {
  it('setStatus: optimistic update, then merges the IPC result and notifies', async () => {
    const updated = { ...A, status: 'completed' as const, completedAt: '2026-07-02T00:00:00Z' };
    invokeMock.mockResolvedValue(updated);
    const onStatusChanged = vi.fn();
    const { result } = setup([A, B], { onStatusChanged });

    await act(() => result.current.mutations.setStatus(A, 'completed'));

    expect(invokeMock).toHaveBeenCalledWith('api_set_action_item_status', {
      id: 'a',
      status: 'completed',
    });
    expect(result.current.items).toEqual([updated, B]);
    expect(onStatusChanged).toHaveBeenCalledWith(updated);
  });

  it('setStatus failure reverts only the mutated item, preserving an interleaved reload', async () => {
    const pending = pendingInvoke();
    const { result } = setup([A]);

    let call!: Promise<void>;
    act(() => {
      call = result.current.mutations.setStatus(A, 'completed');
    });
    // Optimistic flip while the IPC call is in flight.
    expect(result.current.items[0]!.status).toBe('completed');

    // A concurrent `action-items-updated` reload replaces the list (A + new B).
    act(() => result.current.setItems([A, B]));

    await act(async () => {
      pending.reject(new Error('boom'));
      await call;
    });

    // Per-item revert: A restored, the concurrently loaded B survives.
    expect(result.current.items).toEqual([A, B]);
    expect(toastError).toHaveBeenCalledWith('Could not update the item', {
      description: 'boom',
    });
  });

  it('editDescription failure reverts only the edited item', async () => {
    invokeMock.mockRejectedValue(new Error('nope'));
    const { result } = setup([A, B]);

    await act(() => result.current.mutations.editDescription(A, 'Changed'));

    expect(result.current.items).toEqual([A, B]);
    expect(toastError).toHaveBeenCalledWith('Could not save the change', {
      description: 'nope',
    });
  });

  it('assign maps the selection through updateAssigneeArgs and merges the result', async () => {
    const updated = { ...A, assigneeIsSelf: true };
    invokeMock.mockResolvedValue(updated);
    const onAssigned = vi.fn();
    const { result } = setup([A, B], { onAssigned });

    await act(() => result.current.mutations.assign(A, { kind: 'me' }));

    expect(invokeMock).toHaveBeenCalledWith('api_update_action_item', {
      id: 'a',
      assigneeIsSelf: true,
    });
    expect(result.current.items).toEqual([updated, B]);
    expect(onAssigned).toHaveBeenCalledWith(updated);
  });

  it('deleteItem failure re-inserts the item — unless a reload already restored it', async () => {
    // Plain failure: the row comes back.
    invokeMock.mockRejectedValue(new Error('locked'));
    const { result } = setup([A, B]);
    await act(() => result.current.mutations.deleteItem(A));
    expect(result.current.items.map((i) => i.id).sort()).toEqual(['a', 'b']);

    // Failure racing a reload that already restored the row: no duplicate.
    const pending = pendingInvoke();
    let call!: Promise<void>;
    act(() => {
      call = result.current.mutations.deleteItem(A);
    });
    act(() => result.current.setItems([A, B]));
    await act(async () => {
      pending.reject(new Error('locked'));
      await call;
    });
    expect(result.current.items.filter((i) => i.id === 'a')).toHaveLength(1);
  });

  it('add appends the created item (via toRow) and returns true', async () => {
    const created = makeItem({ id: 'new', description: 'New', source: 'manual' });
    invokeMock.mockResolvedValue(created);
    const onAdded = vi.fn();
    const toRow = vi.fn((item: ActionItem) => item);
    const { result } = setup([A], { onAdded, toRow });

    let ok = false;
    await act(async () => {
      ok = await result.current.mutations.add('New');
    });

    expect(ok).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith('api_create_action_item', {
      meetingId: 'm1',
      description: 'New',
    });
    expect(toRow).toHaveBeenCalledWith(created);
    expect(result.current.items).toEqual([A, created]);
    expect(onAdded).toHaveBeenCalledWith(created);
  });

  // specs/0038 WS1.d — Home "Add to-do" creates a STANDALONE item (no meetingId).
  it('add with meetingId null creates a standalone to-do', async () => {
    const created = makeItem({ id: 'todo', meetingId: null, source: 'manual' });
    invokeMock.mockResolvedValue(created);
    const { result } = setup([], { meetingId: null, appendCreated: () => false });

    await act(async () => {
      await result.current.mutations.add('Buy milk');
    });

    expect(invokeMock).toHaveBeenCalledWith('api_create_action_item', {
      meetingId: null,
      description: 'Buy milk',
    });
  });

  it('add respects appendCreated=false (created item outside the current view)', async () => {
    const created = makeItem({ id: 'new', description: 'New', source: 'manual' });
    invokeMock.mockResolvedValue(created);
    const { result } = setup([A], { appendCreated: () => false });

    await act(async () => {
      await result.current.mutations.add('New');
    });

    expect(result.current.items).toEqual([A]);
  });

  it('add returns false on failure (the add row keeps its draft)', async () => {
    invokeMock.mockRejectedValue(new Error('db locked'));
    const { result } = setup([A]);

    let ok = true;
    await act(async () => {
      ok = await result.current.mutations.add('New');
    });

    expect(ok).toBe(false);
    expect(result.current.items).toEqual([A]);
    expect(toastError).toHaveBeenCalledWith('Could not add the item', {
      description: 'db locked',
    });
  });

  // specs/0038 WS1.a — structured due date.
  it('setDueDate: optimistic, dispatches api_update_action_item with the ISO date', async () => {
    const updated = { ...A, dueDate: '2026-07-10' };
    invokeMock.mockResolvedValue(updated);
    const { result } = setup([A, B]);

    await act(() => result.current.mutations.setDueDate(A, '2026-07-10'));

    expect(invokeMock).toHaveBeenCalledWith('api_update_action_item', {
      id: 'a',
      dueDate: '2026-07-10',
    });
    expect(result.current.items[0]!.dueDate).toBe('2026-07-10');
  });

  it('setDueDate: clearing sends "" (the clear sentinel)', async () => {
    invokeMock.mockResolvedValue({ ...A, dueDate: null });
    const { result } = setup([{ ...A, dueDate: '2026-07-10' }]);

    await act(() => result.current.mutations.setDueDate(A, null));

    expect(invokeMock).toHaveBeenCalledWith('api_update_action_item', {
      id: 'a',
      dueDate: '',
    });
  });

  // specs/0038 WS1.b — manual drag order.
  it('reorder: stamps dense sortOrder optimistically and persists the id order', async () => {
    invokeMock.mockResolvedValue(0);
    const { result } = setup([A, B]);

    await act(() => result.current.mutations.reorder(['b', 'a']));

    expect(invokeMock).toHaveBeenCalledWith('api_reorder_action_items', {
      orderedIds: ['b', 'a'],
    });
    const byId = Object.fromEntries(result.current.items.map((i) => [i.id, i.sortOrder]));
    expect(byId).toEqual({ b: 0, a: 1 });
  });

  it('reorder: reverts the sortOrder stamp on failure', async () => {
    invokeMock.mockRejectedValue(new Error('locked'));
    const { result } = setup([A, B]);

    await act(() => result.current.mutations.reorder(['b', 'a']));

    expect(result.current.items).toEqual([A, B]);
    expect(toastError).toHaveBeenCalledWith('Could not save the new order', {
      description: 'locked',
    });
  });

  // specs/0038 WS1.e — bulk status wrapper.
  it('bulkSetStatus: dispatches the filter + status and returns the count', async () => {
    invokeMock.mockResolvedValue(3);
    const { result } = setup([A]);

    let count = 0;
    await act(async () => {
      count = await result.current.mutations.bulkSetStatus({ mode: 'notSelf' }, 'dismissed');
    });

    expect(invokeMock).toHaveBeenCalledWith('api_bulk_set_action_item_status', {
      filter: { mode: 'notSelf' },
      status: 'dismissed',
    });
    expect(count).toBe(3);
  });
});
