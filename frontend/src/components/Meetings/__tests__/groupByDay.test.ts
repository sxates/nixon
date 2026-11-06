import { describe, it, expect } from 'vitest';
import { groupByDay } from '@/components/Meetings/MeetingsList';

// The List view's day cards: days newest first, and within a day the meetings in the order
// they happened (as Agenda and Week show them) — whatever order the input arrives in.

const at = (id: string, day: number, hour: number, minute = 0) => ({
  id,
  title: id,
  createdAt: new Date(2026, 10, day, hour, minute).toISOString(), // local, November 2026
});

describe('groupByDay', () => {
  const now = new Date(2026, 10, 5, 18, 0);

  it('orders days newest first even when the input is not', () => {
    const groups = groupByDay(
      [at('nov5-a', 5, 14), at('nov3', 3, 10), at('nov2', 2, 16), at('nov4', 4, 11), at('nov5-b', 5, 9)],
      now,
    );
    expect(groups.map((g) => g.dateLabel)).toEqual(['Nov 5', 'Nov 4', 'Nov 3', 'Nov 2']);
    expect(groups.map((g) => g.label)).toEqual(['Today', 'Yesterday', 'Tuesday', 'Monday']);
  });

  it('orders the meetings within a day by start time, earliest first', () => {
    const groups = groupByDay(
      [at('2pm', 5, 14), at('9:15am', 5, 9, 15), at('11am', 5, 11)],
      now,
    );
    expect(groups[0].items.map((m) => m.id)).toEqual(['9:15am', '11am', '2pm']);
  });

  it('keeps undated rows in a trailing group', () => {
    const groups = groupByDay(
      [{ id: 'x', title: 'x', createdAt: 'not a date' }, at('nov4', 4, 11)],
      now,
    );
    expect(groups.map((g) => g.key)).toEqual(['2026-11-04', 'undated']);
  });
});
