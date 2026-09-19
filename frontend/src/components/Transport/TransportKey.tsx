'use client';

import React from 'react';
import { cn } from '@/lib/utils';

// specs/0057 §3.1, geometry revised by specs/0063 W2 — an illuminated transport key: 48×44,
// 2px radius, 1px bevel, engraved 9px caps legend, and a 4px lamp bar across the top that
// lights in the key's own color with a real glow. Contents are centred below the bar.
// A lit key lights its whole face: REC red with cream ink, HOLD amber with dark ink.
// STOP is never lit.
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
  rec: 'bg-lamp-red [background-image:linear-gradient(rgba(255,255,255,0.45),rgba(255,255,255,0)_70%)] shadow-[0_0_0_1px_hsl(var(--lamp-red)/0.35),0_0_10px_1px_hsl(var(--lamp-red)/0.85),0_0_22px_4px_hsl(var(--lamp-red)/0.4)]',
  hold: 'bg-lamp-amber [background-image:linear-gradient(rgba(255,255,255,0.45),rgba(255,255,255,0)_70%)] shadow-[0_0_0_1px_hsl(var(--lamp-amber)/0.35),0_0_10px_1px_hsl(var(--lamp-amber)/0.85),0_0_22px_4px_hsl(var(--lamp-amber)/0.4)]',
  stop: '',
};

// specs/0063 W2 — 0057's rule is "REC is cream text on a lit red key", but only the 4px bar
// ever lit; the face stayed cream, which is the opposite. A fully lit key lights its face and
// flips its ink. STOP is never lit, so it has no face of its own.
const LIT_FACE: Record<TransportFn, string> = {
  rec: 'bg-lamp-red',
  hold: 'bg-lamp-amber',
  stop: '',
};
// --record-foreground is the cream that pairs with the red lamp: the ink token deliberately
// stays in the `record` family because no `--lamp-*` cream exists (only `--lamp-ink`, which is
// the dark ink that pairs with amber).
const LIT_GLYPH: Record<TransportFn, string> = {
  rec: 'fill-record-foreground',
  hold: 'fill-lamp-ink',
  stop: 'fill-engrave',
};
const LIT_LEGEND: Record<TransportFn, string> = {
  rec: 'text-record-foreground',
  hold: 'text-lamp-ink',
  stop: 'text-engrave',
};

// specs/0064 W6 — the on-HOLD treatment: standard face, lamp-coloured ink. Readable in both
// themes because the lamp tokens are the same colours the lit faces use, just as ink.
const HOLDING_GLYPH: Record<TransportFn, string> = {
  rec: 'fill-lamp-red',
  hold: 'fill-lamp-amber',
  stop: 'fill-engrave',
};
const HOLDING_LEGEND: Record<TransportFn, string> = {
  rec: 'text-lamp-red',
  hold: 'text-lamp-amber',
  stop: 'text-engrave',
};

export interface TransportKeyProps extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'children'> {
  fn: TransportFn;
  legend: string;
  lit?: boolean;
  /**
   * Lit, but the recording is on HOLD (specs/0064 W6). The key keeps the standard face and
   * takes RED ink with a slowly blinking lamp — "still recording, paused", as opposed to
   * the solid red face of a running recording or the plain engraved face of an idle deck.
   *
   * It replaces the old `dim`, which kept the unlit face while leaving the ink cream at
   * 55%: cream on an unlit key, which the owner read as unreadable rather than as a state.
   */
  holding?: boolean;
}

export function TransportKey({ fn, legend, lit = false, holding = false, disabled, className, ...rest }: TransportKeyProps) {
  // Lighting the whole face and then dimming it reads as neither state, so HOLD keeps the
  // unlit face and changes the INK instead.
  const faceLit = lit && !holding && LIT_FACE[fn] !== '';
  const holdingInk = lit && holding;
  return (
    <button
      type="button"
      // STOP is momentary — it has no pressed state to report, so it carries no aria-pressed.
      aria-pressed={fn === 'stop' ? undefined : lit}
      disabled={disabled}
      className={cn(
        // h-11 (44px) with the contents centred BELOW the lamp bar: at h-10/justify-end the
        // 12px glyph's top landed at ~7px, flush with the bar at y 3-7 (owner feedback, 0.3.1).
        'relative flex h-11 w-12 flex-none flex-col items-center justify-center gap-0.5 rounded-[2px] pt-2',
        faceLit ? LIT_FACE[fn] : 'bg-key',
        'shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_0_-1px_0_hsl(var(--bevel-lo)),0_1px_0_rgba(0,0,0,0.4)]',
        'transition-transform [transition-duration:60ms] [transition-timing-function:cubic-bezier(.2,0,0,1)] active:translate-y-px',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-panel',
        lit && 'shadow-[inset_0_1px_0_hsl(var(--bevel-lo)),inset_0_-1px_0_hsl(var(--bevel-hi))]',
        // A lit key already communicates its state via its lit face, aria-pressed, and the
        // disabled attribute itself — dimming it too (REC's `lit` strictly implies `disabled`)
        // composited the lit red face at 45%, which defeated the point of lighting it.
        disabled && !lit && 'opacity-[0.45]',
        className,
      )}
      {...rest}
    >
      <span
        aria-hidden
        className={cn(
          'absolute left-2 right-2 top-[3px] h-1 rounded-[1px] transition-[background-color,box-shadow] ease-out',
          lit ? cn(LIT_BAR[fn], '[transition-duration:120ms]') : 'bg-border [transition-duration:400ms]',
          // A named keyframe, never an arbitrary `duration-[…]`: tailwindcss-animate makes
          // those ambiguous and Tailwind emits nothing for them (same trap as the reels).
          holdingInk && 'animate-hold-blink motion-reduce:animate-none',
        )}
      />
      <svg
        aria-hidden
        viewBox="0 0 12 12"
        className={cn(
          'h-3 w-3',
          holdingInk ? HOLDING_GLYPH[fn] : lit ? LIT_GLYPH[fn] : 'fill-engrave',
        )}
      >
        {GLYPH[fn]}
      </svg>
      <span
        className={cn(
          'text-[9px] font-semibold tracking-[0.12em]',
          holdingInk ? HOLDING_LEGEND[fn] : lit ? LIT_LEGEND[fn] : 'text-engrave',
        )}
      >
        {legend}
      </span>
    </button>
  );
}
