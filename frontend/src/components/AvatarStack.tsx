'use client';

/**
 * Attendee avatars for the meetings list, the agenda and the participants summary
 * (specs/0038 WS8.a). A thin adapter over `@/components/ui/avatar`'s `AvatarStack`
 * (design-system wave 0b), which now owns the drawing and the overlap.
 *
 * WS8.b: the device owner ("You") is excluded DISPLAY-ONLY, so a 1:1 shows a single face.
 * WS3: a cached directory `photoDataUri` replaces the initials, best-effort per attendee.
 */

import type { AgendaAttendee } from '@/lib/day-agenda';
import { attendeeLabel } from '@/lib/day-agenda';
import { visibleAttendees } from '@/lib/attendees';
import { AvatarStack as UiAvatarStack } from '@/components/ui/avatar';

export function AvatarStack({
  attendees,
  size = 'md',
  max = 3,
}: {
  attendees: AgendaAttendee[];
  size?: 'sm' | 'md';
  /** How many avatars to show before stopping (the "+N" text is rendered by callers). */
  max?: number;
}) {
  const people = visibleAttendees(attendees).map((a) => {
    const label = attendeeLabel(a);
    return {
      name: label,
      photo: a.photoDataUri,
      colorKey: a.email || label,
      title: a.email ? `${label} · ${a.email}` : label,
    };
  });
  return <UiAvatarStack people={people} size={size === 'sm' ? 22 : 24} max={max} />;
}
