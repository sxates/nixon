import { describe, it, expect } from 'vitest';
import { buildQueueView } from '@/lib/transport/queue-view';
import type { BacklogView } from '@/lib/deferred-backlog';
import type { LlmActivityView } from '@/contexts/LlmActivityProvider';

const meeting = (id: string, title: string) => ({ id, title, folderPath: '/x', transcriptCount: 3 });
const backlog = (items: BacklogView['items'], processing = false): BacklogView => ({
  items, processing,
  pendingCount: items.filter((i) => i.status === 'waiting').length,
  active: items.find((i) => ['transcribing', 'diarizing', 'summarizing'].includes(i.status)) ?? null,
  activeOrdinal: 1, total: items.length,
});
const llm = (
  running: LlmActivityView['running'],
  history: LlmActivityView['history'] = [],
  queued: LlmActivityView['queued'] = [],
): LlmActivityView => ({
  queued, running, history, hasFailure: history.some((h) => h.outcome.type === 'failed'),
});

// specs/0057 decision 8 — ONE queue: deferred processing + background AI, one order, one lamp.
describe('buildQueueView', () => {
  it('is empty and unlit with nothing to do', () => {
    const v = buildQueueView(backlog([]), llm([]));
    expect(v.rows).toEqual([]);
    expect(v.count).toBe(0);
    expect(v.lamp).toBe('off');
    expect(v.running).toBeNull();
  });
  it('orders active backlog, running AI, waiting, failures, done and lights amber while running', () => {
    const v = buildQueueView(
      backlog([
        { meeting: meeting('a', 'Hiring loop debrief'), status: 'transcribing' },
        { meeting: meeting('b', 'Design review'), status: 'waiting' },
        { meeting: meeting('c', 'Old one'), status: 'done' },
      ], true),
      llm([{ id: 1, kind: 'meetingSummary', label: 'Summarizing Q3 planning', note: null, meetingId: 'q3' }]),
    );
    expect(v.rows.map((r) => r.title)).toEqual(['Hiring loop debrief', 'Summarizing Q3 planning', 'Design review', 'Old one']);
    expect(v.rows.map((r) => r.stage)).toEqual(['transcribing', 'llm', 'waiting', 'done']);
    expect(v.running?.id).toBe('backlog:a');
    expect(v.nextUp?.title).toBe('Summarizing Q3 planning');
    expect(v.count).toBe(3);
    expect(v.lamp).toBe('amber');
  });
  it('lights red and counts failures when an AI task failed or a backlog item errored', () => {
    const v = buildQueueView(
      backlog([{ meeting: meeting('a', 'X'), status: 'error' }]),
      llm([], [{ id: 9, kind: 'actionItems', label: 'Extracting tasks — X', error: 'Ollama unreachable', meetingId: 'a', outcome: { type: 'failed', error: 'Ollama unreachable' } }]),
    );
    expect(v.lamp).toBe('red');
    expect(v.failures).toBe(2);
    expect(v.rows.map((r) => r.stage)).toEqual(['error', 'error']);
    expect(v.rows[1].error).toBe('Ollama unreachable');
  });
  it('ignores skipped prep history and successful non-prep history', () => {
    const v = buildQueueView(backlog([]), llm([], [
      { id: 2, kind: 'prepBrief', label: 'skip', error: null, meetingId: null, outcome: { type: 'skipped', reason: 'no transcript' } },
      { id: 3, kind: 'meetingSummary', label: 'Summarized', error: null, meetingId: 'm1', outcome: { type: 'success' } },
    ]));
    expect(v.rows).toEqual([]);
    expect(v.lamp).toBe('off');
  });

  // specs/0074 W4 — the queue renders prep work: a queued brief shows as Waiting, a
  // finished one shows as Done, alongside backlog rows in the same stage.
  it('shows a queued prep brief as a Waiting row alongside backlog waiting rows', () => {
    const v = buildQueueView(
      backlog([{ meeting: meeting('b', 'Design review'), status: 'waiting' }]),
      llm([], [], [{ id: 5, kind: 'prepBrief', label: 'Preparing brief — Weekly sync', meetingId: 'w1' }]),
    );
    expect(v.rows.map((r) => r.stage)).toEqual(['waiting', 'waiting']);
    expect(v.rows.map((r) => r.title)).toEqual(['Design review', 'Preparing brief — Weekly sync']);
    expect(v.rows[1].taskId).toBe(5);
    expect(v.rows[1].meetingId).toBe('w1');
    expect(v.nextUp?.title).toBe('Design review');
  });

  it('shows a successful prep brief as a Done row alongside backlog done rows', () => {
    const v = buildQueueView(
      backlog([{ meeting: meeting('c', 'Old one'), status: 'done' }]),
      llm([], [{ id: 6, kind: 'prepBrief', label: 'Preparing brief — Weekly sync', error: null, meetingId: 'w1', outcome: { type: 'success' } }]),
    );
    expect(v.rows.map((r) => r.stage)).toEqual(['done', 'done']);
    expect(v.rows.map((r) => r.title)).toEqual(['Old one', 'Preparing brief — Weekly sync']);
    expect(v.rows[1].taskId).toBe(6);
  });

  it('does not count a Done prep row toward count or the lamp', () => {
    const v = buildQueueView(
      backlog([]),
      llm([], [{ id: 6, kind: 'prepBrief', label: 'Done brief', error: null, meetingId: 'w1', outcome: { type: 'success' } }]),
    );
    expect(v.count).toBe(0);
    expect(v.lamp).toBe('off');
    expect(v.rows).toHaveLength(1);
    expect(v.rows[0].stage).toBe('done');
  });

  it('caps Done prep rows at 5, keeping the newest (history is already newest-first)', () => {
    const history: LlmActivityView['history'] = Array.from({ length: 7 }, (_, i) => ({
      id: i,
      kind: 'prepBrief' as const,
      label: `Brief ${i}`,
      error: null,
      meetingId: `m${i}`,
      outcome: { type: 'success' as const },
    }));
    const v = buildQueueView(backlog([]), llm([], history));
    expect(v.rows).toHaveLength(5);
    expect(v.rows.map((r) => r.title)).toEqual(['Brief 0', 'Brief 1', 'Brief 2', 'Brief 3', 'Brief 4']);
  });
  it('ignores failed history the registry did not flag (a foreground failure already seen)', () => {
    const v = buildQueueView(backlog([]), {
      queued: [], running: [], hasFailure: false,
      history: [{ id: 9, kind: 'askAI', label: 'Ask AI', error: 'boom', meetingId: 'a', outcome: { type: 'failed', error: 'boom' } }],
    });
    expect(v.rows).toEqual([]);
    expect(v.failures).toBe(0);
    expect(v.lamp).toBe('off');
  });
  it('tolerates a null LLM view (provider not mounted)', () => {
    const v = buildQueueView(backlog([{ meeting: meeting('a', 'X'), status: 'waiting' }]), null);
    expect(v.count).toBe(1);
  });

  // specs/0063 W3 Task 5 — a failed row is a status, not a mislabeled button.
  it('labels a failed row "Failed", not "Retry"', () => {
    const v = buildQueueView(
      backlog([{ meeting: meeting('a', 'X'), status: 'error' }]),
      llm([]),
    );
    expect(v.rows[0].stageLabel).toBe('Failed');
    expect(v.rows[0].stageLabel).not.toBe('Retry');
  });

  it('makes a failed backlog row retryable with a retry action', () => {
    const v = buildQueueView(
      backlog([{ meeting: meeting('a', 'X'), status: 'error' }]),
      llm([]),
    );
    expect(v.rows[0].retryable).toBe(true);
    expect(v.rows[0].action).toBe('retry');
  });

  it.each(['prepBrief', 'meetingSummary', 'actionItems', 'diarization'] as const)(
    'marks a failed %s task retryable when it has a meeting',
    (kind) => {
      const v = buildQueueView(
        backlog([]),
        llm([], [{ id: 1, kind, label: 'X', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
      );
      expect(v.rows[0].retryable).toBe(true);
      expect(v.rows[0].action).toBe('retry');
    },
  );

  it('dismisses a failed askAI task instead of offering retry', () => {
    const v = buildQueueView(
      backlog([]),
      llm([], [{ id: 1, kind: 'askAI', label: 'Ask AI', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );
    expect(v.rows[0].retryable).toBeFalsy();
    expect(v.rows[0].action).toBe('dismiss');
  });

  it('will not retry a retryable-kind failure that has no meeting to retry against', () => {
    const v = buildQueueView(
      backlog([]),
      llm([], [{ id: 1, kind: 'meetingSummary', label: 'X', error: 'boom', meetingId: null, outcome: { type: 'failed', error: 'boom' } }]),
    );
    expect(v.rows[0].retryable).toBeFalsy();
    expect(v.rows[0].action).toBe('dismiss');
  });

  it('shows a synthetic transcribing row while a recording is processing, and hides it otherwise', () => {
    const processing = buildQueueView(backlog([]), llm([]), { isProcessing: true, title: 'Standup' });
    expect(processing.rows).toHaveLength(1);
    expect(processing.rows[0]).toMatchObject({
      id: 'transcription',
      title: 'Standup',
      stage: 'transcribing',
      stageLabel: 'Transcribing',
      source: 'recording',
      meetingId: null,
    });
    expect(processing.count).toBe(1);
    expect(processing.lamp).toBe('amber');

    const idle = buildQueueView(backlog([]), llm([]), { isProcessing: false, title: 'Standup' });
    expect(idle.rows).toHaveLength(0);
  });

  it('falls back to a generic title when the recording has none yet', () => {
    const v = buildQueueView(backlog([]), llm([]), { isProcessing: true, title: null });
    expect(v.rows[0].title).toBe('Recording');
  });

  it('sorts the transcribing row with active work, ahead of waiting items', () => {
    const v = buildQueueView(
      backlog([{ meeting: meeting('b', 'Design review'), status: 'waiting' }]),
      llm([]),
      { isProcessing: true, title: 'Standup' },
    );
    expect(v.rows.map((r) => r.stage)).toEqual(['transcribing', 'waiting']);
    expect(v.running?.id).toBe('transcription');
    expect(v.count).toBe(2);
    expect(v.lamp).toBe('amber');
  });

  // specs/0063 W3 Task 6 — the numeric registry id, kept separate from the prefixed
  // string `id` so a Retry click never has to string-slice `llm:` back off.
  it('carries the numeric task id on a running LLM row', () => {
    const v = buildQueueView(
      backlog([]),
      llm([{ id: 42, kind: 'meetingSummary', label: 'Summarizing', note: null, meetingId: 'm1' }]),
    );
    expect(v.rows[0].taskId).toBe(42);
  });

  it('carries the numeric task id on a failed LLM row', () => {
    const v = buildQueueView(
      backlog([]),
      llm([], [{ id: 7, kind: 'prepBrief', label: 'X', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );
    expect(v.rows[0].taskId).toBe(7);
  });

  it('has no task id on a backlog row', () => {
    const v = buildQueueView(backlog([{ meeting: meeting('a', 'X'), status: 'error' }]), llm([]));
    expect(v.rows[0].taskId).toBeUndefined();
  });

  it('defaults the recording parameter when omitted, matching existing two-argument callers', () => {
    const v = buildQueueView(backlog([]), llm([]));
    expect(v.rows).toEqual([]);
  });
});
