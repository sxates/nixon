/**
 * Spinner — the one in-progress glyph (docs/design-system.md). `sm` = 14px (inline with
 * text and in buttons), `md` = 20px (a panel or page that is loading). It inherits the
 * surrounding text colour, so a muted line gets a muted spinner. Never a raw `Loader2` or
 * a CSS border spinner in a screen.
 *
 * Decorative by default; pass `label` when the spinner is the only signal that something
 * is happening, and it becomes a `role="status"` with that accessible name.
 */

import { Loader2 } from '@/components/ui/icons';
import { cn } from '@/lib/utils';

const SIZES = { sm: 14, md: 20 } as const;

export interface SpinnerProps {
  size?: keyof typeof SIZES;
  /** Accessible name. Without it the spinner is aria-hidden. */
  label?: string;
  className?: string;
}

export function Spinner({ size = 'sm', label, className }: SpinnerProps) {
  const px = SIZES[size];
  const glyph = (
    <Loader2
      size={px}
      strokeWidth={1.5}
      className={cn('flex-none animate-spin', className)}
      aria-hidden
      data-size={size}
    />
  );
  if (!label) return glyph;
  return (
    <span role="status" aria-label={label} className="inline-flex">
      {glyph}
    </span>
  );
}
