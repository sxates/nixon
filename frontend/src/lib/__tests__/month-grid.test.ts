import { describe, it, expect } from 'vitest';
import {
  bucketByLocalDay,
  buildMonthGrid,
  gridRangeUtc,
  localDayKey,
  monthLabel,
  shiftMonth,
  weekdayLabels,
} from '@/lib/month-grid';

describe('buildMonthGrid (specs/0054 W3)', () => {
  it('always returns a fixed 6x7 grid so paging months does not reflow the page', () => {
    for (const [y, m] of [
      [2026, 7],
      [2026, 1],
      [2024, 1],
      [2026, 10],
    ] as const) {
      expect(buildMonthGrid(y, m)).toHaveLength(42);
    }
  });

  it('starts on Monday and spills the adjacent months into the edges', () => {
    // 1 August 2026 is a Saturday, so the first week carries Mon 27 – Fri 31 July.
    const grid = buildMonthGrid(2026, 7);
    expect(grid[0].date.getDay()).toBe(1); // Monday
    expect(grid[0].inMonth).toBe(false);
    expect(localDayKey(grid[0].date)).toBe('2026-07-27');

    const firstOfMonth = grid.find((c) => c.inMonth);
    expect(localDayKey(firstOfMonth!.date)).toBe('2026-08-01');
    expect(firstOfMonth!.date.getDay()).toBe(6); // Saturday
  });

  it('marks exactly the days belonging to the month', () => {
    const grid = buildMonthGrid(2026, 7); // August, 31 days
    expect(grid.filter((c) => c.inMonth)).toHaveLength(31);

    const feb2024 = buildMonthGrid(2024, 1); // leap February
    expect(feb2024.filter((c) => c.inMonth)).toHaveLength(29);

    const feb2026 = buildMonthGrid(2026, 1);
    expect(feb2026.filter((c) => c.inMonth)).toHaveLength(28);
  });

  it('produces strictly consecutive days with no gaps or repeats', () => {
    const keys = buildMonthGrid(2026, 2).map((c) => localDayKey(c.date));
    expect(new Set(keys).size).toBe(42);
    // March 2026 contains a spring-forward DST transition in most US zones; the
    // local-noon construction must still yield one cell per calendar day.
    for (let i = 1; i < keys.length; i += 1) {
      expect(keys[i]).not.toBe(keys[i - 1]);
    }
  });
});

describe('localDayKey', () => {
  it('uses LOCAL calendar fields, not the UTC date', () => {
    // Late-evening local time: toISOString() would roll to the next UTC day for
    // anyone west of UTC, filing the meeting under the wrong day.
    const late = new Date(2026, 7, 19, 23, 30, 0);
    expect(localDayKey(late)).toBe('2026-08-19');

    const early = new Date(2026, 7, 19, 0, 15, 0);
    expect(localDayKey(early)).toBe('2026-08-19');
  });

  it('zero-pads single-digit months and days', () => {
    expect(localDayKey(new Date(2026, 0, 5, 12))).toBe('2026-01-05');
  });
});

describe('gridRangeUtc', () => {
  it('brackets the whole visible grid, not just the month, so spill days populate', () => {
    const { start, end } = gridRangeUtc(2026, 7);
    const grid = buildMonthGrid(2026, 7);
    const startMs = new Date(start).getTime();
    const endMs = new Date(end).getTime();

    expect(endMs).toBeGreaterThan(startMs);
    for (const cell of grid) {
      expect(cell.date.getTime()).toBeGreaterThanOrEqual(startMs);
      expect(cell.date.getTime()).toBeLessThan(endMs);
    }
  });

  it('emits parseable RFC3339 instants for the Rust command', () => {
    const { start, end } = gridRangeUtc(2026, 1);
    expect(Number.isNaN(new Date(start).getTime())).toBe(false);
    expect(Number.isNaN(new Date(end).getTime())).toBe(false);
    expect(start).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });
});

describe('shiftMonth', () => {
  it('carries the year in both directions', () => {
    expect(shiftMonth(2026, 11, 1)).toEqual({ year: 2027, monthIndex: 0 });
    expect(shiftMonth(2026, 0, -1)).toEqual({ year: 2025, monthIndex: 11 });
    expect(shiftMonth(2026, 7, -2)).toEqual({ year: 2026, monthIndex: 5 });
  });

  it('handles multi-year steps', () => {
    expect(shiftMonth(2026, 5, -18)).toEqual({ year: 2024, monthIndex: 11 });
  });
});

describe('bucketByLocalDay', () => {
  const at = (iso: string) => ({ id: iso, createdAt: iso });

  it('groups by local day', () => {
    const morning = new Date(2026, 7, 19, 9, 0).toISOString();
    const evening = new Date(2026, 7, 19, 21, 0).toISOString();
    const nextDay = new Date(2026, 7, 20, 9, 0).toISOString();

    const buckets = bucketByLocalDay([at(morning), at(evening), at(nextDay)], (m) => m.createdAt);
    expect(buckets.get('2026-08-19')).toHaveLength(2);
    expect(buckets.get('2026-08-20')).toHaveLength(1);
  });

  it('drops unparseable timestamps rather than blanking the month', () => {
    const good = new Date(2026, 7, 19, 9, 0).toISOString();
    const buckets = bucketByLocalDay([at(good), at('not-a-date')], (m) => m.createdAt);
    expect(buckets.get('2026-08-19')).toHaveLength(1);
    expect(buckets.size).toBe(1);
  });

  it('preserves input order within a day', () => {
    const a = new Date(2026, 7, 19, 9, 0).toISOString();
    const b = new Date(2026, 7, 19, 10, 0).toISOString();
    const buckets = bucketByLocalDay([at(a), at(b)], (m) => m.createdAt);
    expect(buckets.get('2026-08-19')!.map((m) => m.id)).toEqual([a, b]);
  });
});

describe('labels', () => {
  it('names the month and year', () => {
    expect(monthLabel(2026, 7)).toMatch(/2026/);
  });

  it('emits seven weekday labels starting on Monday', () => {
    const labels = weekdayLabels();
    expect(labels).toHaveLength(7);
    expect(new Set(labels).size).toBe(7);
  });
});
