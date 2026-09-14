import { describe, it, expect } from 'vitest';
import { formatElapsedHms } from '@/lib/transport/format-elapsed';

// specs/0057 §3.3 — a counter does not hide its digits: always h:mm:ss with leading zeros.
describe('formatElapsedHms', () => {
  it('formats zero and sub-minute values', () => {
    expect(formatElapsedHms(0)).toBe('0:00:00');
    expect(formatElapsedHms(7)).toBe('0:00:07');
  });
  it('formats minutes and hours', () => {
    expect(formatElapsedHms(252)).toBe('0:04:12');
    expect(formatElapsedHms(3852)).toBe('1:04:12');
    expect(formatElapsedHms(36000)).toBe('10:00:00');
  });
  it('floors fractions and clamps garbage', () => {
    expect(formatElapsedHms(59.9)).toBe('0:00:59');
    expect(formatElapsedHms(-5)).toBe('0:00:00');
    expect(formatElapsedHms(Number.NaN)).toBe('0:00:00');
  });
});
