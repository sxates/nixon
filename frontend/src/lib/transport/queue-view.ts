import type { BacklogItem, BacklogItemStatus, BacklogView } from '@/lib/deferred-backlog';
import type { LlmActivityView, LlmTaskKind } from '@/contexts/LlmActivityProvider';

export type QueueStage = 'waiting' | 'transcribing' | 'diarizing' | 'summarizing' | 'llm' | 'done' | 'error';

export interface QueueRow {
  id: string;
  title: string;
  stage: QueueStage;
  stageLabel: string;
  source: 'backlog' | 'llm' | 'recording';
  error?: string | null;
  meetingId?: string | null;
  /** Failed rows only: whether a retry dispatch exists for it (specs/0063 W3). */
  retryable?: boolean;
  /** Failed rows only: which control the row offers — a real Retry, or just Dismiss. */
  action?: 'retry' | 'dismiss' | null;
  /**
   * `llm` rows only: the registry's numeric task id, straight from `h.id`/`t.id`. `id` above
   * is a prefixed string (`llm:${id}`) built for React keys and must never be string-sliced
   * back into this — that silently breaks the moment the prefix changes. This is the value
   * `api_llm_activity_retry_task` actually takes (specs/0063 W3 Task 6).
   */
  taskId?: number;
  /** `llm` rows only: the registry task kind (a queued prep brief is not "Processing"). */
  kind?: LlmTaskKind;
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
  error: 'Failed',
};

const ACTIVE: BacklogItemStatus[] = ['transcribing', 'diarizing', 'summarizing'];

/**
 * Kinds `retry_task` in `llm_activity/retry.rs::is_retryable` knows how to re-dispatch.
 * The two lists are not linked by the compiler — if that Rust `matches!` arm changes,
 * this array must change with it, or a queue row will offer a Retry that fails, or hide
 * one that would have worked.
 */
const RETRYABLE_KINDS: readonly LlmTaskKind[] = ['prepBrief', 'meetingSummary', 'actionItems', 'diarization'];

/**
 * specs/0074 W4: "Done" prep rows share the queue's done section with finished backlog
 * items, capped so a long session doesn't pile up every brief the pass ever wrote. `history`
 * is already newest-first, so slicing the front keeps the most recent ones. Failures and
 * other kinds are unaffected — only successful `prepBrief` history renders here at all
 * (decision: "Done" rows are prep-only, matching the owner's ask).
 */
const DONE_PREP_LIMIT = 5;

function backlogRow(item: BacklogItem): QueueRow {
  const stage = item.status as QueueStage;
  const isError = item.status === 'error';
  return {
    id: `backlog:${item.meeting.id}`,
    title: item.meeting.title,
    stage,
    stageLabel: QUEUE_STAGE_LABEL[stage],
    source: 'backlog',
    meetingId: item.meeting.id,
    ...(isError ? { retryable: true, action: 'retry' as const } : {}),
  };
}

/**
 * specs/0057 decision 8 — merge the deferred backlog and background LLM activity into ONE
 * ordered queue: active backlog item, running AI tasks, waiting items, failures, done.
 */
