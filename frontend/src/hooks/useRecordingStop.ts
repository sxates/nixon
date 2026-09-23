import { useEffect, useCallback, useRef } from 'react';
import { useRouter } from 'next/navigation';
import { listen } from '@tauri-apps/api/event';
import { safeListen, makeSafeUnlisten } from '@/lib/safe-listen';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { storageService } from '@/services/storageService';
import { transcriptService } from '@/services/transcriptService';
import { consumeStopRecordingResult } from '@/lib/recording-stop';
import { canDiscardMeeting } from '@/lib/discard-meeting';
import {
  markSessionDeferred,
  sessionStartedDeferred,
  sessionEverDeferred,
  stopAction,
  type StopAction,
} from '@/lib/processing-mode';
import { stopFollowUp, type HandoffOutcome } from '@/lib/stop-processing-handoff';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import {
  applyPinnedSummaryLanguageToMeeting,
  detectAndCacheSummaryLanguage,
} from '@/lib/summary-language-preferences';

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';

/** Sentinel meeting id used by SidebarProvider for the "+ New Call" placeholder (no DB row). */
const PLACEHOLDER_MEETING_ID = 'intro-call';

interface UseRecordingStopReturn {
  handleRecordingStop: (callApi: boolean) => Promise<void>;
  isStopping: boolean;
  isProcessingTranscript: boolean;
  isSavingTranscript: boolean;
  summaryStatus: SummaryStatus;
  setIsStopping: (value: boolean) => void;
}

/**
 * Custom hook for managing recording stop lifecycle.
 * Handles the complex stop sequence: transcription wait → buffer flush → SQLite save → navigation.
 *
 * Features:
 * - Transcription completion polling (60s max, 500ms interval)
 * - Transcript buffer flush coordination
 * - SQLite meeting save with folder_path from sessionStorage
 * - Auto-navigation to meeting details
 * - Toast notifications for success/error
 * - Window exposure for Rust callbacks
 */
