'use client';

/**
 * Keyboard model for the To-do list (specs/0088 follow-up), modelled on how the owner
 * drives OneNote. Bound to the list container (a focusable listbox), and ignored while a
 * row is being edited — the edit field owns its keys then.
 *
 *   ↑ / ↓         move the selection (nothing selected: ↓ picks the first, ↑ the last)
 *   Space         edit the selected item (Enter saves, Esc cancels; it stays selected)
 *   ⌘↑ / ⌘↓       move the selected item one place
 *   ⌘1 / ⌘2 / ⌘3  move it to the first / second / third place
 *   ⌘4 / ⌘5       toggle a yellow / green highlight
 *   ⌘D            mark done / not done (the row stays put until you leave the page)
 *   Esc           clear the selection
 */

import { useCallback, type KeyboardEvent } from 'react';

export type Highlight = 'yellow' | 'green';

/** `ids` with `id` moved to `index` (clamped). Unchanged when `id` is absent. */
export function moveToIndex(ids: readonly string[], id: string, index: number): string[] {
  const from = ids.indexOf(id);
  if (from < 0) return [...ids];
  const next = ids.filter((x) => x !== id);
  next.splice(Math.max(0, Math.min(index, next.length)), 0, id);
  return next;
}

/** ⌘4 / ⌘5 targets: pressing the key for the current colour clears it. */
export const HIGHLIGHT_KEYS: Record<string, Highlight> = { '4': 'yellow', '5': 'green' };

interface Options {
  ids: readonly string[];
  selectedId: string | null;
  editing: boolean;
  select: (id: string | null) => void;
  startEdit: (id: string) => void;
  reorder: (next: string[]) => void;
  toggleHighlight: (id: string, color: Highlight) => void;
  toggleDone: (id: string) => void;
}

export function useTodoListKeys({
  ids,
  selectedId,
  editing,
  select,
  startEdit,
  reorder,
  toggleHighlight,
  toggleDone,
}: Options) {
  return useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      if (editing || e.altKey || e.ctrlKey) return;
      const t = e.target as HTMLElement;
      // Only keys aimed at the list itself or a row in it — not the menu, picker or grip.
      if (t !== e.currentTarget && !t.closest('[role="option"]')) return;
      if (t.closest('input, textarea, [contenteditable="true"]')) return;

      const index = selectedId ? ids.indexOf(selectedId) : -1;
      const handled = () => {
        e.preventDefault();
        e.stopPropagation();
      };

      if (!e.metaKey) {
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          handled();
          if (ids.length === 0) return;
          const down = e.key === 'ArrowDown';
          const next = index < 0 ? (down ? 0 : ids.length - 1) : index + (down ? 1 : -1);
          select(ids[Math.max(0, Math.min(next, ids.length - 1))]!);
        } else if (e.key === ' ' && selectedId) {
          handled();
          startEdit(selectedId);
        } else if (e.key === 'Escape' && selectedId) {
          handled();
          select(null);
        }
        return;
      }

      if (!selectedId || index < 0) return;
      if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
        handled();
        const to = index + (e.key === 'ArrowUp' ? -1 : 1);
        if (to >= 0 && to < ids.length) reorder(moveToIndex(ids, selectedId, to));
      } else if (e.key === '1' || e.key === '2' || e.key === '3') {
        handled();
        const to = Number(e.key) - 1;
        if (to !== index) reorder(moveToIndex(ids, selectedId, to));
      } else if (e.key.toLowerCase() === 'd') {
        handled();
        toggleDone(selectedId);
      } else if (HIGHLIGHT_KEYS[e.key]) {
        handled();
        toggleHighlight(selectedId, HIGHLIGHT_KEYS[e.key]!);
      }
    },
    [editing, ids, selectedId, select, startEdit, reorder, toggleHighlight, toggleDone],
  );
}