export function buildQueueView(
  backlog: Pick<BacklogView, 'items'>,
  llm: LlmActivityView | null,
  recording: { isProcessing: boolean; title: string | null; meetingId?: string | null } = {
    isProcessing: false,
    title: null,
  },
): QueueView {
  const transcribing: QueueRow[] = recording.isProcessing
    ? [
        {
          id: 'transcription',
          title: recording.title ?? 'Recording',
          stage: 'transcribing',
          stageLabel: QUEUE_STAGE_LABEL.transcribing,
          source: 'recording',
          meetingId: recording.meetingId ?? null,
        },
      ]
    : [];
  const active = backlog.items.filter((i) => ACTIVE.includes(i.status)).map(backlogRow);
  const running = (llm?.running ?? []).map<QueueRow>((t) => ({
    id: `llm:${t.id}`,
    title: t.label,
    stage: 'llm',
    stageLabel: QUEUE_STAGE_LABEL.llm,
    source: 'llm',
    meetingId: t.meetingId,
    taskId: t.id,
    kind: t.kind,
  }));
  const backlogWaiting = backlog.items.filter((i) => i.status === 'waiting').map(backlogRow);
  // specs/0074 W4 — a queued prep brief (background work waiting its turn, distinct from a
  // running one) renders alongside backlog waiting rows, same stage, same "Waiting" label.
  const llmWaiting = (llm?.queued ?? []).map<QueueRow>((t) => ({
    id: `llm:${t.id}`,
    title: t.label,
    stage: 'waiting',
    stageLabel: QUEUE_STAGE_LABEL.waiting,
    source: 'llm',
    meetingId: t.meetingId,
    taskId: t.id,
    kind: t.kind,
  }));
  const waiting = [...backlogWaiting, ...llmWaiting];
  const backlogErrors = backlog.items.filter((i) => i.status === 'error').map(backlogRow);
  // `hasFailure` is the registry's BACKGROUND-failure flag; `history` is unfiltered and also
  // holds foreground Ask-AI failures the user already saw inline. Lighting the rail red for
  // those would report an error twice, so the whole failed-history slice is gated on the flag.
  const llmFailures = (llm?.hasFailure ? llm.history : [])
    .filter((h) => h.outcome.type === 'failed')
    .map<QueueRow>((h) => {
      const retryable = RETRYABLE_KINDS.includes(h.kind) && Boolean(h.meetingId);
      return {
        id: `llm:${h.id}`,
        title: h.label,
        stage: 'error',
        stageLabel: QUEUE_STAGE_LABEL.error,
        source: 'llm',
        error: h.outcome.type === 'failed' ? h.outcome.error : null,
        meetingId: h.meetingId,
        taskId: h.id,
        retryable,
        action: retryable ? 'retry' : 'dismiss',
      };
    });
  const backlogDone = backlog.items.filter((i) => i.status === 'done').map(backlogRow);
  const doneLlmPrep = (llm?.history ?? [])
    .filter((h) => h.kind === 'prepBrief' && h.outcome.type === 'success')
    .slice(0, DONE_PREP_LIMIT)
    .map<QueueRow>((h) => ({
      id: `llm:${h.id}`,
      title: h.label,
      stage: 'done',
      stageLabel: QUEUE_STAGE_LABEL.done,
      source: 'llm',
      meetingId: h.meetingId,
      taskId: h.id,
    }));
  const done = [...backlogDone, ...doneLlmPrep];

  const rows = [...transcribing, ...active, ...running, ...waiting, ...backlogErrors, ...llmFailures, ...done];
  const failures = backlogErrors.length + llmFailures.length;
  const runningRow = transcribing[0] ?? active[0] ?? running[0] ?? null;
  const nextUp = rows.find((r) => r !== runningRow && r.stage !== 'done' && r.stage !== 'error') ?? null;
  const lamp = failures > 0 ? 'red' : runningRow ? 'amber' : 'off';
  return { rows, running: runningRow, nextUp, count: rows.filter((r) => r.stage !== 'done').length, lamp, failures };
}

/** Row stages that mean "work is running for this meeting right now". */
const IN_FLIGHT: readonly QueueStage[] = ['transcribing', 'diarizing', 'summarizing', 'llm'];

/**
 * The meetings that show "Processing" on Today and All Meetings: derived from the SAME rows
 * the queue rail draws (`buildQueueView`), so a list can never disagree with the rail.
 *
 * In flight = a row that is running (transcribing / speakers / summarizing / any AI task,
 * including a prep brief for an upcoming meeting), or a WAITING row of post-recording work
 * that will run without the user doing anything: a queued AI task other than a prep brief
 * (the registry drains its own queue), or a backlog item while a drain is under way.
 *
 * Not "Processing": a queued prep brief (the prep pass plans a week of them, and a whole week
 * of agenda rows lit up for work that hasn't started would be noise — a prep brief counts
 * only while it runs), a backlog item parked until AC power / a click (nothing is happening
 * to it), and a failed row — failures stay with the queue's own error UI.
 */
export function processingMeetingIds(view: QueueView, backlogDraining: boolean): Set<string> {
  const ids = new Set<string>();
  for (const row of view.rows) {
    if (!row.meetingId) continue;
    const running = IN_FLIGHT.includes(row.stage);
    const willRun =
      row.stage === 'waiting' &&
      (row.source === 'llm' ? row.kind !== 'prepBrief' : backlogDraining);
    if (running || willRun) ids.add(row.meetingId);
  }
  return ids;
}
