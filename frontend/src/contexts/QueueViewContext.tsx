'use client';

import { createContext, useContext, useEffect, useMemo, type ReactNode } from 'react';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { useOptionalLlmActivity } from '@/contexts/LlmActivityProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { usePublishProcessingMeetingIds } from '@/contexts/ProcessingMeetingsContext';
import { buildQueueView, processingMeetingIds, type QueueView } from '@/lib/transport/queue-view';

const QueueViewContext = createContext<QueueView | null>(null);

const IDLE: QueueView = { rows: [], running: null, nextUp: null, count: 0, lamp: 'off', failures: 0 };

/**
 * THE queue view (specs/0057 decision 8), computed once per change and shared: the sidebar's
 * queue row reads it, and the same view feeds the "Processing" meeting ids that Today and
 * All Meetings show, so a list can never disagree with the rail.
 *
 * Mounted inside the deliberately scoped `LlmActivityProvider` (AppShell). The memo depends
 * on the stable pieces — the backlog's `items` array and each LLM activity field, not the
 * provider's per-render `{...view, dismiss}` object — and the processing ids are published
 * to `ProcessingMeetingsProvider`, which only re-renders the pages when the SET changes.
 */
export function QueueViewProvider({ children }: { children: ReactNode }) {
  const { view: backlog } = useBacklog();
  const llm = useOptionalLlmActivity();
  const { isProcessing } = useRecordingState();
  // The '+ New Call' guard is the context's placeholder-for-unnamed-session, not a real
  // title; `buildQueueView` falls back to a generic "Recording" title on null.
  const { meetingTitle } = useTranscripts();
  const recordingTitle = meetingTitle && meetingTitle !== '+ New Call' ? meetingTitle : null;
  // Kept after stop, so the post-stop transcription row can name its meeting.
  const { activeRecordingMeetingId } = useSidebar();
  const publish = usePublishProcessingMeetingIds();

  const items = backlog.items;
  const draining = backlog.processing;
  const queued = llm?.queued;
  const running = llm?.running;
  const history = llm?.history;
  const hasFailure = llm?.hasFailure ?? false;
  const hasLlm = llm !== null;

  const view = useMemo(
    () =>
      buildQueueView(
        { items },
        hasLlm
          ? { queued: queued ?? [], running: running ?? [], history: history ?? [], hasFailure }
          : null,
        { isProcessing: !!isProcessing, title: recordingTitle, meetingId: activeRecordingMeetingId ?? null },
      ),
    [items, hasLlm, queued, running, history, hasFailure, isProcessing, recordingTitle, activeRecordingMeetingId],
  );

  useEffect(() => {
    publish?.(processingMeetingIds(view, draining));
  }, [publish, view, draining]);

  return <QueueViewContext.Provider value={view}>{children}</QueueViewContext.Provider>;
}

/** The shared queue view; idle outside `QueueViewProvider`. */
export function useQueueView(): QueueView {
  return useContext(QueueViewContext) ?? IDLE;
}
