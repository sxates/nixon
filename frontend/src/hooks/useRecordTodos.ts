'use client';

/**
 * The record screen's To-Do rail data (Focus view): either one Topic's open To-Dos
 * (`api_get_topic`'s `todos`, the list the Topic page shows) or, with no Topic, the To-Do
 * list's items that have none (`api_list_todo_view` 'list', `topicId == null`).
 *
 * Refetches on `action-items-updated` (every to-do mutation, extraction) and `topics-updated`
 * (tagging, a topic's "Show in To-Do"), the events the To-Do and Topic pages follow. Creating
 * an item emits neither, so the rail reloads after its own adds.
 *
 * A row ticked off here stays on screen, struck through, until the rail remounts or switches
 * list (the To-Do page's rule): a reload keeps the locally completed rows it no longer returns.
 */

import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';
import { getTopic } from '@/lib/topics';
import type { TodoRow } from '@/types';

export interface RecordTodos {
  items: TodoRow[];
  setItems: Dispatch<SetStateAction<TodoRow[]>>;
  /** False until the first load for this list settles. */
  loaded: boolean;
  error: string | null;
  reload: () => Promise<void>;
}

async function fetchRows(topicId: string | null): Promise<TodoRow[]> {
  const rows = topicId
    ? (await getTopic(topicId))?.todos
    : (await invoke<TodoRow[]>('api_list_todo_view', { view: 'list' }))?.filter((r) => r.topicId == null);
  return (Array.isArray(rows) ? rows : []).filter((r) => r.status === 'open');
}

/** `enabled` false (no meeting, or its Topics still loading) loads nothing. */
export function useRecordTodos(topicId: string | null, enabled: boolean): RecordTodos {
  const [items, setItems] = useState<TodoRow[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Only the latest request may write state (events can overlap a slow load).
  const seq = useRef(0);

  const reload = useCallback(async () => {
    if (!enabled) return;
    const mine = ++seq.current;
    try {
      const fresh = await fetchRows(topicId);
      if (mine !== seq.current) return;
      const ids = new Set(fresh.map((r) => r.id));
      setItems((prev) => [...fresh, ...prev.filter((p) => p.status === 'completed' && !ids.has(p.id))]);
      setError(null);
    } catch (err) {
      console.error('Failed to load the recording’s To-Dos:', err);
      if (mine === seq.current) setError('Could not load the To-Dos.');
    } finally {
      if (mine === seq.current) setLoaded(true);
    }
  }, [topicId, enabled]);

  // A different list starts empty rather than showing the previous one's rows.
  useEffect(() => {
    setItems([]);
    setLoaded(false);
    setError(null);
    void reload();
  }, [reload]);

  useEffect(() => safeListen('action-items-updated', () => void reload()), [reload]);
  useEffect(() => safeListen('topics-updated', () => void reload()), [reload]);

  return { items, setItems, loaded, error, reload };
}
