'use client';

/**
 * The attendee cluster on a Today row: overlapping avatars plus "Maya, +2".
 *
 * Owner feedback 2026-09-21 — "Lots of inconsistencies in how we display meetings between
 * the Today tab and All Meetings." All Meetings has shown faces since specs/0038 WS8.a
 * while Today rendered the bare text "· 3 attendees", even though `DayAgendaItem` has
 * carried `attendees[]` (the first ~5, with cached directory photos) all along. This is the
 * same `AvatarStack` + `attendeeSummaryLabel` pair All Meetings uses, so the two screens
 * agree by construction rather than by two lists of Tailwind classes staying in sync.
 *
 * Renders nothing when there is nobody to show — an empty roster, or a roster that is only
 * the owner (the display-only owner exclusion lives in `lib/attendees`). It never
 * fabricates a count.
 */

import { AvatarStack } from '@/components/AvatarStack';
import {
  attendeeSummaryLabel,
  visibleAttendeeCount,
  visibleAttendees,
} from '@/lib/attendees';
import type { DayAgendaItem } from '@/lib/day-agenda';
import { cn } from '@/lib/utils';

export function AgendaAttendees({
  item,
  /** How many faces before the "+N" takes over. Week rows are narrow, so they pass 2. */
  max = 3,
  /** How many names to spell out before the "+N". */
  maxNames = 1,
  className,
}: {
  item: DayAgendaItem;
  max?: number;
  maxNames?: number;
  className?: string;
}) {
  const attendees = item.attendees ?? [];
  const count = visibleAttendeeCount(attendees, item.attendeeCount ?? 0);
  if (count === 0) return null;

  const label = attendeeSummaryLabel(attendees, item.attendeeCount ?? 0, maxNames);
  // A roster size with no preview rows to draw faces from. `DayAgendaItem.attendees` is
  // documented as the first ~5, so this is the stale-cache case the type warns about (an
  // entry written before the field existed). Keep the information rather than rendering
  // nothing — this is what the row said before it grew avatars.
  if (!label || visibleAttendees(attendees).length === 0) {
    return (
      <span className={cn('truncate text-xs text-muted-foreground', className)}>
        {count} attendee{count === 1 ? '' : 's'}
      </span>
    );
  }

  return (
    <span className={cn('flex min-w-0 items-center gap-1.5', className)}>
      <AvatarStack attendees={attendees} size="sm" max={max} />
      <span className="truncate text-xs text-muted-foreground">{label}</span>
    </span>
  );
}
