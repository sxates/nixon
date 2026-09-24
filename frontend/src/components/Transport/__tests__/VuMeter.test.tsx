import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, act } from '@testing-library/react';

// Plan 2 re-review — the rAF clock must survive the loop's re-arm. `db` is an effect dep (so
// the parked loop restarts when a level arrives), and a `let last = performance.now()` inside
// the effect was re-seeded at re-arm time — mid-frame — so every tick measured only the
// sliver since the re-arm instead of since the previous integrator step.
const { steps } = vi.hoisted(() => ({ steps: [] as number[] }));
vi.mock('@/lib/transport/vu-ballistics', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/transport/vu-ballistics')>();
  return {
    ...actual,
    createVuIntegrator: () => {
      const inner = actual.createVuIntegrator();
      return {
        reset: (v: number) => inner.reset(v),
        step: (targetDb: number, dt: number) => {
          steps.push(dt);
          return inner.step(targetDb, dt);
        },
      };
    },
  };
});

// specs/0077 — Low Power Mode on battery ("calm motion") takes the Reduce Motion path.
const calm = vi.hoisted(() => ({ on: false }));
vi.mock('@/contexts/CalmMotionContext', () => ({ useCalmMotion: () => calm.on }));

import { VuMeter } from '@/components/Transport/VuMeter';

let now = 0;
let nextId = 1;
const pending = new Map<number, FrameRequestCallback>();
const realRaf = window.requestAnimationFrame;
const realCancel = window.cancelAnimationFrame;

/** Run every queued frame callback after advancing the clock by `ms`. */
const frame = (ms: number) => {
  now += ms;
  const due = Array.from(pending.entries());
  pending.clear();
  act(() => {
    due.forEach(([, cb]) => cb(now));
  });
};
/** Advance the clock WITHOUT running a frame (a level update landing between frames). */
const advance = (ms: number) => {
  now += ms;
};

beforeEach(() => {
  calm.on = false;
  steps.length = 0;
  now = 0;
  nextId = 1;
  pending.clear();
  vi.spyOn(performance, 'now').mockImplementation(() => now);
  window.matchMedia = vi.fn().mockImplementation((q: string) => ({
    matches: false, media: q, addEventListener: vi.fn(), removeEventListener: vi.fn(),
  })) as unknown as typeof window.matchMedia;
  window.requestAnimationFrame = ((cb: FrameRequestCallback) => {
    const id = nextId++;
    pending.set(id, cb);
    return id;
  }) as typeof window.requestAnimationFrame;
  window.cancelAnimationFrame = ((id: number) => {
    pending.delete(id);
  }) as typeof window.cancelAnimationFrame;
});

afterEach(() => {
  vi.restoreAllMocks();
  window.requestAnimationFrame = realRaf;
  window.cancelAnimationFrame = realCancel;
});

describe('VuMeter', () => {
  it('a level tick does not reset the integrator clock', () => {
    const { rerender } = render(<VuMeter db={-20} active label="MIX" />);
    frame(16);
    expect(steps).toEqual([16]);

    // A new level arrives 8 ms after the last frame; the effect re-arms the loop.
    advance(8);
    rerender(<VuMeter db={-10} active label="MIX" />);
    // The next frame is a full 16 ms after the PREVIOUS step, not 8 ms after the re-arm.
    frame(8);
    expect(steps).toEqual([16, 16]);
  });

  it('clamps dt after a long park so one frame cannot sub-step forever', () => {
    render(<VuMeter db={-20} active label="MIX" />);
    frame(5000);
    expect(steps).toEqual([100]);
  });

  it('runs no animation loop in calm motion, and snaps the needle to each level', () => {
    calm.on = true;
    const { rerender, getByRole } = render(<VuMeter db={-10} active label="MIX" />);
    expect(pending.size).toBe(0);
    expect(getByRole('img').getAttribute('aria-label')).toBe('MIX level -10 VU');
    rerender(<VuMeter db={-3} active label="MIX" />);
    expect(pending.size).toBe(0);
    expect(getByRole('img').getAttribute('aria-label')).toBe('MIX level -3 VU');
  });
});
