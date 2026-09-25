import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// The queue view is computed ONCE (QueueViewProvider) and shared by the sidebar's queue row
// and the "Processing" ids the pages read — not rebuilt by each consumer, and not rebuilt
// when the LLM provider hands out a fresh `{...view, dismiss}` object with the same data.

const { state, buildSpy } = vi.hoisted(() => ({
  state: {
    backlog: { view: { items: [] as unknown[], processing: false } },
    llm: { queued: [] as unknown[], running: [] as unknown[], history: [] as unknown[], hasFailure: false, dismiss: () => {} },
  },
  buildSpy: vi.fn(),
}));

vi.mock('@/contexts/DeferredBacklogProvider', () => ({ useBacklog: () => state.backlog }));
vi.mock('@/contexts/LlmActivityProvider', () => ({ useOptionalLlmActivity: () => state.llm }));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isProcessing: false }) }));
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => ({ meetingTitle: null }) }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => ({ activeRecordingMeetingId: null }) }));
vi.mock('@/lib/transport/queue-view', async (orig) => {
  const real = await orig<typeof import('@/lib/transport/queue-view')>();
  return {
    ...real,
    buildQueueView: (...args: Parameters<typeof real.buildQueueView>) => {
      buildSpy();
      return real.buildQueueView(...args);
    },
  };
});

import { QueueViewProvider, useQueueView } from '@/contexts/QueueViewContext';
import { ProcessingMeetingsProvider, useProcessingMeetingIds } from '@/contexts/ProcessingMeetingsContext';

function RailProbe() {
  const v = useQueueView();
  return <span data-testid="rail">{v.count}</span>;
}
function PageProbe() {
  const ids = useProcessingMeetingIds();
  return <span data-testid="page">{[...ids].join(',')}</span>;
}
function tree() {
  return (
    <ProcessingMeetingsProvider>
      <QueueViewProvider>
        <RailProbe />
        <RailProbe />
      </QueueViewProvider>
      <PageProbe />
    </ProcessingMeetingsProvider>
  );
}

beforeEach(() => {
  buildSpy.mockClear();
  state.llm = {
    queued: [],
    running: [{ id: 1, kind: 'meetingSummary', label: 'Summary', note: null, meetingId: 'm-1' }],
    history: [],
    hasFailure: false,
    dismiss: () => {},
  };
});

describe('QueueViewProvider', () => {
  it('builds the view once for every consumer and publishes the processing ids', () => {
    render(tree());
    expect(buildSpy).toHaveBeenCalledTimes(1);
    expect(screen.getAllByTestId('rail').map((e) => e.textContent)).toEqual(['1', '1']);
    expect(screen.getByTestId('page').textContent).toBe('m-1');
  });

  it('does not rebuild when the LLM provider re-creates its value object with the same data', () => {
    const { rerender } = render(tree());
    expect(buildSpy).toHaveBeenCalledTimes(1);
    state.llm = { ...state.llm, dismiss: () => {} };
    rerender(tree());
    expect(buildSpy).toHaveBeenCalledTimes(1);
  });
});
