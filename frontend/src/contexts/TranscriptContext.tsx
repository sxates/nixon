'use client';

import React, { createContext, useContext, useState, useEffect, useRef, useCallback, ReactNode, MutableRefObject } from 'react';
import { Transcript, TranscriptUpdate, LiveDiarizationUpdate } from '@/types';
import { toast } from 'sonner';
import { useRecordingState } from './RecordingStateContext';
import { transcriptService } from '@/services/transcriptService';
import { recordingService } from '@/services/recordingService';
import { indexedDBService } from '@/services/indexedDBService';
import { makeSafeUnlisten, safeListen } from '@/lib/safe-listen';
import { devLog } from '@/lib/dev-log';

/**
 * Running totals of audio segments lost during the current recording session
 * (specs/0028 backpressure hardening, surfaced in 0030 WS4).
 * - `dropped`: chunks shed because the bounded transcription queue saturated
 *   (transcription fell behind and audio was skipped to catch up).
 * - `skipped`: chunks that could not be transcribed (e.g. the speech model was
 *   unloaded) and were skipped after bounded retries.
 * Reset when a new recording starts. Non-zero counts mean the live transcript
 * has gaps, so the transcript view can hint at missing audio.
 */
export interface TranscriptGapCounts {
  dropped: number;
  skipped: number;
}

interface TranscriptContextType {
  transcripts: Transcript[];
  transcriptsRef: MutableRefObject<Transcript[]>
  addTranscript: (update: TranscriptUpdate) => void;
  copyTranscript: () => void;
  flushBuffer: () => void;
  meetingTitle: string;
  setMeetingTitle: (title: string) => void;
  clearTranscripts: () => void;
  currentMeetingId: string | null;
  markMeetingAsSaved: () => Promise<void>;
  transcriptGaps: TranscriptGapCounts;
}

// --- Backpressure event payloads (specs/0028; emitted from ---------------------------
// frontend/src-tauri/src/audio/transcription/worker.rs — keep in sync with the
// `serde_json::json!` emit sites there).

/** `transcription-falling-behind` — throttled live warning while the bounded work queue
 *  is saturated and chunks are being shed (`emit_falling_behind`). */
interface FallingBehindPayload {
  total_dropped: number;
  userMessage?: string;
}

/** `transcription-chunk-skipped` — throttled live warning that a chunk could not be
 *  transcribed and was skipped (`emit_chunk_skipped`). */
interface ChunkSkippedPayload {
  worker_id: number;
  chunk_id: number;
  reason: string;
  total_skipped: number;
  userMessage?: string;
}

/** `transcription-chunks-skipped` — accurate end-of-recording total of skipped chunks. */
interface ChunksSkippedSummaryPayload {
  chunks_skipped: number;
  reason?: string;
  userMessage?: string;
}

/** `transcription-chunks-dropped` — accurate end-of-recording total of chunks shed
 *  under backpressure. */
interface ChunksDroppedSummaryPayload {
  chunks_dropped: number;
  userMessage?: string;
}

const TranscriptContext = createContext<TranscriptContextType | undefined>(undefined);

/**
 * Map a live diarization display name to a stable color key (specs/0011, P3-B).
 *
 * Live events carry only the resolved display name ("You" / "Speaker 2"), not the
 * backend speaker key. We use the name itself as a stable per-session color key so
 * `speakerColorClass` gives each speaker a consistent accent, and special-case
 * "You" → "local" so the device owner gets the pinned local slot (matching the
 * post-meeting view). Returns null when there is no label yet.
 */
function liveColorKey(displayName: string | null | undefined): string | null {
  if (!displayName) return null;
  return displayName === 'You' ? 'local' : displayName;
}

