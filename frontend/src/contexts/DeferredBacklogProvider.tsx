'use client';

import { createContext, useContext, type ReactNode } from 'react';
import { useDeferredBacklog, type UseDeferredBacklogReturn } from '@/hooks/useDeferredBacklog';

const BacklogContext = createContext<UseDeferredBacklogReturn | null>(null);

/**
 * Instantiates the deferred-backlog controller ONCE and shares it (spec 0045). The controller
 * owns power watching, auto-start, and the sequential drain; every process affordance (Today
 * button, global indicator, per-meeting "Process now") reads this one instance so their state
 * is always consistent — no local set-once spinners.
 */
export function DeferredBacklogProvider({ children }: { children: ReactNode }) {
  const value = useDeferredBacklog();
  return <BacklogContext.Provider value={value}>{children}</BacklogContext.Provider>;
}

export function useBacklog(): UseDeferredBacklogReturn {
  const ctx = useContext(BacklogContext);
  if (!ctx) throw new Error('useBacklog must be used within a DeferredBacklogProvider');
  return ctx;
}
