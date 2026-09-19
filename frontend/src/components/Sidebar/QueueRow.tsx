'use client';

import React from 'react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { useOptionalLlmActivity } from '@/contexts/LlmActivityProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useQueueOpen } from '@/contexts/QueueOpenContext';
import { buildQueueView } from '@/lib/transport/queue-view';
import { LampDot } from '@/components/Transport/LampDot';
import { QueuePanel } from '@/components/Transport/QueuePanel';

/**
 * The background-work queue, in the sidebar below Settings (specs/0064 W5).
 *
 * It used to be the transport rail's right zone. The rail is the *recording* surface — reel,
 * counter, level, keys — and the queue is app state that outlives any one recording, so it
 * now sits with the other app-state rows (updates, settings) where the user already looks.
 *
 * Expanded: lamp + "Queue N" + one truncated line of what is happening. Collapsed: the lamp
 * alone, centred like every other icon on the rail. Either way it opens the same panel, and
 * the open state is the shared `QueueOpenContext`, so Today's "Processing…" button still
 * opens this one (specs/0063 W3).
 */
export function QueueRow({ collapsed = false }: { collapsed?: boolean }) {
  const { view: backlog } = useBacklog();
  // useOptionalLlmActivity, never the throwing hook: the sidebar is mounted app-wide and must
  // survive a tree where the LLM provider isn't above it.
  const llm = useOptionalLlmActivity();
  const { isProcessing } = useRecordingState();
  // The '+ New Call' guard is the context's placeholder-for-unnamed-session, not a real
  // title. No sidebar-title fallback (specs/0063 W3 Task 6): `buildQueueView` already falls
  // back to a generic "Recording" title, so a plain null on the rare gap is enough.
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
        {collapsed ? (
          <button
            type="button"
            aria-label={`Queue ${view.count} — ${line}`}
            title={`Queue ${view.count} — ${line}`}
            className="mb-1 flex h-10 w-full items-center justify-center transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <LampDot tone={view.lamp} label="Queue" decorative />
          </button>
        ) : (
          <button
            type="button"
            className="relative flex h-8 w-full items-center gap-2.5 pl-3 pr-3.5 text-left transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <LampDot tone={view.lamp} label="Queue" decorative />
            <span className="u-section-label text-engrave">Queue {view.count}</span>
            {/* One line, always — a wrapping status line would shift the settings row below
                it every time a stage name changed. */}
            <span className="min-w-0 flex-1 truncate text-right text-[11px] text-muted-foreground">
              {line}
            </span>
          </button>
        )}
      </PopoverTrigger>
      <PopoverContent
        side="right"
        align="end"
        aria-label="Queue"
        className="w-80 max-w-[90vw] border-border bg-popover p-2"
      >
        <QueuePanel view={view} />
      </PopoverContent>
    </Popover>
  );
}
