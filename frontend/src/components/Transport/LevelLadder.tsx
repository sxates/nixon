import React from 'react';
import { cn } from '@/lib/utils';
import { ladderSegments } from '@/lib/transport/ladder';
import { rmsToVu, vuToArcFraction } from '@/lib/transport/vu-ballistics';

const SEG: Record<'off' | 'g' | 'a' | 'r', string> = {
  off: 'bg-border/70',
  g: 'bg-success',
  a: 'bg-brand',
  r: 'bg-record',
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
  className,
}: {
  level: number;
  count?: number;
  active?: boolean;
  className?: string;
}) {
  // `level` is linear rms (0..1). Map it through the same VU face as the needle so the
  // ladder and the meters agree: 0 VU (-18 dBFS) lights segment 8 of 10, the red pair is
  // the over-zone. A raw linear map lit ≤1 segment for -23 LUFS normalised mic speech.
  const segs = ladderSegments(active ? vuToArcFraction(rmsToVu(level)) : 0, count);
  return (
    <span aria-hidden className={cn('inline-flex h-3.5 items-end gap-0.5', className)}>
      {segs.map((s, i) => (
        <i
          key={i}
          className={cn(
            'block h-full w-1.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] transition-colors [transition-duration:60ms]',
            SEG[s],
          )}
        />
      ))}
    </span>
  );
}
