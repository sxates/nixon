'use client';

/**
 * ⌘4 / ⌘5 highlighter toggle for a locally mirrored to-do list (specs/0088): optimistic,
 * rolled back with a toast if the save fails. Pressing the current colour clears it.
 */

import { useCallback, type Dispatch, type SetStateAction } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import type { Highlight } from '@/hooks/useTodoListKeys';
import type { TodoRow } from '@/types';

export function useTodoHighlight(items: TodoRow[], setItems: Dispatch<SetStateAction<TodoRow[]>>) {
  return useCallback(
    async (id: string, color: Highlight) => {
      const current = items.find((i) => i.id === id)?.highlight ?? null;
      const next = current === color ? null : color;
      setItems((prev) => prev.map((i) => (i.id === id ? { ...i, highlight: next } : i)));
      try {
        await invoke('api_set_action_item_highlight', { id, highlight: next });
      } catch (err) {
        setItems((prev) => prev.map((i) => (i.id === id ? { ...i, highlight: current } : i)));
        toast.error('Could not change the highlight', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [items, setItems],
  );
}
