import { describe, it, expect } from 'vitest';
import { formatMeetingDate } from '@/lib/format-date';

// Shared meeting-date label (canonical command-palette behavior): hide the year
// for the current year, show it otherwise. Assertions avoid locale-specific
// month names by checking only the year component.

describe('formatMeetingDate', () => {
  it('hides the year for current-year dates', () => {
    const year = new Date().getFullYear();
    const label = formatMeetingDate(`${year}-06-15T12:00:00Z`);
    expect(label).toBeTruthy();
    expect(label).not.toContain(String(year));
  });

  it('shows the year for other years', () => {
    expect(formatMeetingDate('2024-06-15T12:00:00Z')).toContain('2024');
  });

  it('returns null for absent or unparseable input', () => {
    expect(formatMeetingDate(null)).toBeNull();
    expect(formatMeetingDate(undefined)).toBeNull();
    expect(formatMeetingDate('not-a-date')).toBeNull();
  });
});
