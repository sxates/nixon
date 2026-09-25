'use client';

import { useState, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** Initials for the avatar fallback, e.g. "Priya Sharma" → "PS". */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '?';
  if (parts.length === 1) return parts[0]!.charAt(0).toUpperCase();
  return (parts[0]!.charAt(0) + parts[parts.length - 1]!.charAt(0)).toUpperCase();
}

const SIZE_CLASSES = {
  /** A speaker-legend name cell (replaces the 10px colour dot when a photo exists). */
  xxs: 'h-4 w-4 text-[8px]',
  /** A transcript run header. */
  xs: 'h-7 w-7 text-[11px]',
  /** The People directory row. */
  sm: 'h-8 w-8 text-xs',
  /** The person page header. */
  lg: 'h-14 w-14 text-lg',
} as const;

interface PersonAvatarProps {
  /** Display name — the initials source. */
  name: string;
  /** Cached directory photo (`Person.photoDataUri`, a self-contained `data:` URI). */
  photoDataUri?: string | null;
  size: keyof typeof SIZE_CLASSES;
  /** Background palette class for the initials chip (e.g. `bg-chart-4`). */
  colorClass: string;
  /** Rendered instead of the initials chip when there is no photo or it fails to load
   *  (the speaker legend's 10px colour dot). Absent => initials. */
  fallback?: ReactNode;
}

/**
 * One person's avatar (specs/0056 W6): the cached Google directory photo when the person
 * carries a `photoDataUri`, else the colored initials chip. Same pattern as `AvatarStack`'s
 * attendee avatar — a failed image load (`onError`) degrades to initials without flashing a
 * broken image. No network: the URI is local base64 read through `attendee_photos`.
 * Also the transcript's speaker avatar and the speaker legend (via `lib/speaker-photos`).
 */
export function PersonAvatar({ name, photoDataUri, size, colorClass, fallback }: PersonAvatarProps) {
  // Remember WHICH uri failed, not just that one did: a row that later gets a different
  // photo (a speaker reassigned to someone else, a refreshed directory photo) tries again.
  const [failedUri, setFailedUri] = useState<string | null>(null);
  const photoFailed = !!photoDataUri && failedUri === photoDataUri;
  const base = cn('flex-shrink-0 rounded-full', SIZE_CLASSES[size]);
  // `bg-muted` is `speakerBgClass`'s null-key fallback and is a *light* chip: its ink must
  // be the foreground, not the background (which would be invisible on it).
  const inkClass = colorClass === 'bg-muted' ? 'text-foreground' : 'text-background';

  if (photoDataUri && !photoFailed) {
    return (
      // A self-contained base64 `data:` URI — next/image can't optimize it and there's no
      // image server in the Tauri shell, so a plain <img> is correct here.
      // eslint-disable-next-line @next/next/no-img-element
      <img
        src={photoDataUri}
        alt=""
        onError={() => setFailedUri(photoDataUri)}
        className={cn(base, 'object-cover')}
      />
    );
  }
  if (fallback !== undefined) return <>{fallback}</>;
  return (
    <span
      className={cn(
        base,
        'flex items-center justify-center font-semibold',
        inkClass,
        colorClass,
      )}
      aria-hidden="true"
    >
      {initials(name)}
    </span>
  );
}
