import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { Reels } from '@/components/Transport/Reels';

// NOTE: the hubs are SVG <g> elements, whose `.className` is an SVGAnimatedString (not a
// string) — `toContain` against it is vacuous in both directions. Read the `class`
// attribute instead so the "stops dead on hold" half of this test can actually fail.

const hubs = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('[data-hub]')).map((g) => g.getAttribute('class') ?? '');

describe('Reels', () => {
  it('spins only while recording and stops dead on hold', () => {
    const { container, rerender } = render(<Reels state="recording" />);
    expect(container.firstElementChild!.getAttribute('data-state')).toBe('recording');
    expect(container.querySelector('[data-hub]')!.getAttribute('class')).toContain('animate-reel');
    rerender(<Reels state="paused" />);
    expect(container.querySelector('[data-hub]')!.getAttribute('class')).not.toContain('animate-reel');
  });

  // specs/0066 — summarizing rewinds. Two things have to hold for it to read as rewind
  // rather than "recording, but somehow different": the hubs turn the other way, and they
  // SWAP roles, because in rewind the supply reel (left) is the one winding tape on.
  it('rewinds backwards with the supply hub leading', () => {
    const { container } = render(<Reels state="rewinding" />);
    const [supply, takeUp] = hubs(container);

    expect(container.firstElementChild!.getAttribute('data-state')).toBe('rewinding');
    // Backwards: the rewind keyframe, never the forward one.
    expect(supply).toContain('animate-reel-rewind');
    expect(takeUp).toContain('animate-reel-rewind');
    expect(supply).not.toMatch(/animate-reel(?!-rewind)/);

    // Swapped: supply is the FAST hub here, the mirror of recording.
    expect(supply).not.toContain('animate-reel-rewind-slow');
    expect(takeUp).toContain('animate-reel-rewind-slow');
  });

  it('keeps recording the other way round, with the take-up hub leading', () => {
    const { container } = render(<Reels state="recording" />);
    const [supply, takeUp] = hubs(container);
    expect(supply).toContain('animate-reel-slow');
    expect(takeUp).not.toContain('animate-reel-slow');
    expect(takeUp).toContain('animate-reel');
  });

  it('honours prefers-reduced-motion while rewinding', () => {
    const { container } = render(<Reels state="rewinding" />);
    for (const hub of hubs(container)) expect(hub).toContain('motion-reduce:animate-none');
  });
});
