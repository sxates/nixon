import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// specs/0069 W4 — the List presentation of Today's agenda (default when no calendar is
// connected). It reuses the SAME click-routing, StateChip, and Join/Record affordances
// as the day-grid `TimelineBlock` — this is a third presentation of the same rows, not
// a second behaviour.

import { DayList } from '@/components/Today/DayList';
import type { DayAgendaItem } from '@/lib/day-agenda';
import type { TimelineContext } from '@/lib/today-timeline';

const NOW = new Date('2024-01-01T12:00:00.000Z');

function at(time: string, title: string, overrides: Partial<DayAgendaItem> = {}): DayAgendaItem {
  return {
    id: `${title}-${time}`,
    title,
    startTime: `2024-01-01T${time}:00.000Z`,
    endTime: null,
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

const ctx: TimelineContext = { now: NOW, isRecording: false, recordingThisId: null };

const base = {
  ctx,
  now: NOW,
  onSelect: vi.fn(),
  onJoin: vi.fn(),
  onRecord: vi.fn(),
  onHide: vi.fn(),
  onEdit: vi.fn(),
  onDelete: vi.fn(),
  onAddMeeting: vi.fn(),
};

describe('DayList (specs/0069 W4)', () => {
  it('lists the day in time order with a state chip each', () => {
    render(<DayList {...base} items={[at('15:00', 'Call with Sam'), at('09:30', 'Standup')]} />);
    const rows = screen.getAllByRole('button', { name: /standup|call with sam/i });
    expect(rows[0]).toHaveTextContent('Standup');
    expect(rows[1]).toHaveTextContent('Call with Sam');
  });

  it('offers a way out of an empty day', () => {
    const onAddMeeting = vi.fn();
    render(<DayList {...base} items={[]} onAddMeeting={onAddMeeting} />);
    expect(screen.getByText(/nothing today/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /add meeting/i }));
    expect(onAddMeeting).toHaveBeenCalled();
  });

  it('routes a click the same way the grid does', () => {
    const onSelect = vi.fn();
    const item = at('15:00', 'Call with Sam');
    render(<DayList {...base} items={[item]} onSelect={onSelect} />);
    fireEvent.click(screen.getByRole('button', { name: /call with sam/i }));
    expect(onSelect).toHaveBeenCalledWith(item);
  });

  it('offers Record (not the chip) for a joinable/happening-now manual entry', () => {
    const onRecord = vi.fn();
    const manual = at('12:00', 'Ad-hoc sync', {
      source: 'manual',
      meetingId: 'meeting-1',
      calendarEventId: 'nixon-manual:meeting-1',
      endTime: '2024-01-01T12:30:00.000Z',
    });
    render(<DayList {...base} items={[manual]} onRecord={onRecord} />);
    fireEvent.click(screen.getByRole('button', { name: 'Record' }));
    expect(onRecord).toHaveBeenCalledWith(manual);
  });

  // fix round 1 Finding 1 — List is the DEFAULT view for no-calendar users, exactly the
  // people most likely to want an unwanted calendar row off their day, so this menu item
  // (and the ⋯ trigger itself) can't be missing here the way it briefly was.
  describe('Hide from timeline (fix round 1 Finding 1)', () => {
    it('offers Hide for an unrecorded calendar event and calls onHide', async () => {
      const onHide = vi.fn();
      const item = at('15:00', 'Standalone sync'); // calendar, unrecorded, no meetingId
      render(<DayList {...base} items={[item]} onHide={onHide} />);

      const trigger = screen.getByRole('button', { name: 'Event options' });
      trigger.focus();
      fireEvent.keyDown(trigger, { key: 'Enter' });
      fireEvent.click(await screen.findByText('Hide from timeline'));
      expect(onHide).toHaveBeenCalledWith(item);
    });

    it('renders no ⋯ menu at all once that calendar event is recorded', () => {
      const item = at('09:00', 'Standup', {
        meetingId: 'meeting-1',
        status: { recorded: true, transcribed: true, summarized: false, speakersIdentified: false },
      });
      render(<DayList {...base} items={[item]} />);
      expect(screen.queryByRole('button', { name: 'Event options' })).not.toBeInTheDocument();
    });
  });

  // fix round 1 Finding 2 — field-by-field parity pass against `TimelineBlock`: the
  // attendee count was surfaced there and silently dropped here.
  //
  // Owner feedback 2026-09-21 turned the bare count into the avatar cluster All Meetings
  // uses. A count with no preview rows to draw faces from (a cache entry predating
  // `DayAgendaItem.attendees`) still falls back to the plain text, so nothing is lost.
  it('falls back to the plain attendee count when there are no preview rows', () => {
    const item = at('09:00', 'Standup', { attendeeCount: 3 });
    render(<DayList {...base} items={[item]} />);
    expect(screen.getByText(/3 attendees/)).toBeInTheDocument();
  });

  it('shows faces and a name summary when the roster has preview rows', () => {
    const item = at('09:00', 'Standup', {
      attendeeCount: 3,
      attendees: [
        { name: 'Maya Okafor', email: 'maya@example.com', isCurrentUser: false },
        { name: 'Tomas Lindqvist', email: 'tomas@example.com', isCurrentUser: false },
        { name: 'Me', email: 'me@example.com', isCurrentUser: true },
      ],
    });
    render(<DayList {...base} items={[item]} />);
    // Owner excluded from the display (specs/0038 WS8.b), so 3 - 1 = 2 people: one named
    // plus "+1".
    expect(screen.getByText('Maya Okafor, +1')).toBeInTheDocument();
    expect(screen.queryByText(/attendees/)).toBeNull();
  });
});
