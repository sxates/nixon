import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';
import { WRAP_UP_EVENT, type WrapUpState } from '@/lib/wrap-up';
import type { SummaryPollCallback } from '@/lib/summary-polling';

/** `api_get_summary` statuses (lowercased by the backend) of a run that is still going. */
const RUNNING = ['pending', 'processing'];

/**
 * specs/0083 W2b — the meeting page follows a summary it did not start.
 *
 * The Rust wrap-up starts a recording's summary (in parallel with diarization) whether or not
 * the window is awake, so the page never starts one for a recording. Instead it joins the
 * shared summary poll (specs/0071, `lib/summary-polling.ts`):
 *   - on mount, when the meeting's summary run is pending or processing (opened mid-run,
 *     from the "Meeting saved" toast's View, or after a reload); and
 *   - on `meeting-wrap-up` `summarizing` for the meeting it shows.
 * Joining a poll that is already running just adds a subscriber, so a double join is free.
 */
export function useJoinRunningSummary(
  meetingId: string,
  startSummaryPolling: (meetingId: string, processId: string, onUpdate: SummaryPollCallback) => void,
  handlers: { onJoin: () => void; onResult: SummaryPollCallback },
): void {
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;
  const pollRef = useRef(startSummaryPolling);
  pollRef.current = startSummaryPolling;

  useEffect(() => {
    if (!meetingId || meetingId === 'intro-call') return;
    let cancelled = false;
    const join = () => {
      if (cancelled) return;
      handlersRef.current.onJoin();
      pollRef.current(meetingId, meetingId, (result) => {
        if (!cancelled) handlersRef.current.onResult(result);
      });
    };

    invoke<{ status?: string } | null>('api_get_summary', { meetingId })
      .then((summary) => {
        if (summary?.status && RUNNING.includes(summary.status.toLowerCase())) join();
      })
      .catch((error) => console.warn('[summary] could not check for a running summary:', error));

    const dispose = safeListen<WrapUpState>(WRAP_UP_EVENT, (event) => {
      if (event.payload?.stage === 'summarizing' && event.payload.meetingId === meetingId) join();
    });
    return () => {
      cancelled = true;
      dispose();
    };
  }, [meetingId]);
}

/**
 * What a joined (Rust-started) summary does to the page when it lands: show it, or say it
 * failed. Mirrors `useSummaryGeneration`'s own completion handling, minus the parts that only
 * apply to a run the page started (regeneration restore, the model-settings prompt).
 */
export function joinedSummaryResult(deps: {
  showSummary: (summary: unknown) => void;
  setStatus: (status: 'idle' | 'completed' | 'error') => void;
  setError: (error: string | null) => void;
  applyTitle: (title: string) => void;
}): SummaryPollCallback {
  return (result) => {
    if (result?.status === 'completed' && result.data) {
      const data = result.data;
      deps.showSummary(
        data.markdown ? { markdown: data.markdown, summary_status: data.summary_status } : data,
      );
      deps.setStatus('completed');
      const title = data.MeetingName || result.meetingName;
      if (title) deps.applyTitle(title);
      return;
    }
    if (result?.status === 'error' || result?.status === 'failed') {
      deps.setError(result.error || 'Summary generation failed');
      deps.setStatus('error');
      return;
    }
    if (result?.status === 'cancelled' || result?.status === 'idle') deps.setStatus('idle');
  };
}
