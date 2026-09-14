'use client';

import { Loader2, Check, AlertTriangle, Clock } from 'lucide-react';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import type { BacklogItemStatus } from '@/lib/deferred-backlog';

const STATUS_LABEL: Record<BacklogItemStatus, string> = {
  waiting: 'Waiting',
  transcribing: 'Transcribing…',
  diarizing: 'Identifying speakers…',
  summarizing: 'Summarizing…',
  done: 'Done',
  error: 'Needs retry',
};

function StatusIcon({ status }: { status: BacklogItemStatus }) {
  if (status === 'done') return <Check size={14} className="text-success" aria-hidden />;
  if (status === 'error') return <AlertTriangle size={14} className="text-destructive" aria-hidden />;
  if (status === 'waiting') return <Clock size={14} className="text-muted-foreground" aria-hidden />;
  return <Loader2 size={14} className="animate-spin text-brand" aria-hidden />;
}

/** Lists each backlog meeting by title with its live per-meeting status (spec 0045 WS3). */
export function BacklogDetailPopover({ onClose }: { onClose?: () => void }) {
  const { view, stop, startNow, dismissDone } = useBacklog();
  const hasDone = view.items.some((i) => i.status === 'done' || i.status === 'error');
  return (
    <div className="w-80 max-w-[90vw] rounded-[3px] border border-border bg-popover p-2 shadow-xl">
      <div className="flex items-center justify-between px-2 py-1.5">
        <span className="text-sm font-semibold text-foreground">Processing meetings</span>
        {view.processing ? (
          <button type="button" onClick={stop} className="text-xs text-muted-foreground hover:text-foreground">
            Stop
          </button>
        ) : view.pendingCount > 0 ? (
          <button type="button" onClick={startNow} className="text-xs font-semibold text-brand hover:underline">
            Process all
          </button>
        ) : null}
      </div>
      <ul className="max-h-72 overflow-y-auto">
        {view.items.length === 0 && (
          <li className="px-2 py-3 text-center text-xs text-muted-foreground">Nothing to process.</li>
        )}
        {view.items.map((item) => (
          <li key={item.meeting.id} className="flex items-center gap-2.5 px-2 py-1.5">
            <StatusIcon status={item.status} />
            <span className="min-w-0 flex-1 truncate text-sm text-foreground">{item.meeting.title}</span>
            <span className="shrink-0 text-xs text-muted-foreground">{STATUS_LABEL[item.status]}</span>
          </li>
        ))}
      </ul>
      {hasDone && (
        <div className="flex justify-end px-2 pt-1">
          <button type="button" onClick={() => { dismissDone(); onClose?.(); }} className="text-xs text-muted-foreground hover:text-foreground">
            Clear finished
          </button>
        </div>
      )}
    </div>
  );
}
