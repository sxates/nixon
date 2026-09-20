import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

const { backlog, llm, invokeMock, toastMock } = vi.hoisted(() => ({
  backlog: { view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 }, stop: vi.fn(), startNow: vi.fn(), dismissDone: vi.fn(), enqueueMeeting: vi.fn() },
  // The provider's context value is FLAT (`{...view, dismiss}` — LlmActivityProvider.tsx),
  // so the fixture mirrors that shape rather than nesting a `view`.
  llm: { running: [] as unknown[], history: [] as unknown[], hasFailure: false, dismiss: vi.fn(), retry: vi.fn() },
  invokeMock: vi.fn(),
  toastMock: { error: vi.fn(), success: vi.fn(), warning: vi.fn() },
}));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({ useBacklog: () => backlog }));
vi.mock('@/contexts/LlmActivityProvider', () => ({ useOptionalLlmActivity: () => llm }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({ toast: toastMock }));

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
  // `enqueueMeeting` always resolves to a HandoffOutcome; a bare undefined here would be
  // a fixture that cannot represent the refusal the panel now has to report.
  backlog.enqueueMeeting.mockResolvedValue({ accepted: true });
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

  // fix-round 1: the first version of this wired every row's Dismiss to the header's
  // full-history api_llm_activity_dismiss, so one row's button silently cleared every
  // OTHER failure too. It must dismiss only its own record, by numeric task id.
  it('renders Dismiss on a failed askAI row and dismisses only that task, not the global history', () => {
    const view = buildQueueView(
      backlogView([]),
      llmView([], [{ id: 9, kind: 'askAI', label: 'Ask AI', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );
    render(<QueuePanel view={view} />);

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));

    expect(invokeMock).toHaveBeenCalledWith('api_llm_activity_dismiss_task', { taskId: 9 });
    expect(llm.dismiss).not.toHaveBeenCalled();
  });

  it('still clears the whole failure history from the header "Dismiss failures" control', () => {
    const view = buildQueueView(
      backlogView([]),
      llmView([], [{ id: 9, kind: 'askAI', label: 'Ask AI', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );
    render(<QueuePanel view={view} />);

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss failures' }));

    expect(llm.dismiss).toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalledWith('api_llm_activity_dismiss_task', expect.anything());
  });
});

// specs/0066 W2 — "Retry does nothing" came back after 0063 W3 fixed the wiring, because
// nothing here tested the FAILURE path: both dispatches ended in `.catch(() => {})`, so a
// refused retry looked exactly like a broken button. These four pin the reporting.
describe('QueuePanel — a refused retry says so (specs/0066 W2)', () => {
  const summaryFailure = () =>
    buildQueueView(
      backlogView([]),
      llmView([], [{ id: 7, kind: 'meetingSummary', label: 'Summarizing Q3', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );

  it('surfaces the Rust error when the retry dispatch is refused', async () => {
    invokeMock.mockRejectedValueOnce('Speaker identification is already running for this meeting.');
    render(<QueuePanel view={summaryFailure()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith('Could not retry', {
        description: 'Speaker identification is already running for this meeting.',
      }),
    );
  });

  it('says nothing when the retry is accepted', async () => {
    render(<QueuePanel view={summaryFailure()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_llm_activity_retry_task', { taskId: 7 }));
    expect(toastMock.error).not.toHaveBeenCalled();
  });

  it('reports a backlog hand-off the queue refused, in the user\'s terms', async () => {
    backlog.enqueueMeeting.mockResolvedValueOnce({ accepted: false, reason: 'no-folder-path' });
    const view = buildQueueView(backlogView([{ meeting: meeting('m1', 'Design review'), status: 'error' }]), llmView([]));
    render(<QueuePanel view={view} />);

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith('Could not retry', {
        description: "This meeting's recording folder is missing, so there is nothing to process.",
      }),
    );
  });

  it('surfaces a refused per-row Dismiss too', async () => {
    invokeMock.mockRejectedValueOnce('That task is no longer in the queue.');
    const view = buildQueueView(
      backlogView([]),
      llmView([], [{ id: 9, kind: 'askAI', label: 'Ask AI', error: 'boom', meetingId: 'm1', outcome: { type: 'failed', error: 'boom' } }]),
    );
    render(<QueuePanel view={view} />);

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith('Could not dismiss', {
        description: 'That task is no longer in the queue.',
      }),
    );
  });
});
