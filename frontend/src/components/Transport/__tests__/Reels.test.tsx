import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { Reels } from '@/components/Transport/Reels';

// NOTE: the hubs are SVG <g> elements, whose `.className` is an SVGAnimatedString (not a
// string) — `toContain` against it is vacuous in both directions. Read the `class`
// attribute instead so the "stops dead on hold" half of this test can actually fail.

describe('Reels', () => {
  it('spins only while recording and stops dead on hold', () => {
    const { container, rerender } = render(<Reels state="recording" />);
    expect(container.firstElementChild!.getAttribute('data-state')).toBe('recording');
    expect(container.querySelector('[data-hub]')!.getAttribute('class')).toContain('animate-reel');
    rerender(<Reels state="paused" />);
    expect(container.querySelector('[data-hub]')!.getAttribute('class')).not.toContain('animate-reel');
  });
});
