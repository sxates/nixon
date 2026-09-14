'use client';

/**
 * Overlapping attendee avatars, used by the All Meetings list (`app/meetings/page.tsx`)
 * — specs/0038 WS8.a. Originally extracted from the (since-retired) Day Agenda component
 * so every surface renders identical avatar chips instead of duplicating markup.
 *
 * WS8.b: the device owner ("You") is excluded DISPLAY-ONLY before rendering, so a 1:1
 * shows a single face. Renders nothing when no non-owner attendees remain.
 *
 * WS3 (specs/0038): when an attendee carries a cached directory `photoDataUri` (a
 * self-contained `data:` image, Google same-org only), render the photo in place of the
 * initials — same size/shape/ring. Per-attendee and best-effort: absent or a broken image
 * (`onError`) falls back to the initials chip. No network — the URI is local base64.
 */

import { useState } from 'react';
import type { AgendaAttendee } from '@/lib/day-agenda';
import { initials, attendeeLabel } from '@/lib/day-agenda';
import { speakerBgClass } from '@/lib/speaker-colors';
import { visibleAttendees } from '@/lib/attendees';

/** One avatar: directory photo when present + loadable, else the colored initials chip. */
function AttendeeAvatar({
  attendee,
  dim,
  overlap,
}: {
  attendee: AgendaAttendee;
  dim: string;
  overlap: boolean;
}) {
  // Track a failed photo load so we degrade to initials without flashing a broken image.
  const [photoFailed, setPhotoFailed] = useState(false);
  const label = attendeeLabel(attendee);
  const colorKey = attendee.email || label;
  const showPhoto = !!attendee.photoDataUri && !photoFailed;
  const base = `overflow-hidden rounded-full border-2 border-card ${dim} ${overlap ? '-ml-[7px]' : ''}`;

  if (showPhoto) {
    return (
      // A self-contained base64 `data:` URI — next/image can't optimize it and there's no
      // image server in the Tauri shell, so a plain <img> is correct here.
      // eslint-disable-next-line @next/next/no-img-element
      <img
        src={attendee.photoDataUri as string}
        alt=""
        title={attendee.email ? `${label} · ${attendee.email}` : label}
        onError={() => setPhotoFailed(true)}
        className={`${base} object-cover`}
      />
    );
  }
  // `bg-muted` is the null-key fallback and is a *light* chip: its ink must be the
  // foreground, not the background (which would be invisible on it).
  const bgClass = speakerBgClass(colorKey);
  const inkClass = bgClass === 'bg-muted' ? 'text-foreground' : 'text-background';
  return (
    <span
      title={attendee.email ? `${label} · ${attendee.email}` : label}
      className={`flex items-center justify-center font-semibold ${inkClass} ${base} ${bgClass}`}
    >
      {initials(label)}
    </span>
  );
}

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
  const shown = visibleAttendees(attendees).slice(0, max);
  if (shown.length === 0) return null;
  const dim = size === 'sm' ? 'h-[22px] w-[22px] text-[9px]' : 'h-6 w-6 text-[10px]';
  return (
    <div className="flex items-center">
      {shown.map((a, i) => (
        <AttendeeAvatar
          key={`${a.email ?? attendeeLabel(a)}-${i}`}
          attendee={a}
          dim={dim}
          overlap={i > 0}
        />
      ))}
    </div>
  );
}
