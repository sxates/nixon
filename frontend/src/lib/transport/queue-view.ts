import type { BacklogItem, BacklogItemStatus, BacklogView } from '@/lib/deferred-backlog';
import type { LlmActivityView } from '@/contexts/LlmActivityProvider';

export type QueueStage = 'waiting' | 'transcribing' | 'diarizing' | 'summarizing' | 'llm' | 'done' | 'error';

export interface QueueRow {
  id: string;
  title: string;
  stage: QueueStage;
  stageLabel: string;
  source: 'backlog' | 'llm';
  error?: string | null;
  meetingId?: string | null;
  /** LLM failure rows only: whether a Retry command exists for it (prep briefs, spec 0052). */
  retryable?: boolean;
}

export interface QueueView {
  rows: QueueRow[];
  running: QueueRow | null;
  nextUp: QueueRow | null;
  count: number;
  lamp: 'off' | 'amber' | 'red';
  failures: number;
}

/** Engraved caps for the rail/panel (specs/0057 §3.6). */
export const QUEUE_STAGE_LABEL: Record<QueueStage, string> = {
  waiting: 'Waiting',
  transcribing: 'Transcribing',
  diarizing: 'Speakers',
  summarizing: 'Summarizing',
  llm: 'AI',
  done: 'Done',
  error: 'Retry',
};

const ACTIVE: BacklogItemStatus[] = ['transcribing', 'diarizing', 'summarizing'];

function backlogRow(item: BacklogItem): QueueRow {
  const stage = item.status as QueueStage;
  return {
    id: `backlog:${item.meeting.id}`,
    title: item.meeting.title,
    stage,
    stageLabel: QUEUE_STAGE_LABEL[stage],
    source: 'backlog',
    meetingId: item.meeting.id,
  };
}

/**
 * specs/0057 decision 8 — merge the deferred backlog and background LLM activity into ONE
 * ordered queue: active backlog item, running AI tasks, waiting items, failures, done.
 */
export function buildQueueView(backlog: BacklogView, llm: LlmActivityView | null): QueueView {
  const active = backlog.items.filter((i) => ACTIVE.includes(i.status)).map(backlogRow);
  const running = (llm?.running ?? []).map<QueueRow>((t) => ({
    id: `llm:${t.id}`,
    title: t.label,
    stage: 'llm',
    stageLabel: QUEUE_STAGE_LABEL.llm,
    source: 'llm',
    meetingId: t.meetingId,
  }));
  const waiting = backlog.items.filter((i) => i.status === 'waiting').map(backlogRow);
  const backlogErrors = backlog.items.filter((i) => i.status === 'error').map(backlogRow);
  // `hasFailure` is the registry's BACKGROUND-failure flag; `history` is unfiltered and also
  // holds foreground Ask-AI failures the user already saw inline. Lighting the rail red for
  // those would report an error twice, so the whole failed-history slice is gated on the flag.
  const llmFailures = (llm?.hasFailure ? llm.history : [])
    .filter((h) => h.outcome.type === 'failed')
    .map<QueueRow>((h) => ({
      id: `llm:${h.id}`,
      title: h.label,
      stage: 'error',
      stageLabel: QUEUE_STAGE_LABEL.error,
      source: 'llm',
      error: h.outcome.type === 'failed' ? h.outcome.error : null,
      meetingId: h.meetingId,
      retryable: h.kind === 'prepBrief' && Boolean(h.meetingId),
    }));
  const done = backlog.items.filter((i) => i.status === 'done').map(backlogRow);

  const rows = [...active, ...running, ...waiting, ...backlogErrors, ...llmFailures, ...done];
  const failures = backlogErrors.length + llmFailures.length;
  const runningRow = active[0] ?? running[0] ?? null;
  const nextUp = rows.find((r) => r !== runningRow && r.stage !== 'done' && r.stage !== 'error') ?? null;
  const lamp = failures > 0 ? 'red' : runningRow ? 'amber' : 'off';
  return { rows, running: runningRow, nextUp, count: rows.filter((r) => r.stage !== 'done').length, lamp, failures };
}
