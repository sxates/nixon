import { describe, it, expect } from 'vitest';
import { createVuIntegrator, rmsToVu, vuToArcFraction } from '@/lib/transport/vu-ballistics';

// specs/0057 §3.2 — true VU ballistics: 300 ms to 99% of a step input, symmetric return,
// ≤1.5% overshoot. Stepped at the 12.5 Hz feed and at 60 fps; both must converge.
describe('createVuIntegrator', () => {
  function runStep(dtMs: number, totalMs: number) {
    const vu = createVuIntegrator();
    let t = 0;
    let max = -Infinity;
    let at99: number | null = null;
    while (t < totalMs) {
      const v = vu.step(0, dtMs); // target 0 dB from rest at -20 dB
      max = Math.max(max, v);
      if (at99 === null && v >= -0.2) at99 = t; // 99% of a 20 dB step
      t += dtMs;
    }
    return { max, at99, final: vu.value() };
  }
  it('reaches 99% of a step within ~300 ms at 60 fps', () => {
    const { at99, max, final } = runStep(16.7, 1000);
    expect(at99).not.toBeNull();
    expect(at99!).toBeLessThanOrEqual(340);
    expect(at99!).toBeGreaterThanOrEqual(220);
    expect(max).toBeLessThanOrEqual(0.3); // ≤1.5% of 20 dB overshoot
    expect(final).toBeCloseTo(0, 1);
  });
  it('converges without blowing up at the 80 ms feed cadence', () => {
    const { max, final } = runStep(80, 1000);
    expect(max).toBeLessThanOrEqual(0.5);
    expect(final).toBeCloseTo(0, 1);
  });
  it('returns symmetrically', () => {
    const vu = createVuIntegrator();
    for (let i = 0; i < 60; i++) vu.step(0, 16.7);
    let t = 0;
    let at99: number | null = null;
    while (t < 1000) {
      const v = vu.step(-20, 16.7);
      if (at99 === null && v <= -19.8) at99 = t;
      t += 16.7;
    }
    expect(at99!).toBeLessThanOrEqual(340);
  });
});

describe('rmsToVu / vuToArcFraction', () => {
  it('maps rms to the -20..+3 VU face with 0 VU at -18 dBFS', () => {
    expect(rmsToVu(0)).toBe(-20);
    expect(rmsToVu(Math.pow(10, -18 / 20))).toBeCloseTo(0, 5); // -18 dBFS → 0 VU
    expect(rmsToVu(Math.pow(10, -23 / 20))).toBeCloseTo(-5, 5); // -23 LUFS mic speech → -5 VU
    expect(rmsToVu(Math.pow(10, -38 / 20))).toBe(-20); // floor
    expect(rmsToVu(1)).toBe(3); // full scale clamps into the red
    expect(rmsToVu(2)).toBe(3);
  });
  it('puts 0 VU at 72% of arc travel and -20 at 0', () => {
    expect(vuToArcFraction(-20)).toBe(0);
    expect(vuToArcFraction(0)).toBeCloseTo(0.72, 5);
    expect(vuToArcFraction(3)).toBe(1);
  });
});
