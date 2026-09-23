'use client';

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
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

/** A Rust command error is a plain string; anything else gets a readable fallback. */
function messageOf(error: unknown): string {
  if (typeof error === 'string' && error.trim() !== '') return error;
  if (error instanceof Error && error.message.trim() !== '') return error.message;
  return 'Something went wrong. Check the logs for details.';
}

/** Why a backlog hand-off was refused, in the user's terms (`HandoffOutcome.reason`). */
function describeHandoff(reason: 'no-folder-path' | 'threw'): string {
  return reason === 'no-folder-path'
    ? "This meeting's recording folder is missing, so there is nothing to process."
    : 'The meeting could not be queued for processing.';
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
  // specs/0074 W4 — a Done prep row (llm success history) also counts as "finished", so the
  // control shows up and clears it even when no backlog item is done/errored.
  const hasLlmDone = view.rows.some((r) => r.source === 'llm' && r.stage === 'done');
  const hasFinished = backlog.items.some((i) => i.status === 'done' || i.status === 'error') || hasLlmDone;
  const hasLlmFailure = view.rows.some((r) => r.source === 'llm' && r.stage === 'error');

  // A backlog row retries by re-enqueuing that meeting (TranscriptPanel.tsx's "Process now"
  // uses the same call shape); an LLM row retries by its numeric registry task id, never by
  // string-slicing the `llm:` prefix off `row.id` (specs/0063 W3 Task 6).
  //
  // specs/0066 W2 — a rejected dispatch SAYS SO. Both of these used to end in
  // `.catch(() => {})`, borrowed from the LLM-activity provider, where swallowing is right:
  // a snapshot poll that fails is noise the user never asked for. A click is the opposite.
  // Every reason a retry can be refused — the task already gone, a diarization pass already
  // running, no model configured, a meeting whose folder has moved — arrived as a clean Rust
  // error message and was then thrown away, which is precisely what "clicking Retry doesn't
  // seem to do anything" looks like from the outside. Success stays silent: the row leaving
  // the failed section and coming back as running is the feedback.
  const handleRetry = async (row: QueueRow) => {
    if (row.source === 'backlog' && row.meetingId) {
      const outcome = await enqueueMeeting(row.meetingId, { force: true });
      if (!outcome.accepted) {
        toast.error('Could not retry', { description: describeHandoff(outcome.reason) });
      }
      return;
    }
    if (row.source === 'llm' && typeof row.taskId === 'number') {
      try {
        await invoke('api_llm_activity_retry_task', { taskId: row.taskId });
      } catch (error) {
        toast.error('Could not retry', { description: messageOf(error) });
      }
    }
  };
  // Per-row dismiss removes just that record (fix-round 1, specs/0063 W3 Task 6) — distinct
  // from the header's `handleDismissAll`, which is the honestly-global
  // `api_llm_activity_dismiss` and stays that way.
  const handleDismiss = async (row: QueueRow) => {
    if (typeof row.taskId === 'number') {
      try {
        await invoke('api_llm_activity_dismiss_task', { taskId: row.taskId });
      } catch (error) {
        toast.error('Could not dismiss', { description: messageOf(error) });
      }
    }
  };
  const handleDismissAll = () => void llm?.dismiss();
  // specs/0074 W4 — "Clear finished" now clears BOTH finished surfaces: the backlog's local
  // done/error items (unchanged, `dismissDone`) and the registry's success/skipped history
  // (new — `api_llm_activity_clear_finished`, which is what makes a Done prep row go away).
  // Best-effort like the header's dismiss: a failed clear just leaves the rows for next time.
  const handleClearFinished = async () => {
    dismissDone();
    try {
      await invoke('api_llm_activity_clear_finished');
    } catch (error) {
      toast.error('Could not clear finished tasks', { description: messageOf(error) });
    }
  };

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
          <Row
            key={row.id}
            row={row}
            onRetry={() => void handleRetry(row)}
            onDismiss={() => void handleDismiss(row)}
          />
        ))}
      </ul>
      {hasFinished && (
        <div className="flex justify-end px-2">
          <button
            type="button"
            onClick={() => void handleClearFinished()}
            className="text-[10px] text-muted-foreground hover:text-foreground"
          >
            Clear finished
          </button>
        </div>
      )}
    </div>
  );
}
