import { describe, it, expect } from 'vitest';
import { avatarColorClass } from '@/lib/avatar-colors';

describe('avatarColorClass', () => {
  it('is stable for the same key', () => {
    expect(avatarColorClass('person-42')).toBe(avatarColorClass('person-42'));
  });

  it('only ever returns one of the 8 chart tokens', () => {
    const seen = new Set(
      Array.from({ length: 200 }, (_, i) => avatarColorClass(`id-${i}`)),
    );
    for (const cls of seen) expect(cls).toMatch(/^bg-chart-[1-8]$/);
  });

  it('spreads distinct keys across the whole 8-slot palette', () => {
    // The old 4-entry rotation could never reach chart-5..8; this is the regression guard.
    const seen = new Set(
      Array.from({ length: 200 }, (_, i) => avatarColorClass(`id-${i}`)),
    );
    expect(seen.size).toBe(8);
  });

  it('handles the empty key without throwing', () => {
    expect(avatarColorClass('')).toMatch(/^bg-chart-[1-8]$/);
  });
});
