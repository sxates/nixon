'use client';

/**
 * useTopics / useTopic (specs/0089) — load topics through the typed wrappers and refetch
 * on the `topics-updated` event (`{ topic_id: string | null }`; null = several changed).
 * Only the latest request may write state.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { safeListen } from '@/lib/safe-listen';
import { getTopic, listTopics } from '@/lib/topics';
import type { TopicDetail, TopicStatus, TopicSummary, TopicsUpdatedPayload } from '@/types';

export interface UseAsync<T> {
  data: T | null;
  /** True until the first load settles. */
  loading: boolean;
  /** User-facing message when the last load failed; null otherwise. */
  error: string | null;
  refresh: () => Promise<void>;
}

const message = (e: unknown, fallback: string) =>
  typeof e === 'string' && e ? e : e instanceof Error && e.message ? e.message : fallback;

function useLoaded<T>(
  fetcher: () => Promise<T>,
  matches: (topicId: string | null) => boolean,
  fallback: string,
): UseAsync<T> {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const seq = useRef(0);

  const refresh = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const next = await fetcher();
      if (mine !== seq.current) return;
      setData(next);
      setError(null);
    } catch (e) {
      console.error(fallback, e);
      if (mine === seq.current) setError(message(e, fallback));
    } finally {
      if (mine === seq.current) setLoading(false);
    }
  }, [fetcher, fallback]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const matchRef = useRef(matches);
  matchRef.current = matches;
  useEffect(
    () =>
      safeListen<TopicsUpdatedPayload>('topics-updated', (e) => {
        if (matchRef.current(e?.payload?.topic_id ?? null)) void refresh();
      }),
    [refresh],
  );

  return { data, loading, error, refresh };
}

/** Topic index rows; `status` filters to open or closed (omit for all). */
export function useTopics(status?: TopicStatus): UseAsync<TopicSummary[]> {
  const fetcher = useCallback(() => listTopics(status), [status]);
  return useLoaded(fetcher, () => true, 'Could not load topics.');
}

/** One topic's detail; ignores events for other topics. */
export function useTopic(id: string): UseAsync<TopicDetail> {
  const fetcher = useCallback(() => getTopic(id), [id]);
  const matches = useCallback((t: string | null) => t === null || t === id, [id]);
  return useLoaded(fetcher, matches, 'Could not load this topic.');
}
