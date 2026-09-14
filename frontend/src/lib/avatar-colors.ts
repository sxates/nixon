/**
 * Per-entity avatar / row-dot colour over the 8-slot chart palette (specs/0057).
 *
 * The people and meetings lists used to cycle a 4-entry rotation by list index, so a row's
 * colour changed whenever the list re-sorted or filtered, and only half the palette that
 * transcript speakers use was ever shown. Hashing a stable key (the entity id) instead gives
 * each person/meeting one colour for life and spreads across all 8 chart tokens.
 */
const CLASSES = [
  'bg-chart-1',
  'bg-chart-2',
  'bg-chart-3',
  'bg-chart-4',
  'bg-chart-5',
  'bg-chart-6',
  'bg-chart-7',
  'bg-chart-8',
] as const;

/** Stable per-entity avatar/dot color over the 8-slot chart palette (djb2 hash). */
export function avatarColorClass(key: string): string {
  let h = 5381;
  for (let i = 0; i < key.length; i++) h = ((h << 5) + h + key.charCodeAt(i)) | 0;
  return CLASSES[Math.abs(h) % CLASSES.length];
}
