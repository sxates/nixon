import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { LevelLadder } from '@/components/Transport/LevelLadder';

// Owner feedback 2026-09-19 — the rail's meter was too tall and wider than the timer above
// it, and a lit segment looked painted on rather than lit. Height is halved, `fill` makes it
// span exactly the width it is given (the counter's), and a lit segment glows in its own
// colour while unlit ones stay flat — the glow is the signal.

const segments = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('i')).map((el) => el.className);

describe('LevelLadder (specs/0064 W6 + owner feedback)', () => {
  it('glows only on the segments that are lit', () => {
    // 0.1 rms lights the first six segments and leaves four dark — a mix, which is the
    // point: at 0.2 the whole ladder is lit and the test would prove nothing about unlit.
    const { container } = render(<LevelLadder level={0.1} active />);
    const classes = segments(container);

    const lit = classes.filter((c) => c.includes('bg-success') || c.includes('bg-brand') || c.includes('bg-record'));
    const unlit = classes.filter((c) => c.includes('bg-border'));

    expect(lit.length).toBeGreaterThan(0);
    expect(unlit.length).toBeGreaterThan(0);
    for (const c of lit) expect(c).toMatch(/shadow-\[0_0_\dpx_hsl\(var\(--/);
    for (const c of unlit) expect(c).not.toContain('shadow-[0_0_');
  });

  it('an inactive ladder lights nothing, so it glows nowhere', () => {
    const { container } = render(<LevelLadder level={0.9} active={false} />);
    for (const c of segments(container)) {
      expect(c).toContain('bg-border');
      expect(c).not.toContain('shadow-[0_0_');
    }
  });

  it('is half its old height', () => {
    const { container } = render(<LevelLadder level={0} />);
    expect(container.firstElementChild?.className).toContain('h-[7px]');
    expect(container.firstElementChild?.className).not.toContain('h-3.5');
  });

  it('fill mode spans the width it is given instead of a fixed segment width', () => {
    const { container } = render(<LevelLadder level={0.5} fill />);
    expect(container.firstElementChild?.className).toContain('w-full');
    for (const c of segments(container)) {
      expect(c).toContain('flex-1');
      expect(c).not.toContain('w-1.5');
    }
  });
});
