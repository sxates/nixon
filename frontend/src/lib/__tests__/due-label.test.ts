import { describe, it, expect } from 'vitest';
import { calendarDaysBetween, friendlyDue, parseLocalDate, toLocalIso } from '../due-label';

// 2026-11-12 is a Thursday.
const TODAY = new Date(2026, 10, 12);

describe('friendlyDue', () => {
  const cases: [string, string, boolean, boolean][] = [
    // iso, label, overdue, dueSoon
    ['2026-11-01', 'Nov 1', true, true],
    ['2026-11-10', 'Nov 10', true, true],
    ['2026-11-11', 'Yesterday', true, true],
    ['2026-11-12', 'Today', false, true],
    ['2026-11-13', 'Tomorrow', false, false],
    ['2026-11-14', 'Saturday', false, false],
    ['2026-11-18', 'Wednesday', false, false],
    ['2026-11-19', 'Next Thursday', false, false],
    ['2026-11-25', 'Next Wednesday', false, false],
    ['2026-11-26', 'Nov 26', false, false],
    ['2026-11-28', 'Nov 28', false, false],
    ['2027-01-05', 'Jan 5, 2027', false, false],
    ['2025-11-10', 'Nov 10, 2025', true, true],
  ];
  for (const [iso, label, overdue, dueSoon] of cases) {
    it(`${iso} → ${label}`, () => {
      const r = friendlyDue(iso, TODAY);
      expect(r.label).toBe(label);
      expect(r.overdue).toBe(overdue);
      expect(r.dueSoon).toBe(dueSoon);
    });
  }

  it('full is the long form', () => {
    expect(friendlyDue('2026-11-18', TODAY).full).toBe('Wednesday, November 18, 2026');
  });

  it('ignores the time of day on today', () => {
    expect(friendlyDue('2026-11-13', new Date(2026, 10, 12, 23, 59)).label).toBe('Tomorrow');
    expect(friendlyDue('2026-11-12', new Date(2026, 10, 12, 0, 1)).label).toBe('Today');
  });

  it('accepts a timestamp suffix and leaves junk verbatim', () => {
    expect(friendlyDue('2026-11-13T00:00:00', TODAY).label).toBe('Tomorrow');
    expect(friendlyDue('soon', TODAY)).toEqual({
      label: 'soon',
      full: 'soon',
      overdue: false,
      dueSoon: false,
    });
  });
});

describe('local-date helpers', () => {
  it('parseLocalDate rejects impossible dates', () => {
    expect(parseLocalDate('2026-02-30')).toBeNull();
    expect(parseLocalDate('2026-11-12')?.getDate()).toBe(12);
  });

  it('toLocalIso round-trips', () => {
    expect(toLocalIso(new Date(2027, 0, 5))).toBe('2027-01-05');
  });

  it('calendarDaysBetween counts across a DST change', () => {
    expect(calendarDaysBetween(new Date(2026, 2, 7), new Date(2026, 2, 9))).toBe(2);
    expect(calendarDaysBetween(new Date(2026, 10, 12), new Date(2026, 10, 1))).toBe(-11);
  });
});
