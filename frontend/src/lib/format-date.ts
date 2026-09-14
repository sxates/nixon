/**
 * Shared meeting-date label: "Jun 24" for dates in the current year, "Jun 24, 2025"
 * otherwise (canonical behavior lifted from the command palette so meeting dates read
 * the same everywhere). Returns null for absent/unparseable input so callers can
 * conditionally render.
 */
export function formatMeetingDate(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleDateString([], {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
  });
}

/**
 * Compact label for a structured due date (specs/0038 WS1.a): a date-only ISO string
 * (`YYYY-MM-DD`). Parsed from its parts as a LOCAL date so it never shifts a day across
 * time zones (unlike `new Date("YYYY-MM-DD")`, which is UTC midnight). Same "Jun 24" /
 * "Jun 24, 2025" shape as `formatMeetingDate`; null for absent/unparseable input.
 */
export function formatDueDate(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso.trim());
  if (!match) return null;
  const [, y, m, day] = match;
  const d = new Date(Number(y), Number(m) - 1, Number(day));
  if (Number.isNaN(d.getTime())) return null;
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleDateString([], {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
  });
}
