import { describe, it, expect } from 'vitest';
import { ladderSegments } from '@/lib/transport/ladder';

describe('ladderSegments', () => {
  it('lights proportionally with green, then amber, then red at the top', () => {
    expect(ladderSegments(0)).toEqual(Array(10).fill('off'));
    expect(ladderSegments(0.5)).toEqual(['g', 'g', 'g', 'g', 'g', 'off', 'off', 'off', 'off', 'off']);
    expect(ladderSegments(1)).toEqual(['g', 'g', 'g', 'g', 'g', 'g', 'a', 'a', 'r', 'r']);
  });
  it('clamps', () => {
    expect(ladderSegments(-1)[0]).toBe('off');
    expect(ladderSegments(9)[9]).toBe('r');
  });
});
