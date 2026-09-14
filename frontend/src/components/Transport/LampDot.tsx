import React from 'react';
import { cn } from '@/lib/utils';

// specs/0057 §2 — an indicator lamp: flat token fill + a tight halo. No blur filters.
// Lamp attack 120 ms / decay 400 ms is done with a transition on background/box-shadow.
//
// Durations/easings are written as arbitrary *properties* (`[transition-duration:120ms]`)
// rather than the usual duration/ease utilities with an arbitrary value: the
// `tailwindcss-animate` plugin registers its own duration/ease utilities on top of the core
// ones, so an arbitrary value there is ambiguous and Tailwind emits nothing (it warns:
// "matches multiple utilities"). Named steps (`ease-out`, `duration-150`) still work.
export type LampTone = 'off' | 'amber' | 'red' | 'green';

const TONE: Record<LampTone, string> = {
  off: 'bg-border shadow-[inset_0_1px_1px_rgba(0,0,0,0.5)] [transition-duration:400ms]',
  amber: 'bg-brand shadow-[0_0_0_1px_hsl(var(--brand)/0.25),0_0_8px_-1px_hsl(var(--brand)/0.55)] [transition-duration:120ms]',
  red: 'bg-record shadow-[0_0_0_1px_hsl(var(--record)/0.25),0_0_8px_-1px_hsl(var(--record)/0.55)] [transition-duration:120ms]',
  green: 'bg-success shadow-[0_0_0_1px_hsl(var(--success)/0.25),0_0_8px_-1px_hsl(var(--success)/0.55)] [transition-duration:120ms]',
};

export function LampDot({
  tone,
  label,
  decorative = false,
  className,
}: {
  tone: LampTone;
  label: string;
  /** Inside an already-labelled control, where the lamp's own name would pollute the
   *  accessible name ("Queue off Queue 0 Idle"). Renders purely decoratively. */
  decorative?: boolean;
  className?: string;
}) {
  return (
    <span
      // role="img", not "status": a status is a live region, and PEAK latching would then
      // spam every level change at screen readers. The lamp is a picture of a state.
      role={decorative ? undefined : 'img'}
      aria-hidden={decorative || undefined}
      aria-label={decorative ? undefined : tone === 'off' ? `${label} off` : label}
      data-tone={tone}
      className={cn(
        'inline-block h-2 w-2 flex-none rounded-full transition-[background-color,box-shadow] ease-out',
        TONE[tone],
        className,
      )}
    />
  );
}