export function TranscriptProvider({ children }: { children: ReactNode }) {
  const [transcripts, setTranscripts] = useState<Transcript[]>([]);
  const [meetingTitle, setMeetingTitle] = useState('+ New Call');
  const [currentMeetingId, setCurrentMeetingId] = useState<string | null>(null);
  // Audio segments lost during this recording (specs/0028 backpressure — see
  // TranscriptGapCounts). Drives the transcript panel's "may be missing audio" hint.
  const [transcriptGaps, setTranscriptGaps] = useState<TranscriptGapCounts>({
    dropped: 0,
    skipped: 0,
  });

  // Recording state context - provides backend-synced state
  const recordingState = useRecordingState();

  // Mirror currentMeetingId into a ref so the Tauri listener effects below can be mount-only
  // ([]) instead of tearing down and re-subscribing on every id change — churning the
  // subscription dropped early transcript events and reset the in-flight buffer (spec 0028,
  // Medium). The listeners read the *live* id through this ref, so per-transcript IndexedDB
  // saves still target the correct meeting.
  const currentMeetingIdRef = useRef<string | null>(currentMeetingId);
  useEffect(() => {
    currentMeetingIdRef.current = currentMeetingId;
  }, [currentMeetingId]);

  // Refs for transcript management
  const transcriptsRef = useRef<Transcript[]>(transcripts);
  const finalFlushRef = useRef<(() => void) | null>(null);

  // Keep ref updated with current transcripts
  useEffect(() => {
    transcriptsRef.current = transcripts;
  }, [transcripts]);

  // NOTE (specs/0029 WS4.1): this context used to own a second auto-scroll system (a
  // container ref force-scrolled to the bottom on every `transcripts` change). That
  // competed with the hysteresis-based follower in `useAutoScroll` and yanked the view
  // down while the user was reading. Scroll ownership now lives in ONE place:
  // `VirtualizedTranscriptView` + `useAutoScroll` (`lib/auto-scroll.ts`).

  // Initialize IndexedDB and listen for recording-started/stopped events
  useEffect(() => {
    let disposed = false;
    let unlistenRecordingStarted: (() => void) | undefined;
    let unlistenRecordingStopped: (() => void) | undefined;

    const setupRecordingListeners = async () => {
      try {
        // Initialize IndexedDB
        await indexedDBService.init();

        // Listen for recording-started event
        unlistenRecordingStarted = makeSafeUnlisten(await recordingService.onRecordingStarted(async () => {
          try {
            // Generate unique meeting ID
            const meetingId = `meeting-${Date.now()}`;
            setCurrentMeetingId(meetingId);

            // Fresh recording — no backpressure gaps yet (specs/0028).
            setTranscriptGaps({ dropped: 0, skipped: 0 });

            // Store in sessionStorage as fallback for markMeetingAsSaved
            sessionStorage.setItem('indexeddb_current_meeting_id', meetingId);
            console.log('[Recording Started] 💾 IndexedDB meeting ID stored:', meetingId);

            // Get meeting name
            const meetingName = await recordingService.getRecordingMeetingName();

            // Use a better fallback that matches the backend's naming pattern
            const effectiveTitle = meetingName || `Meeting ${new Date().toISOString().slice(0, 19).replace('T', '_').replace(/:/g, '-')}`;

            // Initialize meeting metadata in IndexedDB
            await indexedDBService.saveMeetingMetadata({
              meetingId,
              title: effectiveTitle,
              startTime: Date.now(),
              lastUpdated: Date.now(),
              transcriptCount: 0,
              savedToSQLite: false,
              folderPath: undefined // Will update shortly
            });

            // Synchronize meeting title to state (fixes tray stop title issue)
            setMeetingTitle(effectiveTitle);

            // Fetch folder path from backend and update metadata
            // This ensures folder path is persisted even if app crashes
            try {
              const { invoke } = await import('@tauri-apps/api/core');
              const folderPath = await invoke<string>('get_meeting_folder_path');
              if (folderPath) {
                const metadata = await indexedDBService.getMeetingMetadata(meetingId);
                if (metadata) {
                  metadata.folderPath = folderPath;
                  await indexedDBService.saveMeetingMetadata(metadata);
                }
              }
            } catch (_error) {
              // Non-fatal - will be set on stop if recording completes normally
            }
          } catch (error) {
            console.error('Failed to initialize meeting in IndexedDB:', error);
          }
        }));
        // If cleanup already ran before this resolved, tear it down immediately.
        if (disposed) unlistenRecordingStarted();

        // Listen for recording-stopped event
        unlistenRecordingStopped = makeSafeUnlisten(await recordingService.onRecordingStopped(async (payload) => {
          try {
            const activeMeetingId = currentMeetingIdRef.current;
            if (activeMeetingId) {
              // Update folder path in IndexedDB
              const metadata = await indexedDBService.getMeetingMetadata(activeMeetingId);

              if (metadata && payload.folder_path) {
                metadata.folderPath = payload.folder_path;
                await indexedDBService.saveMeetingMetadata(metadata);
              }
            }
          } catch (error) {
            console.error('Failed to update meeting metadata on stop:', error);
          }
        }));
        // If cleanup already ran before this resolved, tear it down immediately.
        if (disposed) unlistenRecordingStopped();
      } catch (error) {
        console.error('Failed to setup recording listeners:', error);
      }
    };

    setupRecordingListeners();

    return () => {
      disposed = true;
      // makeSafeUnlisten makes each of these once-guarded + crash-proof.
      if (unlistenRecordingStarted) {
        unlistenRecordingStarted();
        console.log('🧹 Recording started listener cleaned up');
      }
      if (unlistenRecordingStopped) {
        unlistenRecordingStopped();
        console.log('🧹 Recording stopped listener cleaned up');
      }
    };
    // Mount-only: the callbacks read the live meeting id via currentMeetingIdRef, so this
    // effect no longer re-subscribes on every id change (spec 0028, Medium).
  }, []);

  // Main transcript buffering logic with sequence_id ordering
  useEffect(() => {
    let disposed = false;
    let unlistenFn: (() => void) | undefined;
    let transcriptCounter = 0;
    const transcriptBuffer = new Map<number, Transcript>();
    let lastProcessedSequence = 0;
    let processingTimer: NodeJS.Timeout | undefined;

    const processBufferedTranscripts = (forceFlush = false) => {
      const sortedTranscripts: Transcript[] = [];

      // Process all available sequential transcripts
      let nextSequence = lastProcessedSequence + 1;
      while (transcriptBuffer.has(nextSequence)) {
        const bufferedTranscript = transcriptBuffer.get(nextSequence)!;
        sortedTranscripts.push(bufferedTranscript);
        transcriptBuffer.delete(nextSequence);
        lastProcessedSequence = nextSequence;
        nextSequence++;
      }

      // Add any buffered transcripts that might be out of order
      const now = Date.now();
      const staleThreshold = 100;  // 100ms safety net only (serial workers = sequential order)
      const recentThreshold = 0;    // Show immediately - no delay needed with serial processing
      const staleTranscripts: Transcript[] = [];
      const recentTranscripts: Transcript[] = [];
      const forceFlushTranscripts: Transcript[] = [];

      for (const [sequenceId, transcript] of transcriptBuffer.entries()) {
        if (forceFlush) {
          // Force flush mode: process ALL remaining transcripts regardless of timing
          forceFlushTranscripts.push(transcript);
          transcriptBuffer.delete(sequenceId);
          console.log(`Force flush: processing transcript with sequence_id ${sequenceId}`);
        } else {
          const transcriptAge = now - parseInt(transcript.id.split('-')[0]);
          if (transcriptAge > staleThreshold) {
            // Process stale transcripts (>100ms old - safety net)
            staleTranscripts.push(transcript);
            transcriptBuffer.delete(sequenceId);
          } else if (transcriptAge >= recentThreshold) {
            // Process immediately (0ms threshold with serial workers)
            recentTranscripts.push(transcript);
            transcriptBuffer.delete(sequenceId);
            console.log(`Processing transcript with sequence_id ${sequenceId}, age: ${transcriptAge}ms`);
          }
        }
      }

      // Sort both stale and recent transcripts by chunk_start_time, then by sequence_id
      const sortTranscripts = (transcripts: Transcript[]) => {
        return transcripts.sort((a, b) => {
          const chunkTimeDiff = (a.chunk_start_time || 0) - (b.chunk_start_time || 0);
          if (chunkTimeDiff !== 0) return chunkTimeDiff;
          return (a.sequence_id || 0) - (b.sequence_id || 0);
        });
      };

      const sortedStaleTranscripts = sortTranscripts(staleTranscripts);
      const sortedRecentTranscripts = sortTranscripts(recentTranscripts);
      const sortedForceFlushTranscripts = sortTranscripts(forceFlushTranscripts);

      const allNewTranscripts = [...sortedTranscripts, ...sortedRecentTranscripts, ...sortedStaleTranscripts, ...sortedForceFlushTranscripts];

      if (allNewTranscripts.length > 0) {
        setTranscripts(prev => {
          // Create a set of existing sequence_ids for deduplication
          const existingSequenceIds = new Set(prev.map(t => t.sequence_id).filter(id => id !== undefined));

          // Filter out any new transcripts that already exist
          const uniqueNewTranscripts = allNewTranscripts.filter(transcript =>
            transcript.sequence_id !== undefined && !existingSequenceIds.has(transcript.sequence_id)
          );

          // Only combine if we have unique new transcripts
          if (uniqueNewTranscripts.length === 0) {
            console.log('No unique transcripts to add - all were duplicates');
            return prev; // No new unique transcripts to add
          }

          console.log(`Adding ${uniqueNewTranscripts.length} unique transcripts out of ${allNewTranscripts.length} received`);

          // Merge with existing transcripts, maintaining chronological order
          const combined = [...prev, ...uniqueNewTranscripts];

          // Sort by chunk_start_time first, then by sequence_id
          return combined.sort((a, b) => {
            const chunkTimeDiff = (a.chunk_start_time || 0) - (b.chunk_start_time || 0);
            if (chunkTimeDiff !== 0) return chunkTimeDiff;
            return (a.sequence_id || 0) - (b.sequence_id || 0);
          });
        });

        // Log the processing summary
        const logMessage = forceFlush
          ? `Force flush processed ${allNewTranscripts.length} transcripts (${sortedTranscripts.length} sequential, ${forceFlushTranscripts.length} forced)`
          : `Processed ${allNewTranscripts.length} transcripts (${sortedTranscripts.length} sequential, ${recentTranscripts.length} recent, ${staleTranscripts.length} stale)`;
        console.log(logMessage);
      }
    };

    // Assign final flush function to ref for external access
    finalFlushRef.current = () => processBufferedTranscripts(true);

    const setupListener = async () => {
      try {
        console.log('🔥 Setting up MAIN transcript listener during component initialization...');
        unlistenFn = makeSafeUnlisten(await transcriptService.onTranscriptUpdate((update) => {
          const now = Date.now();
          devLog('🎯 MAIN LISTENER: Received transcript update:', {
            sequence_id: update.sequence_id,
            text: update.text.substring(0, 50) + '...',
            timestamp: update.timestamp,
            is_partial: update.is_partial,
            received_at: new Date(now).toISOString(),
            buffer_size_before: transcriptBuffer.size
          });

          // Check for duplicate sequence_id before processing
          if (transcriptBuffer.has(update.sequence_id)) {
            console.log('🚫 MAIN LISTENER: Duplicate sequence_id, skipping buffer:', update.sequence_id);
            return;
          }

          // Create transcript for buffer with NEW timestamp fields
          const newTranscript: Transcript = {
            id: `${Date.now()}-${transcriptCounter++}`,
            text: update.text,
            timestamp: update.timestamp,
            sequence_id: update.sequence_id,
            chunk_start_time: update.chunk_start_time,
            is_partial: update.is_partial,
            confidence: update.confidence,
            // NEW: Recording-relative timestamps for playback sync
            audio_start_time: update.audio_start_time,
            audio_end_time: update.audio_end_time,
            duration: update.duration,
            // Live speaker diarization (specs/0011, P3-B): the resolved display
            // name when this segment was already labeled by the time it was
            // emitted. We render it through the same label/color path as the
            // post-meeting view — `speaker_name` is the shown label and `speaker`
            // is the color key (live carries only the name, so the name doubles as
            // a stable per-session color key; "You" maps to the pinned local slot).
            // null until a pass labels it (patched later via
            // `live-diarization-update`), and always null when live diarization is off.
            //
            // specs/0029 WS3.4: when there is no live-diarization label, a
            // microphone-tagged segment shows "You" from the cheap capture-channel
            // tag instead (live diarization is off by default; the channel tag is
            // free). The offline pass at stop remains the authoritative labeler.
            speaker: liveColorKey(
              update.speaker ?? (update.channel === 'microphone' ? 'You' : null)
            ),
            speaker_name:
              update.speaker ?? (update.channel === 'microphone' ? 'You' : null),
            // Carried through to the save payload → persisted as transcripts.channel.
            channel: update.channel ?? null,
            // specs/0055: window-resolution runs for the render-time split. Not
            // persisted — the backend ignores unknown fields on save, and the
            // authoritative offline pass recomputes this from the channel WAVs.
            channel_runs: update.channel_runs,
          };

          // Add to buffer
          transcriptBuffer.set(update.sequence_id, newTranscript);
          console.log(`✅ MAIN LISTENER: Buffered transcript with sequence_id ${update.sequence_id}. Buffer size: ${transcriptBuffer.size}, Last processed: ${lastProcessedSequence}`);

          // Save to IndexedDB (non-blocking). Read the live id from the ref so this listener
          // can stay mount-only without dropping the per-transcript save.
          const activeMeetingId = currentMeetingIdRef.current;
          if (activeMeetingId) {
            indexedDBService.saveTranscript(activeMeetingId, update)
              .catch(err => console.warn('IndexedDB save failed:', err));
          }

          // Clear any existing timer and set a new one
          if (processingTimer) {
            clearTimeout(processingTimer);
          }

          // Process buffer with minimal delay for immediate UI updates (serial workers = sequential order)
          processingTimer = setTimeout(processBufferedTranscripts, 10);
        }));
        // If cleanup already ran before this resolved, tear it down immediately.
        if (disposed) unlistenFn();
        console.log('✅ MAIN transcript listener setup complete');
      } catch (error) {
        console.error('❌ Failed to setup MAIN transcript listener:', error);
        toast.error('Could not start live transcription', {
          description: 'The transcript listener failed to initialize. Check the console for details.',
        });
      }
    };

    setupListener();
    console.log('Started enhanced listener setup');

    return () => {
      console.log('🧹 CLEANUP: Cleaning up MAIN transcript listener...');
      disposed = true;
      if (processingTimer) {
        clearTimeout(processingTimer);
        console.log('🧹 CLEANUP: Cleared processing timer');
      }
      if (unlistenFn) {
        unlistenFn();
        console.log('🧹 CLEANUP: MAIN transcript listener cleaned up');
      }
    };
    // Mount-only: the transcript callback reads the live meeting id via currentMeetingIdRef,
    // so the listener is set up once and never churns on id changes (spec 0028, Medium).
  }, []);

  // Live speaker diarization retroactive labels (specs/0011, P3-B).
  //
  // The backend emits `live-diarization-update` carrying display-name labels for
  // already-emitted live segments that a later diarization pass first resolved
  // (the common case: a segment arrived via `transcript-update` with `speaker:
  // null`, and the next pass labels it). We patch those rows in place, correlating
  // each `segment_id` (the transcript `sequence_id` as a string) against our
  // `sequence_id`.
  //
  // The backend guarantees "stable-once-shown": a label, once sent for a segment,
  // never changes during the live session — only `null → labeled` transitions are
  // sent. So we apply each update as a one-way set and never overwrite an existing
  // label (the authoritative offline pass at stop is what reconciles, separately).
  useEffect(() => {
    const dispose = safeListen<LiveDiarizationUpdate>(
      'live-diarization-update',
      (event) => {
        const { segments } = event.payload;
        if (!segments || segments.length === 0) return;

        // Build a sequence_id -> display name map for this batch.
        const labelBySequence = new Map<number, string>();
        for (const seg of segments) {
          const seq = parseInt(seg.segment_id, 10);
          if (!Number.isNaN(seq)) labelBySequence.set(seq, seg.speaker);
        }
        if (labelBySequence.size === 0) return;

        setTranscripts((prev) => {
          let changed = false;
          const next = prev.map((t) => {
            if (t.sequence_id === undefined) return t;
            const label = labelBySequence.get(t.sequence_id);
            // Only set when newly labeled; never overwrite (stable-once-shown).
            if (label === undefined || t.speaker_name) return t;
            changed = true;
            return { ...t, speaker: liveColorKey(label), speaker_name: label };
          });
          return changed ? next : prev;
        });
      },
    );

    return () => {
      dispose();
    };
  }, []);

  // Backend backpressure warnings (spec 0028, wired in 0030 WS4). The audio/transcription
  // pipeline emits four events when audio can't be transcribed:
  //   - `transcription-falling-behind` (live, backend-throttled): the bounded work queue
  //     saturated and chunks are being shed to stay responsive.
  //   - `transcription-chunk-skipped` (live, backend-throttled): a chunk couldn't be
  //     transcribed (e.g. speech model unloaded) and was skipped.
  //   - `transcription-chunks-dropped` / `transcription-chunks-skipped` (once, at stop):
  //     accurate end-of-recording totals for each condition.
  // We coalesce each condition into ONE sonner toast (fixed id — bursts update it in place
  // instead of stacking) with an additional 30s re-notify throttle for the live events, and
  // track running totals in `transcriptGaps` so the transcript view can show a persistent
  // "audio may be missing" hint. The backend payload counts are running totals, so we take
  // max() rather than summing.
  useEffect(() => {
    const DROPPED_TOAST_ID = 'transcription-chunks-dropped';
    const SKIPPED_TOAST_ID = 'transcription-chunks-skipped';
    const LIVE_TOAST_THROTTLE_MS = 30000;
    let lastDroppedToastAt = 0;
    let lastSkippedToastAt = 0;

    const segments = (n: number) => `${n} audio segment${n === 1 ? '' : 's'}`;

    const recordDropped = (total: unknown) => {
      if (typeof total !== 'number' || total <= 0) return 0;
      setTranscriptGaps((prev) =>
        total > prev.dropped ? { ...prev, dropped: total } : prev
      );
      return total;
    };

    const recordSkipped = (total: unknown) => {
      if (typeof total !== 'number' || total <= 0) return 0;
      setTranscriptGaps((prev) =>
        total > prev.skipped ? { ...prev, skipped: total } : prev
      );
      return total;
    };

    const disposers = [
      safeListen<FallingBehindPayload | undefined>(
        'transcription-falling-behind',
        (event) => {
          const dropped = recordDropped(event?.payload?.total_dropped);
          const now = Date.now();
          if (now - lastDroppedToastAt < LIVE_TOAST_THROTTLE_MS) return;
          lastDroppedToastAt = now;
          toast.warning('Transcription is behind', {
            id: DROPPED_TOAST_ID,
            description:
              dropped > 0
                ? `Audio is arriving faster than it can be transcribed — ${segments(dropped)} skipped so far to catch up.`
                : 'Audio is arriving faster than it can be transcribed — some audio is being skipped to catch up.',
            duration: 6000,
          });
        },
      ),
      safeListen<ChunkSkippedPayload | undefined>(
        'transcription-chunk-skipped',
        (event) => {
          const skipped = recordSkipped(event?.payload?.total_skipped);
          const now = Date.now();
          if (now - lastSkippedToastAt < LIVE_TOAST_THROTTLE_MS) return;
          lastSkippedToastAt = now;
          toast.warning('Some audio could not be transcribed', {
            id: SKIPPED_TOAST_ID,
            description:
              skipped > 0
                ? `The speech model was unavailable, so ${segments(skipped)} ${skipped === 1 ? 'was' : 'were'} skipped so far.`
                : 'The speech model was unavailable, so some audio was skipped.',
            duration: 6000,
          });
        },
      ),
      // End-of-recording totals: always shown (no throttle) with the accurate final
      // count; the shared toast ids replace any still-visible live warning.
      safeListen<ChunksDroppedSummaryPayload | undefined>(
        'transcription-chunks-dropped',
        (event) => {
          const n = Math.max(recordDropped(event?.payload?.chunks_dropped), 1);
          lastDroppedToastAt = Date.now();
          toast.warning('Transcription fell behind during this recording', {
            id: DROPPED_TOAST_ID,
            description: `${segments(n)} ${n === 1 ? 'was' : 'were'} skipped to catch up and ${n === 1 ? 'is' : 'are'} missing from the transcript.`,
            duration: 10000,
          });
        },
      ),
      safeListen<ChunksSkippedSummaryPayload | undefined>(
        'transcription-chunks-skipped',
        (event) => {
          const n = Math.max(recordSkipped(event?.payload?.chunks_skipped), 1);
          lastSkippedToastAt = Date.now();
          toast.warning('Some audio could not be transcribed', {
            id: SKIPPED_TOAST_ID,
            description: `${segments(n)} could not be transcribed because the speech model was unavailable, and ${n === 1 ? 'is' : 'are'} missing from the transcript.`,
            duration: 10000,
          });
        },
      ),
    ];
    return () => disposers.forEach((dispose) => dispose());
  }, []);

  // Sync transcript history and meeting name from backend on reload
  // This fixes the issue where reloading during active recording causes state desync
  useEffect(() => {
    const syncFromBackend = async () => {
      // If recording is active and we have no local transcripts, sync from backend
      if (recordingState.isRecording && transcripts.length === 0) {
        try {
          console.log('[Reload Sync] Recording active after reload, syncing transcript history...');

          // Fetch transcript history from backend
          const history = await transcriptService.getTranscriptHistory();
          console.log(`[Reload Sync] Retrieved ${history.length} transcript segments from backend`);

          // Convert backend format to frontend Transcript format
          const formattedTranscripts: Transcript[] = history.map((segment: any) => ({
            id: segment.id,
            text: segment.text,
            timestamp: segment.display_time, // Use display_time for UI
            sequence_id: segment.sequence_id,
            chunk_start_time: segment.audio_start_time,
            is_partial: false, // History segments are always final
            confidence: segment.confidence,
            audio_start_time: segment.audio_start_time,
            audio_end_time: segment.audio_end_time,
            duration: segment.duration,
            // specs/0029 WS3.4 follow-up: the backend rehydration copy now carries
            // the capture-channel tag, so a mid-recording reload keeps channel
            // attribution (persisted as transcripts.channel at save) and the live
            // "You" label for microphone-tagged segments — same derivation as the
            // live transcript-update path above.
            speaker: liveColorKey(segment.channel === 'microphone' ? 'You' : null),
            speaker_name: segment.channel === 'microphone' ? 'You' : null,
            channel: segment.channel ?? null,
          }));

          setTranscripts(formattedTranscripts);
          console.log('[Reload Sync] ✅ Transcript history synced successfully');

          // Fetch meeting name from backend
          const meetingName = await recordingService.getRecordingMeetingName();
          if (meetingName) {
            console.log('[Reload Sync] Retrieved meeting name:', meetingName);
            setMeetingTitle(meetingName);
            console.log('[Reload Sync] ✅ Meeting title synced successfully');
          }
        } catch (error) {
          console.error('[Reload Sync] Failed to sync from backend:', error);
        }
      }
    };

    syncFromBackend();
  }, [recordingState.isRecording]); // Run when recording state changes

  // Manual transcript update handler (for RecordingControls component)
  const addTranscript = useCallback((update: TranscriptUpdate) => {
    devLog('🎯 addTranscript called with:', {
      sequence_id: update.sequence_id,
      text: update.text.substring(0, 50) + '...',
      timestamp: update.timestamp,
      is_partial: update.is_partial
    });

    const newTranscript: Transcript = {
      id: update.sequence_id ? update.sequence_id.toString() : Date.now().toString(),
      text: update.text,
      timestamp: update.timestamp,
      sequence_id: update.sequence_id || 0,
      chunk_start_time: update.chunk_start_time,
      is_partial: update.is_partial,
      confidence: update.confidence,
      audio_start_time: update.audio_start_time,
      audio_end_time: update.audio_end_time,
      duration: update.duration,
    };

    setTranscripts(prev => {
      console.log('📊 Current transcripts count before update:', prev.length);

      // Check if this transcript already exists
      const exists = prev.some(
        t => t.text === update.text && t.timestamp === update.timestamp
      );
      if (exists) {
        devLog('🚫 Duplicate transcript detected, skipping:', update.text.substring(0, 30) + '...');
        return prev;
      }

      // Add new transcript and sort by sequence_id to maintain order
      const updated = [...prev, newTranscript];
      const sorted = updated.sort((a, b) => (a.sequence_id || 0) - (b.sequence_id || 0));

      console.log('✅ Added new transcript. New count:', sorted.length);
      devLog('📝 Latest transcript:', {
        id: newTranscript.id,
        text: newTranscript.text.substring(0, 30) + '...',
        sequence_id: newTranscript.sequence_id
      });

      return sorted;
    });
  }, []);

  // Copy transcript to clipboard with recording-relative timestamps
  const copyTranscript = useCallback(() => {
    // Format timestamps as recording-relative [MM:SS] instead of wall-clock time
    const formatTime = (seconds: number | undefined): string => {
      if (seconds === undefined) return '[--:--]';
      const totalSecs = Math.floor(seconds);
      const mins = Math.floor(totalSecs / 60);
      const secs = totalSecs % 60;
      return `[${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}]`;
    };

    const fullTranscript = transcripts
      .map(t => `${formatTime(t.audio_start_time)} ${t.text}`)
      .join('\n');
    navigator.clipboard.writeText(fullTranscript);

    toast.success("Transcript copied to clipboard");
  }, [transcripts]);

  // Force flush buffer (for final transcript processing)
  const flushBuffer = useCallback(() => {
    if (finalFlushRef.current) {
      console.log('🔄 Flushing transcript buffer...');
      finalFlushRef.current();
    }
  }, []);

  // Clear transcripts (used when starting new recording)
  const clearTranscripts = useCallback(() => {
    setTranscripts([]);
    // A new session starts gap-free (also reset by the recording-started listener).
    setTranscriptGaps({ dropped: 0, skipped: 0 });
    // Don't clear currentMeetingId here - it will be set by recording-started event
  }, []);

  // Mark current meeting as saved in IndexedDB
  const markMeetingAsSaved = useCallback(async () => {
    // Try context state first, fallback to sessionStorage
    const meetingId = currentMeetingId || sessionStorage.getItem('indexeddb_current_meeting_id');

    if (!meetingId) {
      console.error('[IndexedDB] ❌ Cannot mark meeting as saved: No meeting ID available!');
      console.error('[IndexedDB] currentMeetingId:', currentMeetingId);
      console.error('[IndexedDB] sessionStorage:', sessionStorage.getItem('indexeddb_current_meeting_id'));
      return;
    }

    try {
      await indexedDBService.markMeetingSaved(meetingId);

      // Clear both sources
      setCurrentMeetingId(null);
      sessionStorage.removeItem('indexeddb_current_meeting_id');
    } catch (error) {
      console.error('[IndexedDB] ❌ Failed to mark meeting as saved:', error);
    }
  }, [currentMeetingId]);

  const value: TranscriptContextType = {
    transcripts,
    transcriptsRef,
    addTranscript,
    copyTranscript,
    flushBuffer,
    meetingTitle,
    setMeetingTitle,
    clearTranscripts,
    currentMeetingId,
    markMeetingAsSaved,
    transcriptGaps,
  };

  return (
    <TranscriptContext.Provider value={value}>
      {children}
    </TranscriptContext.Provider>
  );
}

export function useTranscripts() {
  const context = useContext(TranscriptContext);
  if (context === undefined) {
    throw new Error('useTranscripts must be used within a TranscriptProvider');
  }
  return context;
}
