'use client';

/**
 * Avatar + AvatarStack (docs/design-system.md §2) — a person as a photo or initials.
 *
 * `Avatar` draws the cached directory photo when `photo` is a loadable `data:` URI, else the
 * coloured initials chip. A failed image load (`onError`) degrades to initials without
 * flashing a broken image, and it remembers WHICH uri failed so a later, different photo
 * (a speaker reassigned, a refreshed directory photo) is tried again. No network: the URI is
 * local base64 read through `attendee_photos`.
 *
 * `AvatarStack` overlaps several avatars. Each disc carries a 2px card-coloured border as
 * the separator, and the overlap is 4px — inside the previous disc's border plus 2px of its
 * face — so a neighbour never covers the initials. (The 7px overlap it replaces clipped the
 * second letter on 22px discs.)
 *
 * Sizes are pixel names, like `StatusDot`: 16 (legend cell), 22 / 24 (stacks), 28
 * (transcript run header), 32 (directory row), 56 (person page header).
 *
 * This file replaces `components/People/PersonAvatar.tsx` and `components/AvatarStack.tsx`,
 * which are now shims over it.
 */

import { useState, type ReactNode } from 'react';
import { speakerBgClass } from '@/lib/speaker-colors';
import { initials } from '@/lib/initials';
import { cn } from '@/lib/utils';

export { initials };

const AVATAR_SIZE_CLASSES = {
  16: 'h-4 w-4 text-micro',
  22: 'h-[22px] w-[22px] text-micro',
  24: 'h-6 w-6 text-label',
  28: 'h-7 w-7 text-caption',
  32: 'h-8 w-8 text-xs',
  56: 'h-14 w-14 text-heading',
} as const;

export type AvatarSize = keyof typeof AVATAR_SIZE_CLASSES;
export const AVATAR_SIZES = Object.keys(AVATAR_SIZE_CLASSES).map(Number) as AvatarSize[];

export interface AvatarProps {
  /** Display name — the initials source (an email works too: its local part is used). */
  name: string;
  /** Cached directory photo, a self-contained `data:` URI. */
  photo?: string | null;
  size?: AvatarSize;
  /**
   * Background palette class for the initials chip (e.g. `bg-chart-4`). Defaults to the
   * speaker colour hashed from `colorKey ?? name`.
   */
  colorClass?: string;
  /** Key for the default colour when it should not be the name (an email, a speaker key). */
  colorKey?: string | null;
  /**
   * Rendered instead of the initials chip when there is no photo or it fails to load
   * (the speaker legend's colour dot), centred inside the avatar's size box so it takes
   * the same slot as a photo. Absent → initials.
   */
  fallback?: ReactNode;
  /** Native tooltip (the stack uses "Name · email"). */
  title?: string;
  className?: string;
}

/** One person: photo when present and loadable, else the coloured initials chip. */
export function Avatar({
  name,
  photo,
  size = 32,
  colorClass,
  colorKey,
  fallback,
  title,
  className,
}: AvatarProps) {
  const [failedUri, setFailedUri] = useState<string | null>(null);
  const photoFailed = !!photo && failedUri === photo;
  const bg = colorClass ?? speakerBgClass(colorKey ?? name);
  const base = cn('flex-none rounded-full', AVATAR_SIZE_CLASSES[size], className);

  if (photo && !photoFailed) {
    return (
      // A self-contained base64 `data:` URI — next/image can't optimize it and there's no
      // image server in the Tauri shell, so a plain <img> is correct here.
      // eslint-disable-next-line @next/next/no-img-element
      <img
        src={photo}
        alt=""
        title={title}
        data-size={size}
        onError={() => setFailedUri(photo)}
        className={cn(base, 'object-cover')}
      />
    );
  }
  // The fallback sits in the avatar's own size box, centred — so a 10px speaker dot in a
  // 16px slot keeps the name beside it on the same x as a row that has a photo.
  if (fallback !== undefined) {
    return (
      <span
        data-size={size}
        data-avatar-fallback=""
        title={title}
        className={cn('inline-flex flex-none items-center justify-center', AVATAR_SIZE_CLASSES[size], className)}
      >
        {fallback}
      </span>
    );
  }
  // `bg-muted` is `speakerBgClass`'s null-key fallback and is a *light* chip: its ink must
  // be the foreground, not the background (which would be invisible on it).
  const ink = bg === 'bg-muted' ? 'text-foreground' : 'text-background';
  return (
    <span
      title={title}
      data-size={size}
      aria-hidden="true"
      className={cn(
        'flex items-center justify-center overflow-hidden font-semibold leading-none',
        ink,
        bg,
        base,
      )}
    >
      {initials(name)}
    </span>
  );
}

export interface AvatarStackPerson {
  name: string;
  photo?: string | null;
  /** Default-colour key (the email, so a person keeps one colour across surfaces). */
  colorKey?: string | null;
  title?: string;
}

export interface AvatarStackProps {
  people: AvatarStackPerson[];
  /** 22 (dense rows) or 24 (default). */
  size?: 22 | 24;
  /** How many to draw. The "+N" text, if any, is the caller's. */
  max?: number;
  className?: string;
}

/** Overlapping avatars with a card-coloured separator ring. Renders nothing when empty. */
export function AvatarStack({ people, size = 24, max = 3, className }: AvatarStackProps) {
  const shown = people.slice(0, max);
  if (shown.length === 0) return null;
  return (
    <div className={cn('flex items-center', className)} data-avatar-stack="">
      {shown.map((p, i) => (
        <Avatar
          key={`${p.colorKey ?? p.name}-${i}`}
          name={p.name}
          photo={p.photo}
          colorKey={p.colorKey}
          title={p.title}
          size={size}
          className={cn('border-2 border-card', i > 0 && '-ml-1')}
        />
      ))}
    </div>
  );
}
