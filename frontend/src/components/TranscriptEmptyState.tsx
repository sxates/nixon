'use client';

import { motion } from 'framer-motion';
import { UnprocessedTranscriptEmptyState } from './UnprocessedTranscriptEmptyState';

interface TranscriptEmptyStateProps {
  /** Whether recording is in progress — shows the "Listening…" / paused copy. */
  isRecording: boolean;
  /** Whether recording is paused (only meaningful while `isRecording`). */
  isPaused: boolean;
  /** specs/0045 WS4 — a finished-but-unprocessed deferred recording (only meaningful
   *  when not recording): shows the "hasn't been processed yet" copy instead of the
   *  generic welcome copy. */
  unprocessed: boolean;
  /** specs/0045 WS4 — "Process now" action, forwarded to the unprocessed variant. */
  onProcessNow?: () => void;
}

/**
 * The transcript panel's zero-segment empty state — extracted out of
 * VirtualizedTranscriptView to keep that file under the 800-line file-size cap
 * (specs/0042 WS6, now specs/0065; scripts/check-file-size.sh). Three mutually exclusive variants,
 * unchanged in behavior from their prior inline form:
 *  - recording (Listening… / paused)
 *  - unprocessed deferred recording (specs/0045 WS4)
 *  - fresh meeting ("Welcome to Nixon!")
 */
export function TranscriptEmptyState({ isRecording, isPaused, unprocessed, onProcessNow }: TranscriptEmptyStateProps) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      className="text-center text-muted-foreground mt-8"
    >
      {isRecording ? (
        <>
          <div className="flex items-center justify-center mb-3">
            <div className={`w-3 h-3 rounded-full ${isPaused ? 'bg-muted-foreground' : 'bg-record animate-pulse'}`}></div>
          </div>
          <p className="text-sm text-muted-foreground">
            {isPaused ? 'Recording paused' : 'Listening for speech...'}
          </p>
          <p className="text-xs mt-1 text-muted-foreground">
            {isPaused ? 'Click resume to continue recording' : 'Speak to see live transcription'}
          </p>
        </>
      ) : unprocessed ? (
        <UnprocessedTranscriptEmptyState onProcessNow={onProcessNow} />
      ) : (
        <>
          <p className="font-display text-lg font-semibold">Welcome to Nixon!</p>
          <p className="text-xs mt-1">Start recording to see live transcription</p>
        </>
      )}
    </motion.div>
  );
}
