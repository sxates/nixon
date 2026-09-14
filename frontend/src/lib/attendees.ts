/**
 * Shared attendee-display helpers (specs/0038 WS8), used by both the Day Agenda and the
 * All Meetings list so avatars, names, and counts stay consistent across surfaces.
 *
 * WS8.b — owner exclusion is DISPLAY-ONLY. The device owner ("You", `isCurrentUser`) is
 * filtered out of what we RENDER (avatars + the "N attendees" count), so a 1:1 (me + one
 * other) reads as the single other person. The owner is never removed from the underlying
 * roster/data — these helpers only shape the presentation.
 */

import type { AgendaAttendee } from '@/lib/day-agenda';
import { attendeeLabel } from '@/lib/day-agenda';

/** The rendered attendees: the roster's OTHER people (owner filtered out). */
export function visibleAttendees(attendees: AgendaAttendee[]): AgendaAttendee[] {
  return attendees.filter((a) => !a.isCurrentUser);
}

/**
 * Owner-excluded display count for a bounded attendee preview. `totalCount` is the
 * owner-INCLUSIVE roster size from the backend (Day Agenda `attendeeCount` /
 * meetings-list `attendeeCount`); we subtract any owner visible in the previewed
 * `attendees`. The meetings-list roster is owner-excluded at seed time, so usually the
 * owner isn't present at all and this is just `totalCount`. Never negative.
 */
export function visibleAttendeeCount(attendees: AgendaAttendee[], totalCount: number): number {
  const ownerInPreview = attendees.filter((a) => a.isCurrentUser).length;
  return Math.max(0, totalCount - ownerInPreview);
}

/**
 * "Sarah Chen, +6"-style summary from a bounded preview: names up to `maxNames` non-owner
 * attendees, then "+N" for the remaining (owner-excluded) roster. Returns `null` when there
 * is no one to show (empty roster, or every attendee is the owner).
 */
export function attendeeSummaryLabel(
  attendees: AgendaAttendee[],
  totalCount: number,
  maxNames = 2,
): string | null {
  const others = visibleAttendees(attendees);
  const total = visibleAttendeeCount(attendees, totalCount);
  if (total === 0 || others.length === 0) return null;
  const named = others.slice(0, maxNames).map((a) => attendeeLabel(a));
  const remaining = total - named.length;
  return remaining > 0 ? `${named.join(', ')}, +${remaining}` : named.join(', ');
}
