import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import type { BacklogItem, BacklogItemStatus } from '@/lib/deferred-backlog';

// specs/0071 W3 — stopping a meeting that was deferred at any point queues a full repass
// (retranscribe, then summarize). On the reported 114.6s recording that took 3 min 19 s, and
// the page the user lands on said nothing: the sidebar queue knew, the meeting did not, so a
// stale transcript looked like the final answer.
//
// The spec's premise was wrong about the cause — the stop path ALREADY hands the repass to
// this controller (`enqueueMeeting(id, {force: true})`). The gap was that this panel imported
// `useBacklog` and used only `enqueueMeeting`, never asking whether its own meeting was in
// flight.

const { invoke, backlog } = vi.hoisted(() => ({
  invoke: vi.fn(),
  backlog: { items: [] as BacklogItem[] },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({
    view: { items: backlog.items, pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 },
    enqueueMeeting: vi.fn(),
    stop: vi.fn(),
    dismissDone: vi.fn(),
    startNow: vi.fn(),
  }),
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ betaFeatures: { importAndRetranscribe: true } }),
}));
vi.mock('@/hooks/useDiarization', () => ({
  useDiarization: () => ({ isRunning: false, stage: null, progressPct: null, identifySpeakers: vi.fn() }),
}));
vi.mock('../RetranscribeDialog', () => ({ RetranscribeDialog: () => null }));
vi.mock('@/components/VirtualizedTranscriptView', () => ({
  VirtualizedTranscriptView: () => <div data-testid="transcript-list" />,
}));

import { TranscriptPanel } from '@/components/MeetingDetails/TranscriptPanel';

function item(status: BacklogItemStatus): BacklogItem {
  return {
    meeting: { id: 'm1', title: 'T', folderPath: '/tmp', transcriptCount: 4 },
    status,
  };
}

const props = {
  meetingId: 'm1',
  transcripts: [{ id: 't1', text: 'hello', timestamp: '0' }],
  onCopyTranscript: vi.fn(),
  onOpenMeetingFolder: vi.fn().mockResolvedValue(undefined),
  speakersController: { speakers: [] },
} as unknown as React.ComponentProps<typeof TranscriptPanel>;

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(null);
  backlog.items = [];
});

describe('W3 — the meeting says when it is being processed', () => {
  it('says nothing when the meeting is not in the queue', async () => {
    render(<TranscriptPanel {...props} />);
    await waitFor(() => expect(screen.getByTestId('transcript-list')).toBeInTheDocument());
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('says it is re-transcribing while the repass runs', async () => {
    backlog.items = [item('transcribing')];
    render(<TranscriptPanel {...props} />);
    const note = await screen.findByRole('status');
    expect(note.textContent).toMatch(/Re-transcribing this meeting/);
    expect(note.textContent).toMatch(/will update when it finishes/);
  });

  it('distinguishes queued from running', async () => {
    backlog.items = [item('waiting')];
    render(<TranscriptPanel {...props} />);
    expect((await screen.findByRole('status')).textContent).toMatch(/Queued for processing/);
  });

  it('says it is summarizing at the summarize stage', async () => {
    backlog.items = [item('summarizing')];
    render(<TranscriptPanel {...props} />);
    expect((await screen.findByRole('status')).textContent).toMatch(/Summarizing this meeting/);
  });

  it('stops saying so once the work reaches a terminal state', async () => {
    backlog.items = [item('done')];
    render(<TranscriptPanel {...props} />);
    await waitFor(() => expect(screen.getByTestId('transcript-list')).toBeInTheDocument());
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('ignores another meeting being processed', async () => {
    backlog.items = [
      { meeting: { id: 'other', title: 'X', folderPath: '/tmp', transcriptCount: 0 }, status: 'transcribing' },
    ];
    render(<TranscriptPanel {...props} />);
    await waitFor(() => expect(screen.getByTestId('transcript-list')).toBeInTheDocument());
    expect(screen.queryByRole('status')).toBeNull();
  });
});
