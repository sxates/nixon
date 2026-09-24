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
  /** The record page's start/stop transition (useRecordEmptyPhase). `'starting'` looks
   *  exactly like recording; `'saving'` says the meeting is being saved. */
  phase?: 'starting' | 'saving';
}

/**
 * The transcript panel's zero-segment empty state — extracted out of
 * VirtualizedTranscriptView to keep that file under the 800-line file-size cap
 * (specs/0042 WS6, now specs/0065; scripts/check-file-size.sh). Three mutually exclusive variants,
 * unchanged in behavior from their prior inline form:
 *  - recording (Listening… / paused)
 *  - unprocessed deferred recording (specs/0045 WS4)
 *  - no transcript yet
 * The idle copy was a "Welcome to Nixon!" heading until 2026-09-23: the record page showed
 * it for a moment after REC and again while a stopped meeting saved (owner feedback), so
 * the record page now passes its transition `phase` and the idle copy is plain.
 */
export function TranscriptEmptyState({ isRecording, isPaused, unprocessed, onProcessNow, phase }: TranscriptEmptyStateProps) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      className="text-center text-muted-foreground mt-8"
    >
      {isRecording || phase === 'starting' ? (
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
      ) : phase === 'saving' ? (
        <>
          <div className="flex items-center justify-center mb-3">
            <div className="w-3 h-3 rounded-full bg-muted-foreground"></div>
          </div>
          <p className="text-sm text-muted-foreground">Saving the recording…</p>
        </>
      ) : unprocessed ? (
        <UnprocessedTranscriptEmptyState onProcessNow={onProcessNow} />
      ) : (
        // Plain, because meeting details shares this state: "press REC" would be wrong
        // on a past meeting's transcript tab.
        <p className="text-sm text-muted-foreground">No transcript yet</p>
      )}
    </motion.div>
  );
}
