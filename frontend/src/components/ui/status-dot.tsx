/**
 * StatusDot (docs/design-system.md §2) — a lamp, a state dot or a speaker's colour dot.
 * Status is a dot, never a glyph. `size` is the diameter in px: 6, 8 (default) or 10.
 *
 * `tone` picks a lamp colour, each meaning one thing: `brand` (armed, active, selected),
 * `record` (recording only), `success` (granted, done), `destructive` (error), `muted`
 * (idle, off). For a speaker's colour pass the palette class as `className`
 * (`speakerBgClass(key)`) and leave `tone` unset.
 *
 * Decorative by default. Pass `label` when the dot is the only carrier of the state and it
 * becomes `role="img"` with that accessible name.
 *
 * `Transport/LampDot` is the hardware lamp with glow and recess; it stays its own part.
 */

import { cn } from '@/lib/utils';

export const STATUS_DOT_TONES = {
  brand: 'bg-brand',
  record: 'bg-record',
  success: 'bg-success',
  destructive: 'bg-destructive',
  muted: 'bg-engrave',
} as const;

export const STATUS_DOT_SIZES = {
  6: 'h-1.5 w-1.5',
  8: 'h-2 w-2',
  10: 'h-2.5 w-2.5',
} as const;

export type StatusDotTone = keyof typeof STATUS_DOT_TONES;
export type StatusDotSize = keyof typeof STATUS_DOT_SIZES;

export interface StatusDotProps {
  tone?: StatusDotTone;
  size?: StatusDotSize;
  /** Accessible name. Without it the dot is aria-hidden. */
  label?: string;
  /** A colour class (speaker colours) or layout tweaks. */
  className?: string;
}

export function StatusDot({ tone, size = 8, label, className }: StatusDotProps) {
  return (
    <span
      data-tone={tone}
      data-size={size}
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={cn(
        'inline-block flex-none rounded-full',
        STATUS_DOT_SIZES[size],
        tone && STATUS_DOT_TONES[tone],
        className,
      )}
    />
  );
}
