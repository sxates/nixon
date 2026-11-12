'use client';

/**
 * useTodoViews (specs/0088) — loads the To-do / Inbox / Done lists (and optionally the
 * dismissed list) through `api_list_todo_view`, plus the inbox badge count, and refetches
 * on the `action-items-updated` event.
 *
 * That event fires from extraction, triage, notes, dismiss-older and
 * `api_update_action_item`, but NOT from the legacy status / bulk status / delete /
 * reorder commands — callers of those must call `reload()` themselves.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';
import type { TodoRow, TodoView } from '@/types';

export interface UseTodoViewsOptions {
  /** Also load the dismissed list (the "Show dismissed" toggle). Default false. */
  includeDismissed?: boolean;
}

export interface TodoViews {
  list: TodoRow[];
  inbox: TodoRow[];
  done: TodoRow[];
  dismissed: TodoRow[];
  inboxCount: number;
  /** False until the first load settles. */
  loaded: boolean;
  reload: () => Promise<void>;
}

const fetchView = (view: TodoView) => invoke<TodoRow[]>('api_list_todo_view', { view });

export function useTodoViews({ includeDismissed = false }: UseTodoViewsOptions = {}): TodoViews {
  const [list, setList] = useState<TodoRow[]>([]);
  const [inbox, setInbox] = useState<TodoRow[]>([]);
  const [done, setDone] = useState<TodoRow[]>([]);
  const [dismissed, setDismissed] = useState<TodoRow[]>([]);
  const [inboxCount, setInboxCount] = useState(0);
  const [loaded, setLoaded] = useState(false);
  // Only the latest request may write state (events can overlap a slow load).
  const seq = useRef(0);

  const reload = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const [l, i, d, x, n] = await Promise.all([
        fetchView('list'),
        fetchView('inbox'),
        fetchView('done'),
        includeDismissed ? fetchView('dismissed') : Promise.resolve([] as TodoRow[]),
        invoke<number>('api_inbox_count'),
      ]);
      if (mine !== seq.current) return;
      setList(l);
      setInbox(i);
      setDone(d);
      setDismissed(x);
      setInboxCount(n);
    } catch (err) {
      console.error('Failed to load to-do views:', err);
    } finally {
      if (mine === seq.current) setLoaded(true);
    }
  }, [includeDismissed]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => safeListen('action-items-updated', () => void reload()), [reload]);

  return { list, inbox, done, dismissed, inboxCount, loaded, reload };
}
