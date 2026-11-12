'use client';

/**
 * useTodoViews (specs/0088) — loads the To-do / Inbox / Done lists (and optionally the
 * dismissed list) through `api_list_todo_view`, plus the inbox badge count, and refetches
 * on the `action-items-updated` event.
 *
 * That event fires from extraction and every mutation (triage, status, bulk status,
 * delete, reorder, notes, dismiss-older, update), so the lists stay current on their own.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';
import type { TodoRow, TodoView } from '@/types';

export interface UseTodoViewsOptions {
  /** Also load the dismissed list (the "Show dismissed" toggle). Default false. */
  includeDismissed?: boolean;
  /** Only fetch the inbox badge count (the sidebar): skips the three list queries. */
  countOnly?: boolean;
}

export interface TodoViews {
  list: TodoRow[];
  inbox: TodoRow[];
  done: TodoRow[];
  dismissed: TodoRow[];
  inboxCount: number;
  /** False until the first load settles. */
  loaded: boolean;
  /** User-facing message when the last load failed; null otherwise. */
  error: string | null;
  reload: () => Promise<void>;
}

const fetchView = (view: TodoView) => invoke<TodoRow[]>('api_list_todo_view', { view });

export function useTodoViews({
  includeDismissed = false,
  countOnly = false,
}: UseTodoViewsOptions = {}): TodoViews {
  const [list, setList] = useState<TodoRow[]>([]);
  const [inbox, setInbox] = useState<TodoRow[]>([]);
  const [done, setDone] = useState<TodoRow[]>([]);
  const [dismissed, setDismissed] = useState<TodoRow[]>([]);
  const [inboxCount, setInboxCount] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Only the latest request may write state (events can overlap a slow load).
  const seq = useRef(0);

  const reload = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const none = Promise.resolve([] as TodoRow[]);
      const [l, i, d, x, n] = await Promise.all([
        countOnly ? none : fetchView('list'),
        countOnly ? none : fetchView('inbox'),
        countOnly ? none : fetchView('done'),
        includeDismissed && !countOnly ? fetchView('dismissed') : none,
        invoke<number>('api_inbox_count'),
      ]);
      if (mine !== seq.current) return;
      const rows = (v: unknown) => (Array.isArray(v) ? (v as TodoRow[]) : []);
      setList(rows(l));
      setInbox(rows(i));
      setDone(rows(d));
      setDismissed(rows(x));
      setInboxCount(typeof n === 'number' ? n : 0);
      setError(null);
    } catch (err) {
      console.error('Failed to load to-do views:', err);
      if (mine === seq.current) setError('Could not load your to-do list.');
    } finally {
      if (mine === seq.current) setLoaded(true);
    }
  }, [includeDismissed, countOnly]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => safeListen('action-items-updated', () => void reload()), [reload]);

  return { list, inbox, done, dismissed, inboxCount, loaded, error, reload };
}
