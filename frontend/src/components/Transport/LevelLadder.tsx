import React from 'react';
import { cn } from '@/lib/utils';
import { ladderSegments } from '@/lib/transport/ladder';
import { rmsToVu, vuToArcFraction } from '@/lib/transport/vu-ballistics';

// Owner feedback 2026-09-19 — a lit segment glows in its own colour, the way a real meter's
// lamps bleed onto the panel around them. Unlit segments stay flat: the glow IS the signal,
// so giving it to everything would say nothing. Each shadow is built from the same token as
// the fill, so Deck and Faceplate stay in parity without a second palette.
const SEG: Record<'off' | 'g' | 'a' | 'r', string> = {
  off: 'bg-border/70',
  g: 'bg-success shadow-[0_0_4px_hsl(var(--success)/0.75)]',
  a: 'bg-brand shadow-[0_0_4px_hsl(var(--brand)/0.75)]',
  r: 'bg-record shadow-[0_0_5px_hsl(var(--record)/0.85)]',
};

/**
 * specs/0057 decision 6 — 10-segment level ladder for compact spots (rail, device picker).
 * `level` is linear rms; see below for the VU mapping.
 *
 * The 60 ms segment fade is written as an arbitrary *property*
 * (`[transition-duration:60ms]`): `tailwindcss-animate` registers its own duration
 * utilities on top of the core ones, so an arbitrary value on the duration utility is
 * ambiguous and Tailwind emits nothing for it (same trap as LampDot).
 */
export function LevelLadder({
  level,
  count = 10,
  active = true,
  fill = false,
  className,
}: {
  level: number;
  count?: number;
  active?: boolean;
  /** Stretch the segments to fill the available width (specs/0064 W6 — the rail's
   *  instrument stack sizes the ladder to the tape counter above it). */
  fill?: boolean;
  className?: string;
}) {
  // `level` is linear rms (0..1). Map it through the same VU face as the needle so the
  // ladder and the meters agree: 0 VU (-18 dBFS) lights segment 8 of 10, the red pair is
  // the over-zone. A raw linear map lit ≤1 segment for -23 LUFS normalised mic speech.
  const segs = ladderSegments(active ? vuToArcFraction(rmsToVu(level)) : 0, count);
  return (
    <span
      aria-hidden
      className={cn(
        // h-[7px]: half the old 14px (owner feedback 2026-09-19) — at rail size the ladder
        // is a level indication, not a second instrument competing with the counter.
        'h-[7px] items-end gap-0.5',
        fill ? 'flex w-full' : 'inline-flex',
        className,
      )}
    >
      {segs.map((s, i) => (
        <i
          key={i}
          className={cn(
            'block h-full shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] transition-colors [transition-duration:60ms]',
            fill ? 'min-w-0 flex-1' : 'w-1.5',
            SEG[s],
          )}
        />
      ))}
    </span>
  );
}
