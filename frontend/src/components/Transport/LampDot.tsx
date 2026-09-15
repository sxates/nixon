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
  // Lit lamps carry a bright core highlight and a two-ring halo (0.1.0 feedback: the
  // recording lamp looked dull). Still no blur filters — box-shadow only.
  amber: 'bg-brand [background-image:radial-gradient(circle_at_35%_30%,rgba(255,255,255,0.55),rgba(255,255,255,0)_60%)] shadow-[0_0_0_1px_hsl(var(--brand)/0.35),0_0_8px_1px_hsl(var(--brand)/0.8),0_0_16px_3px_hsl(var(--brand)/0.35)] [transition-duration:120ms]',
  red: 'bg-record [background-image:radial-gradient(circle_at_35%_30%,rgba(255,255,255,0.55),rgba(255,255,255,0)_60%)] shadow-[0_0_0_1px_hsl(var(--record)/0.35),0_0_8px_1px_hsl(var(--record)/0.8),0_0_16px_3px_hsl(var(--record)/0.35)] [transition-duration:120ms]',
  green: 'bg-success [background-image:radial-gradient(circle_at_35%_30%,rgba(255,255,255,0.55),rgba(255,255,255,0)_60%)] shadow-[0_0_0_1px_hsl(var(--success)/0.35),0_0_8px_1px_hsl(var(--success)/0.8),0_0_16px_3px_hsl(var(--success)/0.35)] [transition-duration:120ms]',
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
