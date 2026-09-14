export type LadderSeg = 'off' | 'g' | 'a' | 'r';

/** specs/0057 decision 6 — the compact level meter: N segments, last two red, two before amber. */
export function ladderSegments(level01: number, count = 10): LadderSeg[] {
  const lit = Math.round(Math.min(1, Math.max(0, level01)) * count);
  return Array.from({ length: count }, (_, i) => {
    if (i >= lit) return 'off';
    if (i >= count - 2) return 'r';
    if (i >= count - 4) return 'a';
    return 'g';
  });
}
