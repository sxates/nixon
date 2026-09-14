/**
 * Month-grid date math for the meetings calendar (specs/0054 W3).
 *
 * Kept as pure functions, separate from the component, because every bug worth
 * having in a calendar lives here: month boundaries, week alignment, DST, and the
 * local/UTC seam. The backend stores `created_at` as RFC3339 UTC while the grid
 * buckets by the user's LOCAL day, so the conversion has exactly one home.
 *
 * All construction uses local noon rather than local midnight. On a spring-forward
 * DST day, midnight may not exist in the local zone and `new Date(y, m, d)` can
 * land on the previous day; noon is safely inside every real local day.
 */

/** A single cell of the 6×7 grid. */
export interface MonthGridCell {
  date: Date;
  /** False for the leading/trailing days spilled in from adjacent months. */
  inMonth: boolean;
}

/** Local-noon Date for a y/m/d, avoiding the DST-midnight trap described above. */
function localNoon(year: number, monthIndex: number, day: number): Date {
  return new Date(year, monthIndex, day, 12, 0, 0, 0);
}

/**
 * Stable local-day key (`YYYY-MM-DD`). Built from local getters, never
 * `toISOString()` — that converts to UTC and would file an evening meeting under
 * the next day for anyone east of UTC.
 */
export function localDayKey(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

/**
 * A Monday-first 6×7 grid covering `monthIndex` (0-based) of `year`, including the
 * spill days needed to fill the first and last weeks.
 *
 * Always 42 cells: a fixed height stops the grid reflowing as the user pages
 * through months, which is the whole point of a calendar as a browsing surface.
 */
export function buildMonthGrid(year: number, monthIndex: number): MonthGridCell[] {
  const first = localNoon(year, monthIndex, 1);
  // getDay(): 0=Sunday. Monday-first means Monday→0 … Sunday→6.
  const leading = (first.getDay() + 6) % 7;
  const cells: MonthGridCell[] = [];
  for (let i = 0; i < 42; i += 1) {
    const date = localNoon(year, monthIndex, 1 - leading + i);
    cells.push({ date, inMonth: date.getMonth() === monthIndex });
  }
  return cells;
}

/**
 * The UTC instants bracketing a local month, widened to the whole grid so the
 * spill days are populated too. Passed straight to `api_get_meetings_in_range`,
 * whose range is half-open `[start, end)`.
 */
export function gridRangeUtc(year: number, monthIndex: number): { start: string; end: string } {
  const cells = buildMonthGrid(year, monthIndex);
  const firstCell = cells[0].date;
  const lastCell = cells[cells.length - 1].date;
  const start = new Date(firstCell.getFullYear(), firstCell.getMonth(), firstCell.getDate(), 0, 0, 0, 0);
  // Exclusive end: local midnight following the last cell.
  const end = new Date(lastCell.getFullYear(), lastCell.getMonth(), lastCell.getDate() + 1, 0, 0, 0, 0);
  return { start: start.toISOString(), end: end.toISOString() };
}

/** Step `monthIndex` by `delta` months, carrying the year. */
export function shiftMonth(
  year: number,
  monthIndex: number,
  delta: number,
): { year: number; monthIndex: number } {
  const d = localNoon(year, monthIndex + delta, 1);
  return { year: d.getFullYear(), monthIndex: d.getMonth() };
}

/**
 * Bucket items by their local day key. Items whose timestamp does not parse are
 * dropped rather than thrown — one bad row must not blank the whole month.
 */
export function bucketByLocalDay<T>(items: T[], timestampOf: (item: T) => string): Map<string, T[]> {
  const byDay = new Map<string, T[]>();
  for (const item of items) {
    const parsed = new Date(timestampOf(item));
    if (Number.isNaN(parsed.getTime())) continue;
    const key = localDayKey(parsed);
    const bucket = byDay.get(key);
    if (bucket) bucket.push(item);
    else byDay.set(key, [item]);
  }
  return byDay;
}

/** `"August 2026"` in the user's locale, for the grid header. */
export function monthLabel(year: number, monthIndex: number): string {
  return localNoon(year, monthIndex, 1).toLocaleDateString(undefined, {
    month: 'long',
    year: 'numeric',
  });
}

/** Monday-first weekday initials for the grid header, in the user's locale. */
export function weekdayLabels(): string[] {
  // 2026-06-01 is a Monday; any known Monday works as the anchor.
  return Array.from({ length: 7 }, (_, i) =>
    localNoon(2026, 5, 1 + i).toLocaleDateString(undefined, { weekday: 'short' }),
  );
}
