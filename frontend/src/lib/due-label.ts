/**
 * Friendly due-date labels for to-do items (specs/0088): "Today", "Tomorrow",
 * "Wednesday", "Next Wednesday", "Nov 28". All arithmetic is on LOCAL calendar dates —
 * a `YYYY-MM-DD` string is parsed from its parts, never via `new Date(iso)` (UTC).
 */

const WEEKDAY_NAMES = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const MONTH_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const MONTH_LONG = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
];

export interface FriendlyDue {
  /** Short label for the row: "Today", "Next Wednesday", "Nov 28". */
  label: string;
  /** Long form for a tooltip: "Wednesday, November 18, 2026". */
  full: string;
  /** Before today. */
  overdue: boolean;
  /** Today or overdue — the amber "urgent" state. */
  dueSoon: boolean;
}

/** `YYYY-MM-DD` (optionally followed by a time) as a local date, or null. */
export function parseLocalDate(iso: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso.trim());
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const date = new Date(y, mo - 1, d);
  if (date.getFullYear() !== y || date.getMonth() !== mo - 1 || date.getDate() !== d) return null;
  return date;
}

/** A local date as `YYYY-MM-DD` (never via UTC). */
export function toLocalIso(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Whole calendar days from `a` to `b` (local dates; DST-proof). */
export function calendarDaysBetween(a: Date, b: Date): number {
  const ua = Date.UTC(a.getFullYear(), a.getMonth(), a.getDate());
  const ub = Date.UTC(b.getFullYear(), b.getMonth(), b.getDate());
  return Math.round((ub - ua) / 86_400_000);
}

function shortDate(d: Date, today: Date): string {
  const base = `${MONTH_SHORT[d.getMonth()]} ${d.getDate()}`;
  return d.getFullYear() === today.getFullYear() ? base : `${base}, ${d.getFullYear()}`;
}

/** The long form: "Wednesday, November 18, 2026". */
export function fullDate(d: Date): string {
  return `${WEEKDAY_NAMES[d.getDay()]}, ${MONTH_LONG[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()}`;
}

/**
 * The friendly label for a due date relative to `today`. An unparseable string comes
 * back verbatim (never urgent).
 */
export function friendlyDue(iso: string, today: Date): FriendlyDue {
  const d = parseLocalDate(iso);
  if (!d) return { label: iso, full: iso, overdue: false, dueSoon: false };
  const days = calendarDaysBetween(today, d);
  const weekday = WEEKDAY_NAMES[d.getDay()];
  let label: string;
  if (days === -1) label = 'Yesterday';
  else if (days === 0) label = 'Today';
  else if (days === 1) label = 'Tomorrow';
  else if (days >= 2 && days <= 6) label = weekday;
  else if (days >= 7 && days <= 13) label = `Next ${weekday}`;
  else label = shortDate(d, today);
  return { label, full: fullDate(d), overdue: days < 0, dueSoon: days <= 0 };
}
