'use client';

import React from 'react';
import { cn } from '@/lib/utils';

// specs/0057 §3.1 — an illuminated transport key: 48×40 (spec said 44×34; at that height the
// glyph collided with the lamp bar and the keys read as clipped — 0.1.0 owner feedback),
// 2px radius, 1px bevel, engraved 9px caps legend, and a 4px lamp bar across the top that
// lights in the key's own color with a real glow.
// REC lamp = record, HOLD lamp = brand, STOP is never lit.
//
// `[transition-duration:120ms]` rather than the usual duration utility with an arbitrary
// value — see the note in LampDot.tsx: `tailwindcss-animate` makes those ambiguous and
// Tailwind then emits nothing for them.
export type TransportFn = 'rec' | 'hold' | 'stop';

const GLYPH: Record<TransportFn, React.ReactNode> = {
  rec: <circle cx="6" cy="6" r="5" />,
  hold: (
    <>
      <rect x="1.5" y="1" width="3.2" height="10" />
      <rect x="7.3" y="1" width="3.2" height="10" />
    </>
  ),
  stop: <rect x="1.5" y="1.5" width="9" height="9" />,
};

const LIT_BAR: Record<TransportFn, string> = {
  // Lit bars: a bright core (white highlight over the token) plus a two-ring halo. The
  // original single 8px halo read as a dull dot in the panel — owner feedback for 0.1.0.
  rec: 'bg-record [background-image:linear-gradient(rgba(255,255,255,0.45),rgba(255,255,255,0)_70%)] shadow-[0_0_0_1px_hsl(var(--record)/0.35),0_0_10px_1px_hsl(var(--record)/0.85),0_0_22px_4px_hsl(var(--record)/0.4)]',
  hold: 'bg-brand [background-image:linear-gradient(rgba(255,255,255,0.45),rgba(255,255,255,0)_70%)] shadow-[0_0_0_1px_hsl(var(--brand)/0.35),0_0_10px_1px_hsl(var(--brand)/0.85),0_0_22px_4px_hsl(var(--brand)/0.4)]',
  stop: '',
};
const LIT_GLYPH: Record<TransportFn, string> = { rec: 'fill-record', hold: 'fill-brand', stop: 'fill-engrave' };

export interface TransportKeyProps extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'children'> {
  fn: TransportFn;
  legend: string;
  lit?: boolean;
  /** lit but dimmed to 55% (REC while on HOLD) */
  dim?: boolean;
}

export function TransportKey({ fn, legend, lit = false, dim = false, disabled, className, ...rest }: TransportKeyProps) {
  return (
    <button
      type="button"
      // STOP is momentary — it has no pressed state to report, so it carries no aria-pressed.
      aria-pressed={fn === 'stop' ? undefined : lit}
      disabled={disabled}
      className={cn(
        'relative flex h-10 w-12 flex-none flex-col items-center justify-end gap-1 rounded-[2px] pb-1.5',
        'bg-key shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_0_-1px_0_hsl(var(--bevel-lo)),0_1px_0_rgba(0,0,0,0.4)]',
        'transition-transform [transition-duration:60ms] [transition-timing-function:cubic-bezier(.2,0,0,1)] active:translate-y-px',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-panel',
        lit && 'shadow-[inset_0_1px_0_hsl(var(--bevel-lo)),inset_0_-1px_0_hsl(var(--bevel-hi))]',
        disabled && 'opacity-[0.45]',
        className,
      )}
      {...rest}
    >
      <span
        aria-hidden
        className={cn(
          'absolute left-2 right-2 top-[3px] h-1 rounded-[1px] transition-[background-color,box-shadow] ease-out',
          lit ? cn(LIT_BAR[fn], '[transition-duration:120ms]') : 'bg-border [transition-duration:400ms]',
          lit && dim && 'opacity-[0.55]',
        )}
      />
      <svg
        aria-hidden
        viewBox="0 0 12 12"
        className={cn('h-3 w-3', lit ? LIT_GLYPH[fn] : 'fill-engrave', lit && dim && 'opacity-[0.55]')}
      >
        {GLYPH[fn]}
      </svg>
      <span className={cn('text-[9px] font-semibold tracking-[0.12em]', lit ? 'text-foreground' : 'text-engrave')}>
        {legend}
      </span>
    </button>
  );
}
