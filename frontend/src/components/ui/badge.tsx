/**
 * Badge (docs/design-system.md §2) — the status pill: "Default", "Active", "Recommended",
 * a tier, a count. 11px (`text-caption`) semibold on a `rounded-sm` (2px) chip, filled
 * with the named washes rather than ad-hoc opacities.
 *
 * Tones: `brand` (active, default, selected), `success` (granted, ready), `muted`
 * (neutral facts), `destructive` (failed), `outline` (a hairline chip with no fill, for a
 * tier or a quiet tag).
 *
 * Domain chips (`ProcessingBadge`, `RecordingBadge`, `StateChip`) are composed separately.
 */

import * as React from 'react';
import { cn } from '@/lib/utils';

export const BADGE_TONES = {
  brand: 'bg-brand-wash text-brand',
  success: 'bg-success-wash text-success',
  muted: 'bg-muted text-muted-foreground',
  destructive: 'bg-destructive-wash text-destructive ring-1 ring-inset ring-destructive-edge',
  outline: 'border border-border bg-transparent text-muted-foreground',
} as const;

export type BadgeTone = keyof typeof BADGE_TONES;

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: BadgeTone;
}

export const Badge = React.forwardRef<HTMLSpanElement, BadgeProps>(function Badge(
  { tone = 'muted', className, ...props },
  ref,
) {
  return (
    <span
      ref={ref}
      data-tone={tone}
      className={cn(
        'inline-flex flex-none items-center gap-1 whitespace-nowrap rounded-sm px-2 py-0.5 text-caption font-semibold tabular-nums',
        BADGE_TONES[tone],
        className,
      )}
      {...props}
    />
  );
});
