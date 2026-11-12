import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// Owner feedback 2026-09-21: "the 'week' view is missing the … actions menu that lets me
// remove items." It was missing because this view shipped (specs/0038 WS4) as a read-only
// overview and never grew the affordance the grid and the list both have — and it had no
// test file at all, so nothing noticed. These tests pin the parity.

import type { DayAgendaItem } from '@/lib/day-agenda';
import type { TimelineContext } from '@/lib/today-timeline';
import { WeekView } from '@/components/Today/WeekView';

const NOW = new Date('2026-09-16T10:00:00.000Z');
const ctx: TimelineContext = { now: NOW, isRecording: false, recordingThisId: null };

function event(title: string, overrides: Partial<DayAgendaItem> = {}): DayAgendaItem {
  return {
    id: `evt-${title}`,
    title,
    startTime: '2026-09-16T15:00:00.000Z',
    endTime: '2026-09-16T16:00:00.000Z',
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 0,
    meetingId: null,
    status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false },
    dismissed: false,
    ...overrides,
  };
}

function renderWeek(items: DayAgendaItem[], over: Partial<Parameters<typeof WeekView>[0]> = {}) {
  const actions = {
    onHide: vi.fn(),
    onEdit: vi.fn(),
    onDelete: vi.fn(),
    onRecord: vi.fn(),
  };
  render(
    <WeekView
      weekDays={['2026-09-16']}
      weekItems={[items]}
      ctx={ctx}
      now={NOW}
      onSelectItem={vi.fn()}
      onOpenDay={vi.fn()}
      actions={actions}
      {...over}
    />,
  );
  return actions;
}

function openRowMenu() {
  const trigger = screen.getByRole('button', { name: 'Event options' });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  return trigger;
}

describe('WeekView row actions', () => {
  it('offers Hide for an unrecorded calendar event — the reported gap', async () => {
    const item = event('Lunch');
    const actions = renderWeek([item]);

    openRowMenu();
    fireEvent.click(await screen.findByText('Hide from timeline'));
    expect(actions.onHide).toHaveBeenCalledWith(item);
  });

  it('offers Edit and Delete (never Hide) for a manually added meeting', async () => {
    const item = event('Coffee', {
      source: 'manual',
      meetingId: 'meeting-1',
      calendarEventId: 'nixon-manual:meeting-1',
    });
    const actions = renderWeek([item]);

    const trigger = openRowMenu();
    fireEvent.click(await screen.findByText('Edit'));
    expect(actions.onEdit).toHaveBeenCalledWith(item);

    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Delete'));
    expect(actions.onDelete).toHaveBeenCalledWith(item);

    // Deleting a manual row is how you get rid of it; it isn't a calendar event to dismiss.
    expect(screen.queryByText('Hide from timeline')).not.toBeInTheDocument();
  });

  // Owner feedback 2026-09-23: recorded meetings get All Meetings' Delete meeting — and
  // still no Hide, which for a recording would be deleting it.
  it('offers Delete meeting, not Hide, for a recorded meeting', async () => {
    renderWeek([
      event('Product sync', {
        meetingId: 'meeting-2',
        status: { recorded: true, transcribed: true, summarized: false, speakersIdentified: false },
      }),
    ]);
    openRowMenu();
    expect(await screen.findByText('Delete meeting')).toBeInTheDocument();
    expect(screen.queryByText('Hide from timeline')).not.toBeInTheDocument();
  });

  it('opening the menu does not also route the row', async () => {
    const onSelectItem = vi.fn();
    renderWeek([event('Lunch')], { onSelectItem });

    openRowMenu();
    fireEvent.click(await screen.findByText('Hide from timeline'));
    expect(onSelectItem).not.toHaveBeenCalled();
  });

  it('the row itself still routes on click and on Enter', async () => {
    const onSelectItem = vi.fn();
    const item = event('Lunch');
    renderWeek([item], { onSelectItem });

    const row = screen.getByRole('button', { name: /Lunch/ });
    fireEvent.click(row);
    expect(onSelectItem).toHaveBeenCalledWith(item);

    // The row is a native <button> now (ListRow), so Enter activates it the way the
    // browser does — through a real key press, not a synthetic keydown.
    onSelectItem.mockClear();
    row.focus();
    await userEvent.keyboard('{Enter}');
    expect(onSelectItem).toHaveBeenCalledWith(item);
  });
});

