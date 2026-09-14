import { describe, it, expect } from 'vitest';
import { formatReelNumber, formatReelTag } from '@/lib/reel-number';

describe('formatReelNumber', () => {
  it('zero-pads to four digits', () => {
    expect(formatReelNumber(412)).toBe('REEL 0412');
    expect(formatReelNumber(1)).toBe('REEL 0001');
    expect(formatReelNumber(42)).toBe('REEL 0042');
    expect(formatReelNumber(9999)).toBe('REEL 9999');
  });

  it('keeps every digit past four', () => {
    expect(formatReelNumber(10000)).toBe('REEL 10000');
    expect(formatReelNumber(123456)).toBe('REEL 123456');
  });

  it('falls back to an em-dash placeholder when the number is unknown', () => {
    expect(formatReelNumber(null)).toBe('REEL ——');
    expect(formatReelNumber(undefined)).toBe('REEL ——');
    expect(formatReelNumber(0)).toBe('REEL ——');
    expect(formatReelNumber(-3)).toBe('REEL ——');
    expect(formatReelNumber(Number.NaN)).toBe('REEL ——');
  });

  it('floors a non-integer rather than printing a fraction', () => {
    expect(formatReelNumber(12.7)).toBe('REEL 0012');
  });
});

describe('formatReelTag', () => {
  it('is the short spine tag for a log line', () => {
    expect(formatReelTag(412)).toBe('R0412');
    expect(formatReelTag(1)).toBe('R0001');
    expect(formatReelTag(123456)).toBe('R123456');
  });

  it('is empty when there is no reel ordinal — a log line stays blank, never "R——"', () => {
    expect(formatReelTag(null)).toBe('');
    expect(formatReelTag(undefined)).toBe('');
    expect(formatReelTag(0)).toBe('');
    expect(formatReelTag(Number.NaN)).toBe('');
  });
});
