/**
 * Client-side People directory filter (specs/0019 WS3.3 / note 6). Matches a query
 * (case-insensitive, trimmed) against a person's display name, role, or email so the
 * directory stays usable as the number of tracked people grows. Pure and structurally
 * typed so it's trivially unit-testable and reusable by the participant/speaker pickers.
 */
export interface FilterablePerson {
  displayName?: string | null;
  role?: string | null;
  email?: string | null;
}

export function filterPeople<T extends FilterablePerson>(people: T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return people;
  return people.filter((p) =>
    [p.displayName, p.role, p.email].some(
      (field) => !!field && field.toLowerCase().includes(q),
    ),
  );
}
