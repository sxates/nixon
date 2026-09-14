import { describe, it, expect } from 'vitest';
import { splitForDisplay } from '@/lib/list-overflow';

const items = Array.from({ length: 14 }, (_, i) => i);

describe('splitForDisplay (WS3.1)', () => {
  it('shows everything when under the cap', () => {
    expect(splitForDisplay([1, 2, 3], 10, false)).toEqual({ shown: [1, 2, 3], hidden: 0 });
  });

  it('shows everything at exactly the cap', () => {
    const ten = items.slice(0, 10);
    expect(splitForDisplay(ten, 10, false)).toEqual({ shown: ten, hidden: 0 });
  });

  it('caps and reports the hidden count when over', () => {
    const r = splitForDisplay(items, 10, false);
    expect(r.shown).toHaveLength(10);
    expect(r.hidden).toBe(4);
  });

  it('shows everything (hidden 0) when expanded', () => {
    const r = splitForDisplay(items, 10, true);
    expect(r.shown).toHaveLength(14);
    expect(r.hidden).toBe(0);
  });

  it('treats cap <= 0 as no cap', () => {
    expect(splitForDisplay(items, 0, false)).toEqual({ shown: items, hidden: 0 });
  });
});
