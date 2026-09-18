'use client';

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
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

function Row({ row, onRetry, onDismiss }: { row: QueueRow; onRetry: () => void; onDismiss: () => void }) {
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
      {/* `row.action` drives the button, not a source/stage/retryable spot-check (that gate
          used to only cover 'llm' rows, so a failed backlog row's Retry silently did nothing —
          specs/0063 W3 Task 6). A 'retry' row dispatches for real, by numeric task id. A
          'dismiss' row (e.g. a failed askAI task) now has a REAL per-row dismiss too —
          `api_llm_activity_dismiss_task` removes just that one registry record (fix-round 1:
          the first version of this wired every row's Dismiss to the header's full-history
          `api_llm_activity_dismiss`, so clicking one row's button silently cleared every
          other failure too — a button reading its own row while acting on all of them, the
          same class of bug as the original "Retry does nothing" report). The header's own
          "Dismiss failures" is unchanged and still clears the lot on purpose. */}
      {row.action === 'retry' && (
        <div className="flex justify-end pl-[18px]">
          <button type="button" onClick={onRetry} className="text-[10px] text-muted-foreground hover:text-foreground">
            Retry
          </button>
        </div>
      )}
      {row.action === 'dismiss' && (
        <div className="flex justify-end pl-[18px]">
          <button type="button" onClick={onDismiss} className="text-[10px] text-muted-foreground hover:text-foreground">
            Dismiss
          </button>
        </div>
      )}
    </li>
  );
}

/**
 * specs/0057 decision 8 — the ONE queue panel: deferred processing and background AI in a
 * single ordered list, with the same actions their two retired surfaces had (Today's own
 * queue popover's Stop / Process all / Clear finished, and the sidebar LLM row's Retry /
 * Dismiss — specs/0063 W3 Task 6 finished retiring the former).
 */
export function QueuePanel({ view, className }: { view: QueueView; className?: string }) {
  const { view: backlog, stop, startNow, dismissDone, enqueueMeeting } = useBacklog();
  const llm = useOptionalLlmActivity();
  const hasFinished = backlog.items.some((i) => i.status === 'done' || i.status === 'error');
  const hasLlmFailure = view.rows.some((r) => r.source === 'llm' && r.stage === 'error');

  // A backlog row retries by re-enqueuing that meeting (TranscriptPanel.tsx's "Process now"
  // uses the same call shape); an LLM row retries by its numeric registry task id, never by
  // string-slicing the `llm:` prefix off `row.id` (specs/0063 W3 Task 6).
  const handleRetry = (row: QueueRow) => {
    if (row.source === 'backlog' && row.meetingId) {
      void enqueueMeeting(row.meetingId, { force: true });
    } else if (row.source === 'llm' && typeof row.taskId === 'number') {
      void invoke('api_llm_activity_retry_task', { taskId: row.taskId }).catch(() => {
        /* best-effort, like every other LLM-activity call (LlmActivityProvider.tsx) */
      });
    }
  };
  // Per-row dismiss removes just that record (fix-round 1, specs/0063 W3 Task 6) — distinct
  // from the header's `handleDismissAll`, which is the honestly-global
  // `api_llm_activity_dismiss` and stays that way.
  const handleDismiss = (row: QueueRow) => {
    if (typeof row.taskId === 'number') {
      void invoke('api_llm_activity_dismiss_task', { taskId: row.taskId }).catch(() => {
        /* best-effort, like every other LLM-activity call (LlmActivityProvider.tsx) */
      });
    }
  };
  const handleDismissAll = () => void llm?.dismiss();

  return (
    <div className={cn('flex flex-col gap-1', className)}>
      <div className="flex items-center justify-between gap-2 px-2">
        <span className="u-section-label text-[9px]">Item · Stage</span>
        {hasLlmFailure && view.failures > 0 && (
          <button
            type="button"
            onClick={handleDismissAll}
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
          <Row key={row.id} row={row} onRetry={() => handleRetry(row)} onDismiss={() => handleDismiss(row)} />
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
