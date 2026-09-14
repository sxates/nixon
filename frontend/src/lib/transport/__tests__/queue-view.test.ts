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
const llm = (running: LlmActivityView['running'], history: LlmActivityView['history'] = []): LlmActivityView => ({
  running, history, hasFailure: history.some((h) => h.outcome.type === 'failed'),
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
  it('ignores skipped and successful AI history', () => {
    const v = buildQueueView(backlog([]), llm([], [
      { id: 1, kind: 'prepBrief', label: 'ok', error: null, meetingId: null, outcome: { type: 'success' } },
      { id: 2, kind: 'prepBrief', label: 'skip', error: null, meetingId: null, outcome: { type: 'skipped', reason: 'no transcript' } },
    ]));
    expect(v.rows).toEqual([]);
    expect(v.lamp).toBe('off');
  });
  it('ignores failed history the registry did not flag (a foreground failure already seen)', () => {
    const v = buildQueueView(backlog([]), {
      running: [], hasFailure: false,
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
});
