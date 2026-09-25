'use client';

import { createContext, useCallback, useContext, useState, type ReactNode } from 'react';

const EMPTY: ReadonlySet<string> = new Set();

const IdsContext = createContext<ReadonlySet<string>>(EMPTY);
const PublishContext = createContext<((ids: ReadonlySet<string>) => void) | null>(null);

function sameIds(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  if (a.size !== b.size) return false;
  for (const id of a) if (!b.has(id)) return false;
  return true;
}

/**
 * The meetings with work in flight — the "Processing" status on Today and All Meetings.
 *
 * The ids are COMPUTED by `ProcessingMeetingsBridge`, which sits inside the deliberately
 * scoped `LlmActivityProvider` (AppShell: hoisting that provider over the page would
 * re-render every page on each background-task event). The bridge publishes here only when
 * the set of ids actually changes, so a page re-renders when a meeting starts or stops
 * processing — not on every progress event.
 */
export function ProcessingMeetingsProvider({ children }: { children: ReactNode }) {
  const [ids, setIds] = useState<ReadonlySet<string>>(EMPTY);
  const publish = useCallback((next: ReadonlySet<string>) => {
    setIds((prev) => (sameIds(prev, next) ? prev : next));
  }, []);
  return (
    <PublishContext.Provider value={publish}>
      <IdsContext.Provider value={ids}>{children}</IdsContext.Provider>
    </PublishContext.Provider>
  );
}

/** Ids of the meetings showing "Processing". Empty outside the provider. */
export function useProcessingMeetingIds(): ReadonlySet<string> {
  return useContext(IdsContext);
}

/** The bridge's write end; null outside the provider. */
export function usePublishProcessingMeetingIds(): ((ids: ReadonlySet<string>) => void) | null {
  return useContext(PublishContext);
}
