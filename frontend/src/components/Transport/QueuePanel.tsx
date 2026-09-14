'use client';

import React from 'react';
import { cn } from '@/lib/utils';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { useOptionalLlmActivity } from '@/contexts/LlmActivityProvider';
import type { QueueRow, QueueStage, QueueView } from '@/lib/transport/queue-view';
import { LampDot, type LampTone } from './LampDot';

const ROW_LAMP: Record<QueueStage, LampTone> = {
  waiting: 'off',
  transcribing: 'amber',
  diarizing: 'amber',
  summarizing: 'amber',
  llm: 'amber',
  done: 'green',
  error: 'red',
};

const ACTIVE_STAGES: QueueStage[] = ['transcribing', 'diarizing', 'summarizing', 'llm'];

function Row({ row, onRetry }: { row: QueueRow; onRetry: () => void }) {
  const active = ACTIVE_STAGES.includes(row.stage);
  return (
    <li className="flex flex-col gap-1 px-2 py-1.5">
      <div className="flex items-center gap-2.5">
        <LampDot tone={ROW_LAMP[row.stage]} label={row.stageLabel} decorative />
        <span className="min-w-0 flex-1 truncate text-xs text-foreground">{row.title}</span>
        <span className="u-section-label shrink-0 text-[9px]">{row.stageLabel}</span>
      </div>
      {row.error && <p className="break-words pl-[18px] text-[10px] text-destructive">{row.error}</p>}
      {/* A static 60% fill, not a shimmer: the rail's liveness comes from the lamp and the
          counter. Indeterminate bars that animate forever read as progress they can't report. */}
      {active && (
        <span aria-hidden className="ml-[18px] block h-[3px] rounded-[1px] bg-border">
          <span className="block h-full w-[60%] rounded-[1px] bg-brand" />
        </span>
      )}
      {/* Retry only. There is no per-row dismiss: `api_llm_activity_dismiss` takes no id and
          clears the WHOLE failure history, so a per-row control would quietly throw away the
          other failures. Dismissing is a header action, named for what it actually does. */}
      {row.source === 'llm' && row.stage === 'error' && row.retryable && (
        <div className="flex justify-end pl-[18px]">
          <button type="button" onClick={onRetry} className="text-[10px] text-muted-foreground hover:text-foreground">
            Retry
          </button>
        </div>
      )}
    </li>
  );
}

/**
 * specs/0057 decision 8 — the ONE queue panel: deferred processing and background AI in a
 * single ordered list, with the same actions their two retired surfaces had
 * (the retired BacklogDetailPopover's Stop / Process all / Clear finished, and the
 * retired LLM row's Retry / Dismiss).
 */
export function QueuePanel({ view, className }: { view: QueueView; className?: string }) {
  const { view: backlog, stop, startNow, dismissDone } = useBacklog();
  const llm = useOptionalLlmActivity();
  const hasFinished = backlog.items.some((i) => i.status === 'done' || i.status === 'error');
  const hasLlmFailure = view.rows.some((r) => r.source === 'llm' && r.stage === 'error');

  return (
    <div className={cn('flex flex-col gap-1', className)}>
      <div className="flex items-center justify-between gap-2 px-2">
        <span className="u-section-label text-[9px]">Item · Stage</span>
        {hasLlmFailure && view.failures > 0 && (
          <button
            type="button"
            onClick={() => void llm?.dismiss()}
            className="ml-auto text-[10px] text-muted-foreground hover:text-foreground"
          >
            Dismiss failures
          </button>
        )}
        {backlog.processing ? (
          <button type="button" onClick={stop} className="text-[10px] text-muted-foreground hover:text-foreground">
            Stop
          </button>
        ) : backlog.pendingCount > 0 ? (
          <button type="button" onClick={startNow} className="text-[10px] font-semibold text-brand hover:underline">
            Process all
          </button>
        ) : null}
      </div>
      <ul className="max-h-72 overflow-y-auto">
        {view.rows.length === 0 && (
          <li className="px-2 py-3 text-center text-xs text-muted-foreground">Nothing in the queue.</li>
        )}
        {view.rows.map((row) => (
          <Row
            key={row.id}
            row={row}
            onRetry={() => void (row.meetingId && llm?.retry(row.meetingId))}
          />
        ))}
      </ul>
      {hasFinished && (
        <div className="flex justify-end px-2">
          <button type="button" onClick={dismissDone} className="text-[10px] text-muted-foreground hover:text-foreground">
            Clear finished
          </button>
        </div>
      )}
    </div>
  );
}
