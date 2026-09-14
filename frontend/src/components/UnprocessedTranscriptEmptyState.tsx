'use client';

interface UnprocessedTranscriptEmptyStateProps {
  /** "Process now" action; the button is omitted when absent (e.g. no meetingId). */
  onProcessNow?: () => void;
}

/** specs/0045 WS4 — transcript empty-state copy for a finished-but-unprocessed deferred
 *  recording (audio on disk, no transcript yet), replacing the generic "Welcome to
 *  Nixon!" copy. Extracted out of VirtualizedTranscriptView to stay under its file-size
 *  ratchet ceiling (scripts/check-file-size.sh). */
export function UnprocessedTranscriptEmptyState({ onProcessNow }: UnprocessedTranscriptEmptyStateProps) {
  return (
    <>
      <p className="font-display text-lg font-semibold">This recording hasn&apos;t been processed yet</p>
      <p className="text-xs mt-1">Transcribe, identify speakers, and summarize it now.</p>
      {onProcessNow && (
        <button
          type="button"
          onClick={onProcessNow}
          className="mt-3 inline-flex items-center rounded-lg bg-brand px-3 py-1.5 text-xs font-semibold text-brand-foreground hover:bg-brand/90"
        >
          Process now
        </button>
      )}
    </>
  );
}
