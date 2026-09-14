/**
 * Deferred transcription helpers (specs/0029 WS7.2 — record-only mode).
 *
 * A meeting recorded with live transcription OFF has audio on disk but no (or
 * only stray) transcript rows. Two surfaces act on that condition:
 *  - meeting-details shows a "Transcribe now" affordance (TranscriptButtonGroup);
 *  - summary generation runs transcription first, then summarizes
 *    (useSummaryGeneration).
 *
 * Pure logic lives here so it is unit-testable without Tauri.
 */

/**
 * Below this many transcript segments a meeting counts as "not really
 * transcribed". Mirrors the backend retention sweep's exemption threshold
 * (`retention::MIN_TRANSCRIPT_SEGMENTS`), which keeps exactly these meetings'
 * audio from being deleted so it can still be transcribed later.
 */
export const SPARSE_TRANSCRIPT_SEGMENTS = 3;

/**
 * Should we run (deferred) transcription before generating a summary?
 *
 * True only when the transcript is absent/sparse AND we positively know audio
 * exists on disk. `audioAvailable === null` (probe pending or failed) does NOT
 * trigger transcription — the summary flow then behaves exactly as before
 * (notes-grounded fallback or a friendly bail-out).
 */
export function needsTranscriptionBeforeSummary(
  transcriptCount: number,
  audioAvailable: boolean | null,
): boolean {
  return transcriptCount < SPARSE_TRANSCRIPT_SEGMENTS && audioAvailable === true;
}

/**
 * Map the configured transcript provider (ConfigContext `transcriptModelConfig`,
 * where local Whisper is stored as 'localWhisper') to the provider string the
 * `start_retranscription_command` backend expects ('whisper' | 'parakeet').
 * Unknown/cloud providers return null → the backend defaults to Whisper with
 * the model saved in transcript settings.
 */
export function retranscriptionProviderFor(
  configProvider: string | null | undefined,
): string | null {
  if (configProvider === 'parakeet') return 'parakeet';
  if (configProvider === 'localWhisper' || configProvider === 'whisper') return 'whisper';
  return null;
}
