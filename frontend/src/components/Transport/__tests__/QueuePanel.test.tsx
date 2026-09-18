import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

const { backlog, llm, invokeMock } = vi.hoisted(() => ({
  backlog: { view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 }, stop: vi.fn(), startNow: vi.fn(), dismissDone: vi.fn(), enqueueMeeting: vi.fn() },
  // The provider's context value is FLAT (`{...view, dismiss, retry}` — LlmActivityProvider.tsx),
  // so the fixture mirrors that shape rather than nesting a `view`.
  llm: { running: [] as unknown[], history: [] as unknown[], hasFailure: false, dismiss: vi.fn(), retry: vi.fn() },
  invokeMock: vi.fn(),
}));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({ useBacklog: () => backlog }));
vi.mock('@/contexts/LlmActivityProvider', () => ({ useOptionalLlmActivity: () => llm }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { QueuePanel } from '@/components/Transport/QueuePanel';
import { buildQueueView } from '@/lib/transport/queue-view';
import type { BacklogView } from '@/lib/deferred-backlog';
import type { LlmActivityView } from '@/contexts/LlmActivityProvider';

const meeting = (id: string, title: string) => ({ id, title, folderPath: '/x', transcriptCount: 3 });
const backlogView = (items: BacklogView['items']): BacklogView => ({
  items,
  processing: false,
  pendingCount: items.filter((i) => i.status === 'waiting').length,
  active: null,
  activeOrdinal: 0,
  total: items.length,
});
const llmView = (running: LlmActivityView['running'], history: LlmActivityView['history'] = []): LlmActivityView => ({
  running,
  history,
  hasFailure: history.some((h) => h.outcome.type === 'failed'),
});

beforeEach(() => {
  backlog.view = { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 };
  Object.assign(llm, { running: [], history: [], hasFailure: false });
  vi.clearAllMocks();
  invokeMock.mockResolvedValue(undefined);
});

// specs/0063 W3 Task 6 — the queue's Retry/Dismiss buttons must be real, not decorative.
describe('QueuePanel', () => {
  it('renders Retry on a failed backlog row and force-enqueues that meeting', () => {
    const view = buildQueueView(backlogView([{ meeting: meeting('m1', 'Design review'), status: 'error' }]), llmView([]));
    render(<QueuePanel view={view} />);

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

    expect(backlog.enqueueMeeting).toHaveBeenCalledWith('m1', { force: true });
    expect(invokeMock).not.toHaveBeenCalledWith('api_llm_activity_retry_task', expect.anything());
  });

  it('renders Retry on a failed retryable-kind LLM row and retries by numeric task id', () => {
    const view = buildQueueView(
      backlogView([]),
      llmView([], [{ id: 7, kind: 'meetingSummary', label: 'Summarizing Q3', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );
    render(<QueuePanel view={view} />);

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

    expect(invokeMock).toHaveBeenCalledWith('api_llm_activity_retry_task', { taskId: 7 });
    expect(backlog.enqueueMeeting).not.toHaveBeenCalled();
  });

  it('does not render Retry for a failed askAI task', () => {
    const view = buildQueueView(
      backlogView([]),
      llmView([], [{ id: 9, kind: 'askAI', label: 'Ask AI', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );
    render(<QueuePanel view={view} />);

    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('still clears the whole failure history from the header "Dismiss failures" control', () => {
    const view = buildQueueView(
      backlogView([]),
      llmView([], [{ id: 9, kind: 'askAI', label: 'Ask AI', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );
    render(<QueuePanel view={view} />);

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss failures' }));

    expect(llm.dismiss).toHaveBeenCalled();
  });
});
