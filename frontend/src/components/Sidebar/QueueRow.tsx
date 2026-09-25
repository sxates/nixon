'use client';

import React from 'react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useQueueView } from '@/contexts/QueueViewContext';
import { useQueueOpen } from '@/contexts/QueueOpenContext';
import { cn } from '@/lib/utils';
import { LampDot } from '@/components/Transport/LampDot';
import { QueuePanel } from '@/components/Transport/QueuePanel';
import { IconSlot } from './SidebarRow';
import { SIDEBAR_ROW } from './row';

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
  // Computed once in QueueViewProvider (AppShell) and shared with the "Processing" status.
  const view = useQueueView();
  const { open, setOpen } = useQueueOpen();

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
            data-sidebar-row
            className={cn(SIDEBAR_ROW, 'transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring')}
          >
            <IconSlot name="queue">
              <LampDot tone={view.lamp} label="Queue" decorative />
            </IconSlot>
          </button>
        ) : (
          <button
            type="button"
            data-sidebar-row
            className={cn(SIDEBAR_ROW, 'relative text-left transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring')}
          >
            {/* The lamp is narrower than a glyph, so it rides in the shared icon column —
                otherwise "Queue" would start left of every other label. */}
            <IconSlot name="queue">
              <LampDot tone={view.lamp} label="Queue" decorative />
            </IconSlot>
            <span className="u-section-label text-engrave">Queue {view.count}</span>
            {/* One line, always — a wrapping status line would shift the settings row below
                it every time a stage name changed. `SIDEBAR_LABEL` is for the label; this
                trailing status line keeps its own right inset, and a left gap so a long line
                truncates short of the count instead of reading "Queue 1AI · …". */}
            <span className="ml-3 min-w-0 flex-1 truncate pr-3.5 text-right text-[11px] text-muted-foreground">
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
