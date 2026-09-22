'use client';

/**
 * "Recording" marker for a meeting row — spinning reels plus the word.
 *
 * Extracted from `app/meetings/page.tsx` on owner feedback 2026-09-21: All Meetings showed
 * the animated reels on the row being recorded and Today showed nothing at all, so the same
 * live meeting looked live on one screen and inert on the other. One component, both
 * surfaces.
 */

import { Reels } from '@/components/Transport/Reels';

export function RecordingBadge({ size = 14 }: { size?: number }) {
  return (
    <span className="inline-flex flex-shrink-0 items-center gap-1.5">
      <Reels state="recording" size={size} />
      <span className="u-section-label text-record-ink">Recording</span>
    </span>
  );
}
