import React from 'react';
import { cn } from '@/lib/utils';

export type ReelState = 'idle' | 'recording' | 'paused' | 'finalizing';

/**
 * specs/0057 §3.4 — the recording indicator: two hubs joined by a tape path. Turning at
 * 0.85 rev/s while recording; stops dead on HOLD (no ease-out); 600 ms spin-down when
 * finalizing. Never blinks. Under prefers-reduced-motion the hubs stay still (the REC lamp
 * and the counter carry liveness) — done with Tailwind's motion-reduce variant.
 */
export function Reels({ state, size = 22, className }: { state: ReelState; size?: number; className?: string }) {
  const spin = state === 'recording';
  const hub = cn(
    'origin-center',
    spin && 'animate-reel motion-reduce:animate-none',
    state === 'finalizing' && 'animate-reel-spindown motion-reduce:animate-none',
  );
  const h = size;
  const w = Math.round(size * (50 / 22));
  return (
    <svg data-state={state} width={w} height={h} viewBox="0 0 50 22" className={cn('block', className)} aria-hidden>
      <path d="M11 20 L39 20" className="stroke-engrave" strokeWidth="1" fill="none" />
      <g data-hub className={hub} style={{ transformOrigin: '11px 11px' }}>
        <circle cx="11" cy="11" r="8.5" className="stroke-foreground" strokeWidth="1.5" fill="none" />
        <circle cx="11" cy="11" r="5" className="stroke-foreground" strokeWidth="1" fill="none" />
        <g className="stroke-foreground" strokeWidth="1.6" strokeLinecap="square">
          <path d="M11 6 L11 3.5" />
          <path d="M6.7 13.5 L4.5 14.7" />
          <path d="M15.3 13.5 L17.5 14.7" />
        </g>
      </g>
      <g className={hub} style={{ transformOrigin: '39px 11px' }}>
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
