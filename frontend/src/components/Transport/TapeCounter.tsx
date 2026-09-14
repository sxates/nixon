'use client';

import React from 'react';
import { cn } from '@/lib/utils';
import { formatElapsedHms } from '@/lib/transport/format-elapsed';

// specs/0057 §3.3 — mechanical roller counter: six digit wells (hh:mm:ss) machined into the
// panel, each digit rolling vertically (110 ms) when it changes. Not 7-segment, not Nixie.
//
// Every glyph the counter *draws* — the ten roll rows per well and the two colons — is
// painted by `.tape-glyph::before { content: attr(data-d) }` (globals.css), so the element's
// textContent is exactly one thing: the `sr-only` `h:mm:ss` string at the root.
//
// `[transition-duration:120ms]` rather than the usual duration utility with an arbitrary
// value — see the note in LampDot.tsx: `tailwindcss-animate` makes those ambiguous and
// Tailwind then emits nothing for them.
type Size = 'sm' | 'xs';
type Tone = 'normal' | 'amber' | 'dim';

const WELL: Record<Size, string> = {
  sm: 'w-3 h-[18px] text-[13px]',
  xs: 'w-2.5 h-[15px] text-[11px]',
};
const COLON: Record<Size, string> = { sm: 'text-[12px]', xs: 'text-[10px]' };
const TONE: Record<Tone, string> = {
  normal: 'text-foreground',
  amber: 'text-brand',
  dim: 'text-muted-foreground',
};

const ROWS = Array.from({ length: 10 }, (_, i) => String(i));

function Digit({ value, size, tone }: { value: string; size: Size; tone: Tone }) {
  // The roll: a strip ten wells tall (h-[1000%], rows of h-[10%]) translated up by n wells.
  const n = Number(value);
  return (
    <span
      data-digit={value}
      className={cn(
        'relative inline-flex overflow-hidden rounded-[2px] bg-well font-sans font-semibold tabular-nums tracking-[-0.02em]',
        'shadow-[inset_0_1px_2px_rgba(0,0,0,0.6),inset_0_-1px_0_hsl(var(--bevel-hi))]',
        WELL[size],
        TONE[tone],
      )}
    >
      <span
        aria-hidden
        className="absolute inset-x-0 top-0 flex h-[1000%] w-full flex-col transition-transform [transition-duration:110ms] ease-out motion-reduce:transition-none"
        style={{ transform: `translateY(-${n * 10}%)` }}
      >
        {ROWS.map((d) => (
          <i
            key={d}
            data-d={d}
            className="tape-glyph flex h-[10%] w-full items-center justify-center not-italic leading-none"
          />
        ))}
      </span>
    </span>
  );
}

/**
 * `h:mm:ss` → the counter's own `hh:mm:ss` reading: hours padded to the well count, so the
 * accessible name and the sr-only text say exactly what the six wells display (a counter
 * shows `00:04:12`, so that is what it must read out too).
 */
function counterText(text: string): string {
  const [h = '0', m = '00', s = '00'] = text.split(':');
  return `${h.padStart(2, '0')}:${m}:${s}`;
}

export function TapeCounter({
  seconds,
  size = 'sm',
  tone = 'normal',
  className,
  decorative = false,
}: {
  seconds: number;
  size?: Size;
  tone?: Tone;
  className?: string;
  /**
   * specs/0057 Task 6 fix round 1: a decorative counter (e.g. one of many rows in a
   * list) renders the digit wells with no `role="timer"` / `aria-label` and is
   * `aria-hidden` — the row's own accessible name should carry the human duration
   * instead, or every row in the list would announce "Elapsed time 00:42:18".
   */
  decorative?: boolean;
}) {
  const text = counterText(formatElapsedHms(seconds));
  const digits = text.replace(/:/g, '').split('');
  const hourWells = digits.length - 4;
  return (
    <span
      role={decorative ? undefined : 'timer'}
      aria-label={decorative ? undefined : `Elapsed time ${text}`}
      aria-hidden={decorative ? 'true' : undefined}
      data-tone={tone}
      className={cn('inline-flex items-center gap-0.5', className)}
    >
      {digits.map((ch, i) => (
        <React.Fragment key={i}>
          {(i === hourWells || i === hourWells + 2) && (
            <i
              aria-hidden
              data-d=":"
              className={cn('tape-glyph px-px font-semibold not-italic text-engrave', COLON[size])}
            />
          )}
          <Digit value={ch} size={size} tone={tone} />
        </React.Fragment>
      ))}
      {!decorative && <span className="sr-only">{text}</span>}
    </span>
  );
}
