/**
 * Stable per-speaker colors for the transcript view (specs/0010, P1-C).
 *
 * P1 is intentionally minimal: we only need a *stable* accent color derived from
 * the speaker key so consecutive segments from the same person read consistently.
 * The full legend + rename/merge UI (and richer palettes/avatars) is P2 — keep
 * this light. NULL/absent keys render muted (unlabeled, exactly as before).
 */

/** Semantic channel-color classes for speaker name labels (specs/0057). The chart
 *  tokens are theme-aware, so these flip with Faceplate/Deck instead of staying a
 *  light-only palette; the local user ("local") gets the fixed CH1 slot, and the
 *  seven remaining channels (CH2..CH8) carry the remote speakers. */
const SPEAKER_COLOR_CLASSES = [
  'text-chart-1', // local user ("You") — CH1, the owner mic (specs/0046)
  'text-chart-2',
  'text-chart-3',
  'text-chart-4',
  'text-chart-5',
  'text-chart-6',
  'text-chart-7',
  'text-chart-8',
] as const;

/** Deterministic small hash of a string (djb2). */
function hashKey(key: string): number {
  let hash = 5381;
  for (let i = 0; i < key.length; i++) {
    hash = ((hash << 5) + hash + key.charCodeAt(i)) | 0;
  }
  return Math.abs(hash);
}

/**
 * A stable Tailwind text-color class for a speaker key. The local user ("local")
 * is pinned to the first slot so "You" is consistently the primary accent;
 * everyone else is hashed across the remaining palette.
 */
export function speakerColorClass(speakerKey: string | null | undefined): string {
  if (!speakerKey) return 'text-muted-foreground';
  if (speakerKey === 'local') return SPEAKER_COLOR_CLASSES[0];
  // Reserve slot 0 for the local user; spread remotes over the other seven.
  const rest = SPEAKER_COLOR_CLASSES.slice(1);
  return rest[hashKey(speakerKey) % rest.length];
}

/** Tailwind background-color classes for speaker avatars/dots, index-aligned with
 *  {@link SPEAKER_COLOR_CLASSES} so a speaker's dot matches its transcript label. */
const SPEAKER_BG_CLASSES = [
  'bg-chart-1', // local user ("You") — CH1, the owner mic (specs/0046)
  'bg-chart-2',
  'bg-chart-3',
  'bg-chart-4',
  'bg-chart-5',
  'bg-chart-6',
  'bg-chart-7',
  'bg-chart-8',
] as const;

/**
 * A stable Tailwind background-color class for a speaker key — the avatar/dot
 * counterpart to {@link speakerColorClass}, keyed identically so the legend dot
 * and the in-transcript name share a color.
 */
export function speakerBgClass(speakerKey: string | null | undefined): string {
  if (!speakerKey) return 'bg-muted';
  if (speakerKey === 'local') return SPEAKER_BG_CLASSES[0];
  const rest = SPEAKER_BG_CLASSES.slice(1);
  return rest[hashKey(speakerKey) % rest.length];
}
