import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// specs/0057 Task 6 — the timeline block stopped being a glowing rounded card and
// became a transport-log panel: a hairline `rounded-[3px]` panel whose STATE is
// carried by a 3px left bar (the tape-label spine), not by a coloured glow. The bar
// colour per state and the engraved chip text are the contract this pins down.

import type { DayAgendaItem } from '@/lib/day-agenda';
import type { TimelineContext, TimelineVisualState } from '@/lib/today-timeline';
import { TimelineBlock } from '@/components/Today/TimelineBlock';

// The block used to take `canHide`/`canEdit` as booleans; since 2026-09-21 the Hide/Edit/
// Delete gate lives in the shared `AgendaRowMenu`, which derives it from the item + this
// context — so the grid, the list and the week view can't disagree about what a row offers.
const ctx: TimelineContext = {
  now: new Date('2026-09-13T15:30:00.000Z'),
  isRecording: false,
  recordingThisId: null,
};

const item: DayAgendaItem = {
  id: 'evt-1',
  title: 'Quarterly Planning',
  startTime: '2026-09-13T15:00:00.000Z',
  endTime: '2026-09-13T16:00:00.000Z',
  source: 'calendar',
  zoomUrl: null,
  attendees: [],
  attendeeCount: 0,
  meetingId: null,
  status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false },
  dismissed: false,
};

function renderBlock(state: TimelineVisualState, canJoin = false, height = 60) {
  const { container } = render(
    <TimelineBlock
      item={item}
      state={state}
      top={0}
      height={height}
      lane={0}
      laneCount={1}
      canJoin={canJoin}
      canRecord={false}
      ctx={ctx}
      onSelect={vi.fn()}
      onJoin={vi.fn()}
      onHide={vi.fn()}
      onEdit={vi.fn()}
      onDelete={vi.fn()}
      onRecord={vi.fn()}
    />,
  );
  return container;
}

describe('TimelineBlock state bar (specs/0057 Task 6)', () => {
  const cases: Array<[TimelineVisualState, string]> = [
    ['recording', 'bg-record'],
    ['now-joinable', 'bg-brand'],
    ['now', 'bg-brand/50'],
    ['upcoming', 'bg-muted-foreground/40'],
    ['past-recorded', 'bg-success'],
  ];

  it.each(cases)('%s gets a 3px left bar coloured %s', (state, expected) => {
    const container = renderBlock(state);
    const bar = container.querySelector('[data-state-bar]');
    expect(bar).not.toBeNull();
    expect(bar!.className).toContain('w-[3px]');
    expect(bar!.className).toContain(expected);
  });

  it('an unrecorded past event has no bar at all — a dashed outline instead', () => {
    const container = renderBlock('past-unrecorded');
    expect(container.querySelector('[data-state-bar]')).toBeNull();
    expect(container.querySelector('[role="button"]')!.className).toContain('border-dashed');
  });

  it('drops the old glow/pill chrome from every state', () => {
    for (const [state] of cases) {
      const container = renderBlock(state);
      const block = container.querySelector('[role="button"]')!;
      expect(block.className).toContain('rounded-[3px]');
      expect(block.className).not.toContain('rounded-xl');
      expect(container.querySelectorAll('.animate-ping')).toHaveLength(0);
      expect(container.querySelectorAll('.rounded-full')).toHaveLength(0);
    }
  });
});

describe('TimelineBlock chip text (specs/0057 Task 6)', () => {
  it.each([
    ['recording', 'Recording'],
    ['now-joinable', 'Join & record'],
    ['now', 'Now'],
    ['upcoming', 'Prep'],
    ['past-recorded', 'Recorded'],
  ] as Array<[TimelineVisualState, string]>)('%s reads "%s"', (state, label) => {
    renderBlock(state);
    expect(screen.getByText(label)).toBeInTheDocument();
  });

  it('a past unrecorded event carries no chip', () => {
    renderBlock('past-unrecorded');
    for (const label of ['Recording', 'Join & record', 'Now', 'Prep', 'Recorded']) {
      expect(screen.queryByText(label)).not.toBeInTheDocument();
    }
  });

  it('a joinable block offers Join & record as a brand button', () => {
    renderBlock('now-joinable', true);
    expect(screen.getByRole('button', { name: 'Join & record' }).className).toContain('bg-brand');
  });
});

describe('TimelineBlock manual entry actions (specs/0069 W3)', () => {
  const manualItem: DayAgendaItem = {
    ...item,
    id: 'meeting-1',
    source: 'manual',
    meetingId: 'meeting-1',
    calendarEventId: 'nixon-manual:meeting-1',
  };

  it('shows a Record button (not Join & record) for a manual, unrecorded entry', () => {
    const onRecord = vi.fn();
    const onSelect = vi.fn();
    render(
      <TimelineBlock
        item={manualItem}
        state="upcoming"
        top={0}
        height={60}
        lane={0}
        laneCount={1}
        canJoin={false}
        canRecord
        ctx={ctx}
        onSelect={onSelect}
        onJoin={vi.fn()}
        onHide={vi.fn()}
        onEdit={vi.fn()}
        onDelete={vi.fn()}
        onRecord={onRecord}
      />,
    );
    const recordButton = screen.getByRole('button', { name: 'Record' });
    fireEvent.click(recordButton);
    // The button stops propagation, same treatment as Join & Record — clicking it
    // must never also fire the block's own onSelect (which would route to Prep).
    expect(onRecord).toHaveBeenCalledWith(manualItem);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('offers Edit and Delete in the ⋯ menu for a manual, unrecorded entry', async () => {
    const onEdit = vi.fn();
    const onDelete = vi.fn();
    render(
      <TimelineBlock
        item={manualItem}
        state="upcoming"
        top={0}
        height={60}
        lane={0}
        laneCount={1}
        canJoin={false}
        canRecord={false}
        ctx={ctx}
        onSelect={vi.fn()}
        onJoin={vi.fn()}
        onHide={vi.fn()}
        onEdit={onEdit}
        onDelete={onDelete}
        onRecord={vi.fn()}
      />,
    );

    const trigger = screen.getByRole('button', { name: 'Event options' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Edit'));
    expect(onEdit).toHaveBeenCalledWith(manualItem);

    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Delete'));
    expect(onDelete).toHaveBeenCalledWith(manualItem);

    // Never "Hide from timeline" — deleting a manual row is the way to get rid of
    // it, not hiding it (it isn't a calendar event to dismiss).
    expect(screen.queryByText('Hide from timeline')).not.toBeInTheDocument();
  });
});

describe('TimelineBlock vertical alignment (owner feedback 2026-09-24)', () => {
  it('a compact block centres its single line, so blocks of different heights line up', () => {
    const block = renderBlock('past-recorded', false, 40).querySelector('[role="button"]')!;
    expect(block.className).toContain('justify-center');
  });

  it('a full block keeps its two lines anchored to the top', () => {
    const block = renderBlock('past-recorded', false, 60).querySelector('[role="button"]')!;
    expect(block.className).not.toContain('justify-center');
  });
});
