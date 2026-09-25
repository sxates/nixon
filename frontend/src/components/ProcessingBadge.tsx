'use client';

/**
 * "Processing" marker for a meeting row: the queue rail's amber lamp plus the word, shown
 * while the meeting has transcription, speaker, summary or other AI work in flight
 * (`useProcessingMeetingIds`). The sibling of `RecordingBadge`, used by the same surfaces.
 *
 * The lamp is lit but still: no pulse or spin, so it adds no animation (specs/0077 calm
 * motion) and costs nothing in Low Power Mode.
 */

import { LampDot } from '@/components/Transport/LampDot';

export function ProcessingBadge() {
  return (
    <span className="inline-flex flex-shrink-0 items-center gap-1.5">
      <LampDot tone="amber" label="Processing" decorative />
      <span className="u-section-label text-brand">Processing</span>
    </span>
  );
}
