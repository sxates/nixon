'use client';

import { Button } from '@/components/ui/button';

interface UnprocessedTranscriptEmptyStateProps {
  /** "Process now" action; the button is omitted when absent (e.g. no meetingId). */
  onProcessNow?: () => void;
}

/** specs/0045 WS4 — transcript empty-state copy for a finished-but-unprocessed deferred
 *  recording (audio on disk, no transcript yet), replacing the generic "Welcome to
 *  Nixon!" copy. Extracted out of VirtualizedTranscriptView to stay under the 800-line
 *  file-size cap (scripts/check-file-size.sh). */
export function UnprocessedTranscriptEmptyState({ onProcessNow }: UnprocessedTranscriptEmptyStateProps) {
  return (
    <>
      <p className="font-display text-lg font-semibold">This recording hasn&apos;t been processed yet</p>
      <p className="text-xs mt-1">Transcribe, identify speakers, and summarize it now.</p>
      {onProcessNow && (
        <Button type="button" variant="brand" size="sm" onClick={onProcessNow} className="mt-3">
          Process now
        </Button>
      )}
    </>
  );
}
