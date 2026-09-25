import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

/**
 * specs/0064 W5 — the queue moved off the transport rail into the sidebar, below Settings.
 * These three cases came with it from `TransportRail.test.tsx`; the fourth and fifth are new
 * and cover the two states the sidebar has that the rail did not.
 */

const { backlog, llm, transcripts, invokeMock } = vi.hoisted(() => ({
  backlog: { view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 }, stop: vi.fn(), startNow: vi.fn(), dismissDone: vi.fn(), enqueueMeeting: vi.fn() },
  // The provider's context value is FLAT (`{...view, dismiss}` — LlmActivityProvider.tsx),
  // so the fixture mirrors that shape rather than nesting a `view`.
  llm: { running: [] as unknown[], history: [] as unknown[], hasFailure: false, dismiss: vi.fn(), retry: vi.fn() },
  transcripts: { meetingTitle: 'Pricing sync' },
  invokeMock: vi.fn(),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isProcessing: false }) }));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({ useBacklog: () => backlog }));
vi.mock('@/contexts/LlmActivityProvider', () => ({ useOptionalLlmActivity: () => llm }));
// A real (unmocked) useState, not a fixture: the popover's open/close is local UI state,
// not something any test needs to seed or assert on directly — only the resulting DOM.
vi.mock('@/contexts/QueueOpenContext', async () => {
  const react = await import('react');
  return { useQueueOpen: () => { const [open, setOpen] = react.useState(false); return { open, setOpen }; } };
});
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => transcripts }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => ({ activeRecordingMeetingId: null }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import { QueueRow } from '@/components/Sidebar/QueueRow';
// The row reads the shared view computed once by QueueViewProvider (AppShell).
import { QueueViewProvider } from '@/contexts/QueueViewContext';

beforeEach(() => {
  Object.assign(llm, { running: [], history: [], hasFailure: false });
  // The queue fixtures below mutate `backlog.view` in place — without this reset the last
  // test to set it leaks its items into every test that runs after (fixture leak, review).
  backlog.view = { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 };
  vi.clearAllMocks();
  // The panel's retry chains `.catch` onto the invoke result, so the mock must be a promise
  // — `clearAllMocks` drops the implementation, hence re-arming it here.
  invokeMock.mockResolvedValue(undefined);
});

describe('QueueRow (specs/0064 W5)', () => {
  // The lamp inside the labelled trigger must not pollute its accessible name.
  it('is named by its own text, not the lamp', () => {
    render(<QueueViewProvider><QueueRow /></QueueViewProvider>);
    expect(screen.getByRole('button', { name: 'Queue 0 Idle' })).toBeTruthy();
  });

  it('shows count and stage, and opens the panel', () => {
    backlog.view = { items: [{ meeting: { id: 'a', title: 'Hiring loop debrief', folderPath: '/x', transcriptCount: 1 }, status: 'transcribing' }], pendingCount: 0, processing: true, active: null, activeOrdinal: 1, total: 1 } as typeof backlog.view;
    render(<QueueViewProvider><QueueRow /></QueueViewProvider>);

    const btn = screen.getByRole('button', { name: /queue/i });
    expect(btn.textContent).toMatch(/1/);
    expect(btn.textContent).toMatch(/transcribing/i);

    fireEvent.click(btn);
    expect(screen.getByRole('dialog', { name: /queue/i })).toBeTruthy();
    expect(screen.getByText('Hiring loop debrief')).toBeTruthy();
  });

  // The header's "Dismiss failures" clears the WHOLE history via `api_llm_activity_dismiss`
  // (no id); per-row Dismiss (specs/0063 W3 Task 6) uses `api_llm_activity_dismiss_task` to
  // remove only that one record. This covers per-row Retry by numeric task id, and the
  // header dismiss clearing everything.
  it('panel: per-row Retry dispatches by numeric task id, dismiss still clears the lot', () => {
    Object.assign(llm, {
      hasFailure: true,
      history: [{ id: 7, kind: 'prepBrief', label: 'Prep brief — Q3 planning', error: 'Ollama unreachable', meetingId: 'q3', outcome: { type: 'failed', error: 'Ollama unreachable' } }],
    });
    render(<QueueViewProvider><QueueRow /></QueueViewProvider>);

    fireEvent.click(screen.getByRole('button', { name: /queue/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(invokeMock).toHaveBeenCalledWith('api_llm_activity_retry_task', { taskId: 7 });

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss failures' }));
    expect(llm.dismiss).toHaveBeenCalledTimes(1);
  });

  it('collapses to the lamp alone, still named and still opening the panel', () => {
    backlog.view = { items: [{ meeting: { id: 'a', title: 'Hiring loop debrief', folderPath: '/x', transcriptCount: 1 }, status: 'transcribing' }], pendingCount: 0, processing: true, active: null, activeOrdinal: 1, total: 1 } as typeof backlog.view;
    render(<QueueViewProvider><QueueRow collapsed /></QueueViewProvider>);

    // No visible label text — the state is the lamp — but the button still says what it is.
    const btn = screen.getByRole('button', { name: /queue 1/i });
    expect(btn.textContent?.trim()).toBe('');

    fireEvent.click(btn);
    expect(screen.getByRole('dialog', { name: /queue/i })).toBeTruthy();
  });

  it('keeps the status line to one line so the rows below it never shift', () => {
    backlog.view = { items: [{ meeting: { id: 'a', title: 'A meeting with a very long title that would otherwise wrap onto a second line', folderPath: '/x', transcriptCount: 1 }, status: 'transcribing' }], pendingCount: 0, processing: true, active: null, activeOrdinal: 1, total: 1 } as typeof backlog.view;
    render(<QueueViewProvider><QueueRow /></QueueViewProvider>);

    const line = screen.getByText(/A meeting with a very long title/);
    expect(line.className).toContain('truncate');
  });
});
