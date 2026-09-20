/**
 * specs/0029 WS7.3 — auto-summary gate for the post-recording meeting-details visit.
 *
 * Post-stop navigation lands on /meeting-details?source=recording, where a summary is
 * auto-generated iff every gate passes. Each skip used to be a silent console.log — the
 * user asked for a feature that already existed but never visibly fired. This module is
 * the pure, testable decision: WHY auto-generation is skipped, and what (if anything) to
 * tell the user about it.
 *
 * Reason priority (first match wins):
 *   1. not-from-recording  — normal navigation; never surfaced to the user.
 *   2. deferred /
 *      deferred-processing — the deferred-meeting machinery owns this meeting
 *                            (1.10 feedback); nothing may auto-run here.
 *   3. disabled            — the `isAutoSummary` toggle is off (it defaults ON
 *                            since specs/0066 W1, so this is now a deliberate choice).
 *   4. empty-transcript    — nothing to summarize.
 *   5. no-model            — no LLM configured and the gemma3:1b fallback isn't installed.
 */

export type AutoSummarySkipReason =
  | 'not-from-recording'
  // 1.10 feedback: the meeting was recorded in deferred (record-only) mode and is
  // waiting for the backlog (AC return) or the manual "Process now" button. The
  // post-recording visit must NOT kick off transcribe-then-summarize itself —
  // that ran the whole pipeline on battery AND left the defer marker in place,
  // so the AC backlog re-ran it all a second time.
  // spec 0051 WS2: this marker is ALSO written by the stop path for a mid-meeting
  // go-live ('process-now') whose backlog handoff has not completed yet — 'defer'
  // means "needs processing", not only "recorded in low-power mode".
  | 'deferred'
  // The meeting was overridden live mid-recording ('process-now'): the stop path
  // already dispatched the full immediate pipeline (transcribe → diarize →
  // summarize). Skip silently — a second, parallel summary would race it.
  | 'deferred-processing'
  | 'disabled'
  | 'empty-transcript'
  | 'no-model'
  // Not derivable from AutoSummaryGateInput (the model probe threw); callers map their
  // catch block to this so the toast copy lives in one place.
  | 'model-check-failed';

export interface AutoSummaryGateInput {
  /**
   * The `source` query param on /meeting-details. Auto-summary fires for the two
   * flows that land here with a fresh transcript: post-recording (`'recording'`)
   * and audio import (`'import'`). Any other value (or null) is normal navigation.
   */
  source: string | null;
  /** The user's auto-summary toggle (ConfigContext `isAutoSummary`). */
  isAutoSummaryEnabled: boolean;
  /** Loaded transcript segment count for the meeting. */
  transcriptCount: number;
  /** True when a model is configured in the DB or the gemma3:1b fallback is available. */
  hasModelConfigured: boolean;
  /**
   * specs/0029 WS7.2: true when the meeting has recorded audio on disk awaiting
   * deferred transcription (record-only mode). Suppresses the 'empty-transcript'
   * skip — the summary flow transcribes first, then summarizes.
   */
  audioAwaitingTranscription?: boolean;
  /**
   * The meeting's `processing_mode` marker ('defer' | 'live' | null), from
   * `api_get_meeting_processing_mode`. Non-null means the deferred-meeting
   * machinery owns this meeting's processing — auto-summary must not run
   * (1.10 feedback). Absent/null (probe skipped or no marker) changes nothing.
   */
  processingMode?: string | null;
  /**
   * spec 0051 final review (Finding 1): true when the deferred backlog is currently
   * processing this meeting (queued or in a pipeline step — `isMeetingInFlight`).
   *
   * WS2 inverted the marker: a SUCCESSFUL 'process-now' stop writes `'defer'` and
   * hands the meeting to the backlog, which only clears the marker at the END of a
   * multi-minute pipeline. The post-stop visit lands two seconds later, so the marker
   * alone would classify a running pipeline as `'deferred'` and toast "this meeting
   * still needs processing" while the backlog pill shows it processing. The marker
   * says "needs processing"; only the backlog knows whether that is already happening.
   */
  isProcessingInBacklog?: boolean;
}

/** Returns the reason auto-generation must be skipped, or null to proceed. */
export function autoSummarySkipReason(
  input: AutoSummaryGateInput,
): AutoSummarySkipReason | null {
  if (input.source !== 'recording' && input.source !== 'import') return 'not-from-recording';
  // A 'defer' marker means "needs processing". If the backlog already has this meeting
  // in flight, that processing IS happening — skip silently (its pill shows progress)
  // instead of telling the user to press "Process now".
  if (input.processingMode === 'defer') {
    return input.isProcessingInBacklog ? 'deferred-processing' : 'deferred';
  }
  if (input.processingMode === 'live') return 'deferred-processing';
  if (!input.isAutoSummaryEnabled) return 'disabled';
  if (input.transcriptCount <= 0 && !input.audioAwaitingTranscription) {
    return 'empty-transcript';
  }
  if (!input.hasModelConfigured) return 'no-model';
  return null;
}

export interface AutoSummarySkipToast {
  title: string;
  description?: string;
}

/**
 * User-facing copy for a skip reason, or null when the skip should stay silent
 * ('not-from-recording' is just normal navigation — never toast it).
 */
export function autoSummarySkipToast(
  reason: AutoSummarySkipReason,
): AutoSummarySkipToast | null {
  switch (reason) {
    case 'not-from-recording':
      return null;
    case 'deferred':
      return {
        title: 'Processing deferred',
        description:
          'This meeting still needs processing. Use "Process now" on the transcript panel, or it will run automatically when back on power.',
      };
    case 'deferred-processing':
      // The immediate go-live pipeline is already running (its banner shows
      // progress) — an extra toast here would be noise.
      return null;
    case 'disabled':
      return {
        title: 'Auto-summary is off',
        description:
          'Turn on "Summarize automatically when a meeting ends" in Settings → Recording.',
      };
    case 'empty-transcript':
      return {
        title: 'Auto-summary skipped: transcript is empty',
        description: 'Nothing was transcribed for this meeting.',
      };
    case 'no-model':
      return {
        title: 'Auto-summary skipped: no AI model configured',
        description: 'Choose a summary model in Settings to enable auto-summaries.',
      };
    case 'model-check-failed':
      return {
        title: 'Auto-summary skipped',
        description: 'Could not check your summary model configuration.',
      };
  }
}
