'use client';

/**
 * Shim — the avatar moved to `@/components/ui/avatar` (design-system wave 0b). This keeps the
 * old name-based size API (`xxs`/`xs`/`sm`/`lg`) and `photoDataUri` prop working until the
 * callers migrate; new code uses `Avatar` directly.
 */

import type { ReactNode } from 'react';
import { Avatar, type AvatarSize } from '@/components/ui/avatar';

export { initials } from '@/components/ui/avatar';

const SIZE_MAP = { xxs: 16, xs: 28, sm: 32, lg: 56 } as const satisfies Record<string, AvatarSize>;

interface PersonAvatarProps {
  name: string;
  photoDataUri?: string | null;
  size: keyof typeof SIZE_MAP;
  colorClass: string;
  fallback?: ReactNode;
}

export function PersonAvatar({ name, photoDataUri, size, colorClass, fallback }: PersonAvatarProps) {
  return (
    <Avatar
      name={name}
      photo={photoDataUri}
      size={SIZE_MAP[size]}
      colorClass={colorClass}
      fallback={fallback}
    />
  );
}
