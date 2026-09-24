import { describe, it, expect } from 'vitest';
import { calmMotion } from '@/lib/calm-motion';

describe('calmMotion (specs/0077)', () => {
  it('is on only with Low Power Mode on and the Mac on battery', () => {
    expect(calmMotion(true, true)).toBe(true);
    expect(calmMotion(true, false)).toBe(false);
    expect(calmMotion(false, true)).toBe(false);
    expect(calmMotion(false, false)).toBe(false);
  });
});