describe('WeekView row parity with All Meetings', () => {
  it('shows the attendee summary', () => {
    renderWeek([
      event('Standup', {
        attendeeCount: 3,
        attendees: [
          { name: 'Maya Okafor', email: 'maya@example.com', isCurrentUser: false },
          { name: 'Tomas Lindqvist', email: 'tomas@example.com', isCurrentUser: false },
          { name: 'Me', email: 'me@example.com', isCurrentUser: true },
        ],
      }),
    ]);
    // Owner excluded display-only (specs/0038 WS8.b): 3 - 1 = 2, one named plus "+1".
    expect(screen.getByText('Maya Okafor, +1')).toBeInTheDocument();
  });

  it('marks the meeting being recorded as recording', () => {
    const item = event('Product sync', { id: 'live-1' });
    renderWeek([item], {
      ctx: { ...ctx, isRecording: true, recordingThisId: 'live-1' },
    });
    expect(screen.getByText('Recording')).toBeInTheDocument();
    // Once, not twice: the reels badge IS the state label (owner feedback 2026-09-23 —
    // the row carried the badge and the older "Recording…" lamp label side by side).
    expect(screen.getAllByText(/^Recording/)).toHaveLength(1);
  });

  it('leaves other rows unmarked while one is recording', () => {
    renderWeek([event('Other', { id: 'other-1' })], {
      ctx: { ...ctx, isRecording: true, recordingThisId: 'live-1' },
    });
    expect(screen.queryByText('Recording')).not.toBeInTheDocument();
  });

  it('filters out rows the user has hidden', () => {
    renderWeek([event('Lunch', { dismissed: true }), event('Standup')]);
    expect(screen.queryByText('Lunch')).not.toBeInTheDocument();
    expect(screen.getByText('Standup')).toBeInTheDocument();
    // The day header's count follows the filtered rows, not the raw list.
    expect(screen.getByText('1 meeting')).toBeInTheDocument();
  });
});

// Design-system audit 2026-11-11 (P1): Thursday listed 2:00 PM, then 9:15, then 11:00 —
// the view rendered the backend's order. Rows within a day read top to bottom by start time.
describe('WeekView row order', () => {
  it('sorts each day by start time, whatever order the items arrive in', () => {
    renderWeek([
      event('Afternoon review', { id: 'c', startTime: '2026-09-16T14:00:00', endTime: '2026-09-16T15:00:00' }),
      event('Early standup', { id: 'a', startTime: '2026-09-16T09:15:00', endTime: '2026-09-16T09:30:00' }),
      event('Late morning sync', { id: 'b', startTime: '2026-09-16T11:00:00', endTime: '2026-09-16T11:30:00' }),
    ]);
    const titles = screen
      .getAllByText(/^(Afternoon review|Early standup|Late morning sync)$/)
      .map((el) => el.textContent);
    expect(titles).toEqual(['Early standup', 'Late morning sync', 'Afternoon review']);
  });

  it('keeps an item with an unparseable start time last rather than throwing', () => {
    renderWeek([
      event('No time', { id: 'x', startTime: 'not-a-date' }),
      event('Morning', { id: 'y', startTime: '2026-09-16T08:00:00' }),
    ]);
    const titles = screen.getAllByText(/^(No time|Morning)$/).map((el) => el.textContent);
    expect(titles).toEqual(['Morning', 'No time']);
  });
});
