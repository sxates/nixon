'use client';

import { createContext, useContext, useEffect, useRef, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useDeferredBacklog, type UseDeferredBacklogReturn } from '@/hooks/useDeferredBacklog';
import { safeListen } from '@/lib/safe-listen';
import { WRAP_UP_EVENT, type WrapUpState } from '@/lib/wrap-up';
import type { HandoffOutcome } from '@/lib/deferred-backlog';

const BacklogContext = createContext<UseDeferredBacklogReturn | null>(null);

/**
 * specs/0083 W2b — answer the wrap-up's `handoff` stage. A meeting overridden live after a
 * deferred span ('process-now') gets its full pass from the backlog; Rust asks, waits up to
 * 30 s, and on no answer (or a refusal) degrades to diarizing it and saying so. So this
 * always answers, `accepted: false` included.
 */
function useWrapUpHandoff(enqueueMeeting: UseDeferredBacklogReturn['enqueueMeeting']): void {
  const enqueueRef = useRef(enqueueMeeting);
  enqueueRef.current = enqueueMeeting;

  useEffect(
    () =>
      safeListen<WrapUpState>(WRAP_UP_EVENT, (event) => {
        const state = event.payload;
        if (state?.stage !== 'handoff' || !state.meetingId) return;
        const meetingId = state.meetingId;
        void (async () => {
          let outcome: HandoffOutcome;
          try {
            outcome = await enqueueRef.current(meetingId, { force: true });
          } catch (error) {
            console.warn('[backlog] wrap-up handoff threw:', error);
            outcome = { accepted: false, reason: 'threw' };
          }
          try {
            await invoke('api_wrap_up_handoff_result', { meetingId, accepted: outcome.accepted });
          } catch (error) {
            // Rust times out to its degrade path; nothing more to do here.
            console.warn('[backlog] could not answer the wrap-up handoff:', error);
          }
        })();
      }),
    [],
  );
}

/**
 * Instantiates the deferred-backlog controller ONCE and shares it (spec 0045). The controller
 * owns power watching, auto-start, and the sequential drain; every process affordance (Today
 * button, global indicator, per-meeting "Process now") reads this one instance so their state
 * is always consistent — no local set-once spinners.
 */
export function DeferredBacklogProvider({ children }: { children: ReactNode }) {
  const value = useDeferredBacklog();
  useWrapUpHandoff(value.enqueueMeeting);
  return <BacklogContext.Provider value={value}>{children}</BacklogContext.Provider>;
}

export function useBacklog(): UseDeferredBacklogReturn {
  const ctx = useContext(BacklogContext);
  if (!ctx) throw new Error('useBacklog must be used within a DeferredBacklogProvider');
  return ctx;
}
