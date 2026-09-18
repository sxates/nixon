'use client';

import React from 'react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { useOptionalLlmActivity } from '@/contexts/LlmActivityProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useQueueOpen } from '@/contexts/QueueOpenContext';
import { buildQueueView } from '@/lib/transport/queue-view';
import { LampDot } from './LampDot';
import { QueuePanel } from './QueuePanel';

/**
 * Right zone of the rail (specs/0057 decision 8): engraved "Queue", the outstanding count, a
 * single truncated status line and one lamp — the whole of what the retired backlog pill and
 * sidebar LLM row used to say, in the place the user now always looks.
 */
export function QueueIndicator() {
  const { view: backlog } = useBacklog();
  // useOptionalLlmActivity, never the throwing hook: the rail is mounted app-wide and must
  // survive a tree where the LLM provider isn't above it.
  const llm = useOptionalLlmActivity();
  const { isProcessing } = useRecordingState();
  // Same source TransportStatus (the rail's left zone) reads its live title from — the
  // '+ New Call' guard is its placeholder-for-unnamed-session, not a real title. No
  // sidebar-title fallback here (specs/0063 W3 Task 6): `buildQueueView` already falls back
  // to a generic "Recording" title, so a plain null on the rare gap is enough.
  const { meetingTitle } = useTranscripts();
  const recordingTitle = meetingTitle && meetingTitle !== '+ New Call' ? meetingTitle : null;
  const { open, setOpen } = useQueueOpen();
  const view = buildQueueView(backlog, llm, { isProcessing, title: recordingTitle });

  const line = view.running
    ? `${view.running.stageLabel} · ${view.running.title}`
    : view.nextUp
      ? `Next · ${view.nextUp.title}`
      : view.failures > 0
        ? `${view.failures} need retry`
        : 'Idle';

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="flex h-full min-w-0 max-w-[22rem] items-center gap-2.5 px-5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
        >
          <LampDot tone={view.lamp} label="Queue" decorative />
          <span className="flex min-w-0 flex-col leading-tight">
            <span className="u-section-label text-[9px]">Queue {view.count}</span>
            <span className="truncate text-[11px] text-muted-foreground">{line}</span>
          </span>
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="end"
        aria-label="Queue"
        className="w-80 max-w-[90vw] border-border bg-popover p-2"
      >
        <QueuePanel view={view} />
      </PopoverContent>
    </Popover>
  );
}