export function useRecordingStop(
  setIsRecording: (value: boolean) => void,
  setIsRecordingDisabled: (value: boolean) => void
): UseRecordingStopReturn {
  // USE global state instead
  const recordingState = useRecordingState();
  const {
    status,
    setStatus,
    isStopping,
    isProcessing: isProcessingTranscript,
    isSaving: isSavingTranscript
  } = recordingState;

  const {
    transcriptsRef,
    flushBuffer,
    clearTranscripts,
    meetingTitle,
    markMeetingAsSaved,
  } = useTranscripts();

  const {
    refetchMeetings,
    setCurrentMeeting,
    setMeetings,
    meetings,
    setIsMeetingActive,
    currentMeeting,
    activeRecordingMeetingId,
    setActiveRecordingMeetingId,
  } = useSidebar();

  const { enqueueMeeting } = useBacklog();

  const router = useRouter();

  // Guard to prevent duplicate/concurrent stop calls (e.g., from UI and tray simultaneously)
  const stopInProgressRef = useRef(false);
  // A completed stop hands the transcript clear to the recorder's unmount (see below).
  const clearOnLeaveRef = useRef(false);

  // Deferred promise that resolves with the `recording-stopped` event payload.
  //
  // The backend emits `recording-stopped` (carrying the authoritative `folder_path`)
  // asynchronously around the `stop_recording` command, so the event can arrive AFTER
  // `handleRecordingStop` has already started. To avoid persisting `folder_path = NULL`
  // when that ordering happens (a confirmed intermittent race), the save path awaits this
  // promise so it always sees the event payload before reading the folder path. The
  // listener resolves it when the event arrives; the ref always holds a *pending* promise
  // between recordings so awaiting it is safe regardless of ordering.
  const recordingStoppedDataRef = useRef<{
    promise: Promise<void>;
    resolve: () => void;
    settled: boolean;
  } | null>(null);

  const ensureRecordingStoppedDeferred = useCallback(() => {
    if (!recordingStoppedDataRef.current || recordingStoppedDataRef.current.settled) {
      let resolveFn: () => void = () => {};
      const promise = new Promise<void>((resolve) => {
        resolveFn = resolve;
      });
      recordingStoppedDataRef.current = { promise, resolve: resolveFn, settled: false };
    }
    return recordingStoppedDataRef.current;
  }, []);

  // Set up recording-stopped listener: capture the authoritative folder_path/meeting_name
  // from the event payload and resolve the deferred promise the save path awaits.
  useEffect(() => {
    console.log('Setting up recording-stopped listener for navigation...');
    return safeListen<{
      message: string;
      folder_path?: string;
      meeting_name?: string;
      resumed?: boolean;
      prior_audio_duration_seconds?: number;
    }>('recording-stopped', (event) => {
      const { folder_path, meeting_name, resumed, prior_audio_duration_seconds } = event.payload;

      // Store folder_path and meeting_name for later use in handleRecordingStop.
      if (folder_path) {
        sessionStorage.setItem('last_recording_folder_path', folder_path);
      }
      if (meeting_name) {
        sessionStorage.setItem('last_recording_meeting_name', meeting_name);
      }
      // specs/0037 — a resumed session's stop must APPEND to the same meeting (not the
      // guarded new-row save). Stash the flag + the prior audio duration (the offset the
      // appended transcripts get shifted by) alongside folder_path so the save path — which
      // may start before this event arrives — reads them after awaiting the deferred below.
      sessionStorage.setItem('last_recording_resumed', resumed ? 'true' : 'false');
      sessionStorage.setItem(
        'last_recording_prior_audio_duration',
        String(prior_audio_duration_seconds ?? 0),
      );

      // Resolve the deferred promise so an already-running stop can proceed with the
      // payload it was waiting for.
      const deferred = ensureRecordingStoppedDeferred();
      deferred.settled = true;
      deferred.resolve();
    });
  }, [router, ensureRecordingStoppedDeferred]);

  // Main recording stop handler
  const handleRecordingStop = useCallback(async (isCallApi: boolean) => {
    // Make sure there's a pending deferred to await even if the `recording-stopped`
    // event hasn't been delivered yet — the listener will resolve it on arrival.
    const stoppedDeferred = ensureRecordingStoppedDeferred();

    // Guard: prevent duplicate/concurrent stop calls
    if (stopInProgressRef.current) {
      return;
    }
    stopInProgressRef.current = true;

    // specs/0037 review-2 (FIX C): the stop invoke's RETURN VALUE — captured in memory by
    // recordingService.stopRecording — is the PRIMARY source for the resumed flag, the
    // prior-audio-duration offset, and folder_path at save time. The `recording-stopped`
    // EVENT is emitted only after stop_and_save (multi-segment stops run several blocking
    // ffmpeg concats), so on slow stops it can lose the bounded wait below: a late event
    // read resumed=false and saved a resumed session into a DUPLICATE meeting. Consumed
    // (read-and-clear) here, BEFORE any await, so a subsequent start's stale-key cleanup
    // can't wipe it out from under this in-flight save. Null when the stop ran in Rust
    // (tray) or the page reloaded mid-stop — those fall back to the event transport.
    const stopResult = consumeStopRecordingResult();

    // Set status to STOPPING immediately
    setStatus(RecordingStatus.STOPPING);
    setIsRecording(false);
    setIsRecordingDisabled(true);
    const stopStartTime = Date.now();

    try {
      console.log('Post-stop processing (new implementation)...', {
        stop_initiated_at: new Date(stopStartTime).toISOString(),
        current_transcript_count: transcriptsRef.current.length
      });

      // Note: stop_recording is already called by RecordingControls.stopRecordingAction
      // This function only handles post-stop processing (transcription wait, API call, navigation)
      console.log('Recording already stopped by RecordingControls, processing transcription...');

      // Wait for transcription to complete
      setStatus(RecordingStatus.PROCESSING_TRANSCRIPTS, 'Waiting for transcription...');
      console.log('Waiting for transcription to complete...');

      const MAX_WAIT_TIME = 60000; // 60 seconds maximum wait (increased for longer processing)
      const POLL_INTERVAL = 500; // Check every 500ms
      let elapsedTime = 0;
      let transcriptionComplete = false;

      // Listen for transcription-complete event
      const unlistenComplete = makeSafeUnlisten(await listen('transcription-complete', () => {
        console.log('Received transcription-complete event');
        transcriptionComplete = true;
      }));

      // Poll for transcription status
      while (elapsedTime < MAX_WAIT_TIME && !transcriptionComplete) {
        try {
          const status = await transcriptService.getTranscriptionStatus();
          console.log('Transcription status:', status);

          // Check if transcription is complete
          if (!status.is_processing && status.chunks_in_queue === 0) {
            console.log('Transcription complete - no active processing and no chunks in queue');
            transcriptionComplete = true;
            break;
          }

          // If no activity for more than 8 seconds and no chunks in queue, consider it done (increased from 5s to 8s)
          if (status.last_activity_ms > 8000 && status.chunks_in_queue === 0) {
            console.log('Transcription likely complete - no recent activity and empty queue');
            transcriptionComplete = true;
            break;
          }

          // Update user with current status
          if (status.chunks_in_queue > 0) {
            console.log(`Processing ${status.chunks_in_queue} remaining audio chunks...`);
            setStatus(RecordingStatus.PROCESSING_TRANSCRIPTS, `Processing ${status.chunks_in_queue} remaining chunks...`);
          }

          // Wait before next check
          await new Promise(resolve => setTimeout(resolve, POLL_INTERVAL));
          elapsedTime += POLL_INTERVAL;
        } catch (error) {
          console.error('Error checking transcription status:', error);
          break;
        }
      }

      // Clean up listener
      console.log('🧹 CLEANUP: Cleaning up transcription-complete listener');
      unlistenComplete();

      // Whether the poll loop gave up waiting for transcription to finish. On timeout we do
      // NOT discard the meeting — we still persist whatever transcripts have arrived (below)
      // and warn the user, so a stuck/slow transcription can never silently drop a meeting.
      const transcriptionTimedOut = !transcriptionComplete && elapsedTime >= MAX_WAIT_TIME;
      if (transcriptionTimedOut) {
        console.warn('⏰ Transcription wait timeout reached after', elapsedTime, 'ms');
      } else {
        console.log('✅ Transcription completed after', elapsedTime, 'ms');
        // Wait longer for any late transcript segments (increased from 1s to 4s)
        console.log('⏳ Waiting for late transcript segments...');
        await new Promise(resolve => setTimeout(resolve, 4000));
      }

      // Final buffer flush: process ALL remaining transcripts regardless of timing
      const flushStartTime = Date.now();
      console.log('🔄 Final buffer flush: forcing processing of any remaining transcripts...', {
        flush_started_at: new Date(flushStartTime).toISOString(),
        time_since_stop: flushStartTime - stopStartTime,
        current_transcript_count: transcriptsRef.current.length
      });
      setStatus(RecordingStatus.PROCESSING_TRANSCRIPTS, 'Flushing transcript buffer...');
      flushBuffer();
      const flushEndTime = Date.now();
      console.log('✅ Final buffer flush completed', {
        flush_duration: flushEndTime - flushStartTime,
        total_time_since_stop: flushEndTime - stopStartTime,
        final_transcript_count: transcriptsRef.current.length
      });

      // NOTE: Status remains PROCESSING_TRANSCRIPTS until we start saving

      // Wait a bit more to ensure all transcript state updates have been processed
      console.log('Waiting for transcript state updates to complete...');
      await new Promise(resolve => setTimeout(resolve, 500));

      // Save to SQLite
      // NOTE: enabled to save COMPLETE transcripts after frontend receives all updates
      // This ensures user sees all transcripts streaming in before database save.
      //
      // Persistence is intentionally NOT gated on `transcriptionComplete`: a transcription
      // timeout (or a backend that never emits `transcription-complete`) previously skipped
      // this entire branch, jumped to IDLE, and silently discarded the meeting + orphaned the
      // placeholder row (spec 0028, Critical). We now persist whatever transcripts exist on
      // any terminal path and only truly skip when the caller opted out (`isCallApi === false`).
      // The empty-recording case is still handled by the abandoned-cleanup block below.
      if (isCallApi) {

        setStatus(RecordingStatus.SAVING, 'Saving meeting to database...');

        // Get fresh transcript state (ALL transcripts including late ones)
        const freshTranscripts = [...transcriptsRef.current];

        // Resolve the save-routing inputs. PRIMARY: the stop invoke's return value
        // (`stopResult`, consumed above — race-free). FALLBACK: the `recording-stopped`
        // event payload via sessionStorage — only then do we wait (bounded) for the
        // event's deferred, which was the root-cause fix for `folder_path` persisting
        // as NULL before the return-value transport existed.
        if (!stopResult || !stopResult.folder_path) {
          try {
            await Promise.race([
              stoppedDeferred.promise,
              new Promise<void>((resolve) => setTimeout(resolve, 5000)),
            ]);
          } catch (error) {
            console.warn('Error waiting for recording-stopped payload:', error);
          }
        }

        let folderPath =
          stopResult?.folder_path ?? sessionStorage.getItem('last_recording_folder_path');
        const savedMeetingName =
          stopResult?.meeting_name ?? sessionStorage.getItem('last_recording_meeting_name');

        // specs/0037 — resumed-session routing. From the invoke result when available;
        // otherwise from the event-written sessionStorage keys (never mixed — a present
        // result is authoritative, so stale keys can't misroute the save). Absent
        // everywhere => a normal stop (resumed=false, offset=0), the save is unchanged.
        const resumed = stopResult
          ? stopResult.resumed
          : sessionStorage.getItem('last_recording_resumed') === 'true';
        const priorAudioDuration = stopResult
          ? stopResult.prior_audio_duration_seconds
          : parseFloat(sessionStorage.getItem('last_recording_prior_audio_duration') ?? '0');
        const audioOffsetSeconds = Number.isFinite(priorAudioDuration) ? priorAudioDuration : 0;

        // Last-resort fallback: if the event never carried a folder_path, ask the
        // backend directly. Note the recording manager may already be torn down by
        // save time, so this can return empty — the event payload above is the
        // reliable source. Best-effort only.
        if (!folderPath) {
          console.warn('folder_path missing from recording-stopped payload; falling back to get_meeting_folder_path');
          try {
            const fetched = await invoke<string>('get_meeting_folder_path');
            if (fetched) {
              folderPath = fetched;
            }
          } catch (error) {
            console.warn('Fallback get_meeting_folder_path failed:', error);
          }
        }

        // v1.6.1 — attach transcripts to the RECORDING's own meeting, not the app's
        // mutable "current meeting" selection. `currentMeeting` drifts if the user opens
        // another meeting while recording (the 0036 Today/prep view makes this easy), which
        // sent a whole recording's transcript onto an unrelated meeting. Authoritative
        // order: the backend-reported recording id (from the folder metadata, set at start)
        // → the pinned `activeRecordingMeetingId` (never changed by navigation) → only then
        // `currentMeeting` (legacy fallback for a stop with neither, e.g. a page reload).
        const pinnedRecordingId =
          activeRecordingMeetingId && activeRecordingMeetingId !== PLACEHOLDER_MEETING_ID
            ? activeRecordingMeetingId
            : null;
        const currentSelectedId =
          currentMeeting?.id && currentMeeting.id !== PLACEHOLDER_MEETING_ID
            ? currentMeeting.id
            : null;
        const existingMeetingId =
          (stopResult?.meeting_id ?? null) ?? pinnedRecordingId ?? currentSelectedId;

        // Abandoned-recording cleanup: if there are no transcripts AND no notes were typed,
        // delete the empty in-progress row created at start so it doesn't litter the dashboard.
        //
        // specs/0019 WS6.1 — "zero transcripts" is a false abandonment signal for a meeting
        // that is calendar-linked or actually captured audio (e.g. a Join & Record call Zoom
        // auto-stopped before transcripts flushed). The shared gate (lib/discard-meeting,
        // also used by the failed-start orphan cleanup) only allows a delete when there are
        // no typed notes AND the backend confirms nothing durable to keep; on any error it
        // keeps the meeting (never destroy data).
        if (existingMeetingId && freshTranscripts.length === 0) {
          const safeToDiscard = await canDiscardMeeting(existingMeetingId, folderPath ?? null);

          if (safeToDiscard) {
            console.log('🗑️ Abandoned recording (no transcripts, no notes, no audio, not calendar-linked) — deleting empty meeting:', existingMeetingId);
            try {
              await invoke('api_delete_meeting', { meetingId: existingMeetingId });
            } catch (error) {
              console.warn('Failed to delete abandoned empty meeting:', error);
            }

            // Reset to the placeholder so we don't leave a stale id selected.
            setCurrentMeeting({ id: PLACEHOLDER_MEETING_ID, title: '+ New Call' });
            await refetchMeetings();
            await markMeetingAsSaved();
            sessionStorage.removeItem('last_recording_folder_path');
            sessionStorage.removeItem('last_recording_meeting_name');
            sessionStorage.removeItem('last_recording_resumed');
            sessionStorage.removeItem('last_recording_prior_audio_duration');
            sessionStorage.removeItem('indexeddb_current_meeting_id');

            setStatus(RecordingStatus.IDLE);
            setIsMeetingActive(false);
            setIsRecordingDisabled(false);
            return;
          }
        }

        // Transcription didn't reach a clean "complete" state, but there ARE transcripts to
        // keep. Persist them (below) and warn the user that the tail may be missing rather
        // than losing the whole meeting.
        if (transcriptionTimedOut && freshTranscripts.length > 0) {
          toast.warning('Transcription didn’t fully finish', {
            description: 'We saved the transcript captured so far — the end of the meeting may be incomplete.',
            duration: 8000,
          });
        }

        console.log('💾 Saving COMPLETE transcripts to database...', {
          transcript_count: freshTranscripts.length,
          meeting_name: savedMeetingName || meetingTitle,
          folder_path: folderPath,
          existing_meeting_id: existingMeetingId,
          sample_text: freshTranscripts.length > 0 ? freshTranscripts[0].text.substring(0, 50) + '...' : 'none',
          last_transcript: freshTranscripts.length > 0 ? freshTranscripts[freshTranscripts.length - 1].text.substring(0, 30) + '...' : 'none',
        });

        try {
          const responseData = await storageService.saveMeeting(
            savedMeetingName || meetingTitle || 'New Meeting',  // PREFER savedMeetingName (backend source)
            freshTranscripts,
            folderPath,
            existingMeetingId, // attach to the meeting created at start (null => create new)
            resumed,            // specs/0037: append to the same meeting when resuming
            audioOffsetSeconds, // specs/0037: shift appended transcripts by prior duration
          );

          const meetingId = responseData.meeting_id;
          if (!meetingId) {
            console.error('No meeting_id in response:', responseData);
            throw new Error('No meeting ID received from save operation');
          }

          let shouldDetectSummaryLanguage = false;
          try {
            shouldDetectSummaryLanguage = !(await applyPinnedSummaryLanguageToMeeting(meetingId));
          } catch (error) {
            console.warn('Failed to apply pinned summary language preference for new meeting:', error);
            toast.warning('Could not apply default summary language', {
              description: 'The meeting was saved, but the default summary language was not applied.',
            });
          }

          if (shouldDetectSummaryLanguage) {
            try {
              await detectAndCacheSummaryLanguage(
                meetingId,
                freshTranscripts.map(t => t.text)
              );
            } catch (error) {
              console.warn('Failed to detect summary language for new meeting:', error);
              toast.warning('Could not detect summary language', {
                description: 'The meeting was saved, but Auto could not detect the summary language.',
              });
            }
          }

          console.log('✅ Successfully saved COMPLETE meeting with ID:', meetingId);
          console.log('   Transcripts:', freshTranscripts.length);
          console.log('   folder_path:', folderPath);

          // Notes typed during recording are already persisted (the live notepad autosaves
          // directly to this meeting id from recording start), so there's nothing to flush here.

          // Mark meeting as saved in IndexedDB (for recovery system)
          await markMeetingAsSaved();

          // Low-power-mode bookkeeping (spec §§3,5) + spec 0051 WS2 durable handoff.
          let action: StopAction = 'none';
          let handoff: HandoffOutcome | null = null;
          try {
            const overrideMode = await invoke<string | null>('api_get_meeting_processing_mode', { meetingId });
            action = stopAction(sessionStartedDeferred(), sessionEverDeferred(), overrideMode);

            if (action === 'mark-defer' || action === 'process-now') {
              // spec 0051 WS2: write the DURABLE marker first, for both cases. For
              // 'process-now' this is what makes a failed handoff recoverable — the
              // backlog clears it as the last step of a successful pipeline.
              await invoke('api_set_meeting_processing_mode', { meetingId, mode: 'defer' });
            }

            if (action === 'process-now') {
              // Overridden live mid-meeting: full uniform pass now, battery or not.
              // Acknowledged, not fire-and-forget.
              try {
                handoff = await enqueueMeeting(meetingId, { force: true });
              } catch (error) {
                console.warn('Deferred-backlog handoff threw:', error);
                handoff = { accepted: false, reason: 'threw' };
              }
            }
          } catch (error) {
            console.warn('Processing-mode bookkeeping failed (meeting saved fine):', error);
          } finally {
            markSessionDeferred(false);
          }

          const followUp = stopFollowUp(action, handoff);
          if (followUp.toast) {
            toast.warning(followUp.toast.title, {
              description: followUp.toast.description,
              duration: 10000,
            });
          }

          // Optional auto-run speaker diarization (specs/0010, P1-C). Best-effort,
          // non-blocking, gated on the opt-in setting AND models already present — we
          // never auto-download mid-flow (that's the explicit "Identify speakers"
          // button's job). Swallow all errors so they can't affect the save flow.
          // spec 0051 WS2: this ALSO runs when a 'process-now' handoff was refused, so
          // a failed pipeline still leaves the user with speaker labels.
          if (followUp.autoDiarize) {
            void (async () => {
              try {
                const [enabled, modelsPresent] = await Promise.all([
                  invoke<boolean>('api_get_diarization_enabled'),
                  invoke<boolean>('api_diarization_models_present'),
                ]);
                if (enabled && modelsPresent) {
                  await invoke('api_diarize_meeting', { meetingId });
                }
              } catch (error) {
                console.warn('Auto-diarization skipped:', error);
              }
            })();
          }

          // Clean up session storage
          sessionStorage.removeItem('last_recording_folder_path');
          sessionStorage.removeItem('last_recording_meeting_name');
          sessionStorage.removeItem('last_recording_resumed');
          sessionStorage.removeItem('last_recording_prior_audio_duration');
          // Clean up IndexedDB meeting ID (redundant with markMeetingAsSaved cleanup, but ensures cleanup)
          sessionStorage.removeItem('indexeddb_current_meeting_id');

          // Refetch meetings and set current meeting
          await refetchMeetings();

          try {
            const meetingData = await storageService.getMeeting(meetingId);
            if (meetingData) {
              setCurrentMeeting({
                id: meetingId,
                title: meetingData.title
              });
              console.log('✅ Current meeting set:', meetingData.title);
            }
          } catch (error) {
            console.warn('Could not fetch meeting details, using ID only:', error);
            setCurrentMeeting({ id: meetingId, title: savedMeetingName || meetingTitle || 'New Meeting' });
          }

          // Mark as completed
          setStatus(RecordingStatus.COMPLETED);

          // No "saved" toast: the move to the meeting page below is the confirmation.

          // Auto-navigate after a short delay with source parameter
          setTimeout(() => {
            router.push(`/meeting-details?id=${meetingId}&source=recording`);
            // Cleared when the recorder unmounts (below), not here: the push is a transition,
            // so clearing now left the recorder on screen with zero segments — it rendered the
            // fresh-meeting "Welcome to Nixon!" state until the meeting page arrived.
            clearOnLeaveRef.current = true;

            // Reset to IDLE after navigation
            setStatus(RecordingStatus.IDLE);
          }, 2000);

        } catch (saveError) {
          console.error('Failed to save meeting to database:', saveError);
          setStatus(RecordingStatus.ERROR, saveError instanceof Error ? saveError.message : 'Unknown error');
          toast.error('Failed to save meeting', {
            description: saveError instanceof Error ? saveError.message : 'Unknown error'
          });
          throw saveError;
        }
      } else {
        // No save needed, go back to IDLE
        setStatus(RecordingStatus.IDLE);
      }

      setIsMeetingActive(false);
      // isRecording already set to false at function start
      setIsRecordingDisabled(false);
    } catch (error) {
      console.error('Error in handleRecordingStop:', error);
      setStatus(RecordingStatus.ERROR, error instanceof Error ? error.message : 'Unknown error');
      // isRecording already set to false at function start
      setIsRecordingDisabled(false);
    } finally {
      // Always reset the guard flag when done
      stopInProgressRef.current = false;
      // specs/0019 WS6.5 — the recording is over on every terminal path; clear the
      // authoritative live-recording id so opening a past meeting no longer redirects
      // to /record.
      setActiveRecordingMeetingId(null);
    }
  }, [
    setIsRecording,
    setIsRecordingDisabled,
    setStatus,
    transcriptsRef,
    flushBuffer,
    meetingTitle,
    markMeetingAsSaved,
    refetchMeetings,
    setCurrentMeeting,
    setMeetings,
    meetings,
    setIsMeetingActive,
    router,
    currentMeeting,
    activeRecordingMeetingId,
    setActiveRecordingMeetingId,
    ensureRecordingStoppedDeferred,
    enqueueMeeting,
  ]);

  // Set only by a completed stop, so leaving the recorder MID-recording keeps its transcript.
  const clearTranscriptsRef = useRef(clearTranscripts);
  clearTranscriptsRef.current = clearTranscripts;
  useEffect(() => () => {
    if (clearOnLeaveRef.current) clearTranscriptsRef.current();
  }, []);

  // Expose handleRecordingStop function to window for Rust callbacks
  const handleRecordingStopRef = useRef(handleRecordingStop);
  useEffect(() => {
    handleRecordingStopRef.current = handleRecordingStop;
  });

  useEffect(() => {
    (window as any).handleRecordingStop = (callApi: boolean = true) => {
      handleRecordingStopRef.current(callApi);
    };

    // Cleanup on unmount
    return () => {
      delete (window as any).handleRecordingStop;
    };
  }, []);

  // Derive summaryStatus from RecordingStatus for backward compatibility
  const summaryStatus: SummaryStatus = status === RecordingStatus.PROCESSING_TRANSCRIPTS ? 'processing' : 'idle';

  return {
    handleRecordingStop,
    isStopping,
    isProcessingTranscript,
    isSavingTranscript,
    summaryStatus,
    setIsStopping: (value: boolean) => {
      setStatus(value ? RecordingStatus.STOPPING : RecordingStatus.IDLE);
    },
  };
}
