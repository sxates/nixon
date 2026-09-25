import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, act } from '@testing-library/react';
import {
  ProcessingMeetingsProvider,
  useProcessingMeetingIds,
  usePublishProcessingMeetingIds,
} from '@/contexts/ProcessingMeetingsContext';
import { buildQueueView, processingMeetingIds } from '@/lib/transport/queue-view';
import { itemVisualState, type TimelineContext } from '@/lib/today-timeline';
import { StateChip } from '@/components/Today/TimelineBlock';
import type { BacklogItem, BacklogItemStatus, BacklogView } from '@/lib/deferred-backlog';
import type { LlmActivityView } from '@/contexts/LlmActivityProvider';
import type { DayAgendaItem } from '@/lib/day-agenda';

// Owner feedback: Today and All Meetings had Recording / Now / Recorded but nothing for the
// minutes after a stop while transcription, speakers and the summary run. "Processing" is
// derived from the queue rail's own rows so a list can never disagree with the rail.

function backlogItem(id: string, status: BacklogItemStatus): BacklogItem {
  return { meeting: { id, title: id, folderPath: '/x', transcriptCount: 0 }, status };
}
function backlog(items: BacklogItem[], processing = false): BacklogView {
  return { items, pendingCount: 0, processing, active: null, activeOrdinal: 0, total: items.length };
}
const NO_LLM: LlmActivityView = { queued: [], running: [], history: [], hasFailure: false };

function ids(b: BacklogView, llm: LlmActivityView | null, rec = { isProcessing: false, meetingId: null as string | null }) {
  return processingMeetingIds(buildQueueView(b, llm, { ...rec, title: null }), b.processing);
}

describe('processingMeetingIds', () => {
  it('includes every running backlog stage and running/queued AI tasks with a meeting', () => {
    const got = ids(
      backlog([
        backlogItem('m-trans', 'transcribing'),
        backlogItem('m-diar', 'diarizing'),
        backlogItem('m-sum', 'summarizing'),
      ]),
      {
        ...NO_LLM,
        running: [{ id: 1, kind: 'prepBrief', label: 'Prep', note: null, meetingId: 'm-prep' }],
        queued: [{ id: 2, kind: 'actionItems', label: 'Items', meetingId: 'm-queued' }],
      },
    );
    expect([...got].sort()).toEqual(['m-diar', 'm-prep', 'm-queued', 'm-sum', 'm-trans']);
  });

  it('excludes failures, finished work, and a parked backlog item with no drain running', () => {
    const got = ids(
      backlog([backlogItem('m-err', 'error'), backlogItem('m-done', 'done'), backlogItem('m-parked', 'waiting')]),
      {
        ...NO_LLM,
        hasFailure: true,
        history: [
          { id: 3, kind: 'meetingSummary', label: 'S', error: 'boom', meetingId: 'm-llm-err', outcome: { type: 'failed', error: 'boom' } },
          { id: 4, kind: 'prepBrief', label: 'P', error: null, meetingId: 'm-prep-done', outcome: { type: 'success' } },
        ],
      },
    );
    expect(got.size).toBe(0);
  });

  it('counts a waiting backlog item while a drain is running', () => {
    expect(ids(backlog([backlogItem('m-next', 'waiting')], true), null).has('m-next')).toBe(true);
  });

  it("covers the just-stopped recording's final transcription pass", () => {
    const got = ids(backlog([]), null, { isProcessing: true, meetingId: 'm-live' });
    expect(got.has('m-live')).toBe(true);
  });
});

function iso(hour: number): string {
  return new Date(2026, 6, 4, hour, 0, 0, 0).toISOString();
}
function item(partial: Partial<DayAgendaItem>): DayAgendaItem {
  return {
    id: 'row-1',
    title: 'Design review',
    startTime: iso(10),
    endTime: iso(11),
    source: 'recording',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 0,
    meetingId: 'm-1',
    status: { recorded: true, transcribed: false, summarized: false, speakersIdentified: false },
    dismissed: false,
    seriesKey: null,
    ...partial,
  };
}
const ctx = (over: Partial<TimelineContext>): TimelineContext => ({
  now: new Date(2026, 6, 4, 12, 0),
  isRecording: false,
  recordingThisId: null,
  ...over,
});

describe('itemVisualState — Processing precedence', () => {
  const processing = new Set(['m-1']);

  it('Recording beats Processing', () => {
    expect(
      itemVisualState(item({}), ctx({ isRecording: true, recordingThisId: 'row-1', processingIds: processing })),
    ).toBe('recording');
  });

  it('Processing beats Recorded (past) and Now', () => {
    expect(itemVisualState(item({}), ctx({ processingIds: processing }))).toBe('processing');
    const live = item({ source: 'calendar', startTime: iso(11), endTime: iso(13) });
    expect(itemVisualState(live, ctx({}))).toBe('now');
    expect(itemVisualState(live, ctx({ processingIds: processing }))).toBe('processing');
  });

  it('a prep brief running for an upcoming meeting shows Processing on its row', () => {
    const upcoming = item({ source: 'manual', startTime: iso(15), endTime: iso(16), status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false } });
    expect(itemVisualState(upcoming, ctx({}))).toBe('upcoming');
    expect(itemVisualState(upcoming, ctx({ processingIds: processing }))).toBe('processing');
  });

  it('a row with no meeting never shows Processing', () => {
    expect(itemVisualState(item({ meetingId: null }), ctx({ processingIds: new Set(['']) }))).toBe('past-recorded');
  });

  it('StateChip renders the Processing label with a still lamp', () => {
    const { container } = render(<StateChip state="processing" />);
    expect(container.textContent).toBe('Processing');
    const lamp = container.querySelector('[data-tone="amber"]');
    expect(lamp).not.toBeNull();
    expect(lamp!.className).not.toMatch(/animate-/);
  });
});

describe('ProcessingMeetingsProvider', () => {
  it('re-publishes only when the set of ids changes', () => {
    const seen: ReadonlySet<string>[] = [];
    let publish!: (ids: ReadonlySet<string>) => void;
    function Probe() {
      publish = usePublishProcessingMeetingIds()!;
      seen.push(useProcessingMeetingIds());
      return null;
    }
    render(
      <ProcessingMeetingsProvider>
        <Probe />
      </ProcessingMeetingsProvider>,
    );
    act(() => publish(new Set(['m-1'])));
    const afterFirst = seen.length;
    expect(seen[afterFirst - 1].has('m-1')).toBe(true);
    // Same ids again: no new value, so consumers (the pages) don't re-render.
    act(() => publish(new Set(['m-1'])));
    expect(seen.length).toBe(afterFirst);
    act(() => publish(new Set()));
    expect(seen[seen.length - 1].size).toBe(0);
  });
});
