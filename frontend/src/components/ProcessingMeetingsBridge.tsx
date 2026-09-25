'use client';

import { useEffect } from 'react';
import { useOptionalBacklog } from '@/contexts/DeferredBacklogProvider';
import { useOptionalLlmActivity } from '@/contexts/LlmActivityProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { usePublishProcessingMeetingIds } from '@/contexts/ProcessingMeetingsContext';
import { buildQueueView, processingMeetingIds } from '@/lib/transport/queue-view';
import type { BacklogView } from '@/lib/deferred-backlog';

const EMPTY_BACKLOG: BacklogView = {
  items: [],
  pendingCount: 0,
  processing: false,
  active: null,
  activeOrdinal: 0,
  total: 0,
};

/**
 * Derives which meetings are processing from the queue rail's own sources — the deferred
 * backlog, background LLM activity, and the just-stopped recording's final transcription
 * pass — through the same `buildQueueView` the rail draws, and publishes the ids to
 * `ProcessingMeetingsProvider`. Mounted inside `LlmActivityProvider` (AppShell); renders
 * nothing.
 */
export function ProcessingMeetingsBridge() {
  const backlogView = useOptionalBacklog()?.view;
  const llm = useOptionalLlmActivity();
  const { isProcessing } = useRecordingState();
  const { activeRecordingMeetingId } = useSidebar();
  const publish = usePublishProcessingMeetingIds();

  useEffect(() => {
    if (!publish) return;
    const view = buildQueueView(
      { ...EMPTY_BACKLOG, ...backlogView, items: backlogView?.items ?? [] },
      llm,
      { isProcessing: !!isProcessing, title: null, meetingId: activeRecordingMeetingId ?? null },
    );
    publish(processingMeetingIds(view, !!backlogView?.processing));
  }, [publish, backlogView, llm, isProcessing, activeRecordingMeetingId]);

  return null;
}
