import React from 'react';
import { cn } from '@/lib/utils';

export type ReelState = 'idle' | 'recording' | 'paused' | 'finalizing' | 'rewinding';

/**
 * specs/0057 §3.4 — the recording indicator: two hubs joined by a tape path. While
 * recording the take-up hub (right) turns at ~0.38 rev/s and the supply hub (left) at
 * ~0.28 rev/s, like a real deck; stops dead on HOLD (no ease-out); 900 ms spin-down when
 * finalizing. Never blinks. Under prefers-reduced-motion the hubs stay still (the REC lamp
 * and the counter carry liveness) — done with Tailwind's motion-reduce variant.
 *
 * `rewinding` (specs/0066) is the deck going back over a tape it already has: both hubs
 * turn BACKWARDS at several times transport speed, and they swap roles — in rewind the
 * supply reel (left) is the one winding tape on, so it becomes the fast hub. That
 * inversion is what makes it read as rewind rather than "recording, but backwards", and it
 * is why this can never be mistaken for a take in progress.
 */
export function Reels({ state, size = 22, className }: { state: ReelState; size?: number; className?: string }) {
  const spin = state === 'recording';
  const rewind = state === 'rewinding';
  const hubClass = (fast: boolean) =>
    cn(
      'origin-center',
      spin && `${fast ? 'animate-reel' : 'animate-reel-slow'} motion-reduce:animate-none`,
      rewind && `${fast ? 'animate-reel-rewind' : 'animate-reel-rewind-slow'} motion-reduce:animate-none`,
      state === 'finalizing' && 'animate-reel-spindown motion-reduce:animate-none',
    );
  // Recording winds onto the take-up hub; rewinding winds back onto the supply hub.
  const supply = hubClass(rewind);
  const takeUp = hubClass(!rewind);
  const h = size;
  const w = Math.round(size * (50 / 22));
  return (
    <svg data-state={state} width={w} height={h} viewBox="0 0 50 22" className={cn('block', className)} aria-hidden>
      <path d="M11 20 L39 20" className="stroke-engrave" strokeWidth="1" fill="none" />
      <g data-hub className={supply} style={{ transformOrigin: '11px 11px' }}>
        <circle cx="11" cy="11" r="8.5" className="stroke-foreground" strokeWidth="1.5" fill="none" />
        <circle cx="11" cy="11" r="5" className="stroke-foreground" strokeWidth="1" fill="none" />
        <g className="stroke-foreground" strokeWidth="1.6" strokeLinecap="square">
          <path d="M11 6 L11 3.5" />
          <path d="M6.7 13.5 L4.5 14.7" />
          <path d="M15.3 13.5 L17.5 14.7" />
        </g>
      </g>
      <g data-hub className={takeUp} style={{ transformOrigin: '39px 11px' }}>
        <circle cx="39" cy="11" r="8.5" className="stroke-foreground" strokeWidth="1.5" fill="none" />
        <circle cx="39" cy="11" r="3.2" className="stroke-foreground" strokeWidth="1" fill="none" />
        <g className="stroke-foreground" strokeWidth="1.6" strokeLinecap="square">
          <path d="M39 8 L39 3.5" />
          <path d="M36.3 12.6 L32.5 14.7" />
          <path d="M41.7 12.6 L45.5 14.7" />
        </g>
      </g>
    </svg>
  );
}
