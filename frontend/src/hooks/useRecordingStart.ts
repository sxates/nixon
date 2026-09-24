import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { recordingService } from '@/services/recordingService';
import { showRecordingNotification } from '@/lib/recordingNotification';
import {
  consumePendingJoinMeeting,
  seedMeetingParticipants,
  waitForPendingJoinCreate,
  type PendingJoinMeeting,
} from '@/lib/calendar';
import {
  consumeResumeRecording,
  markRecoveryEntriesSavedForFolder,
  RESUME_ARMED_EVENT,
  setResumeInFlight,
  type ResumeRecordingDescriptor,
} from '@/lib/resume-recording';
import { clearStopRecordingResult } from '@/lib/recording-stop';
import { canDiscardMeeting } from '@/lib/discard-meeting';
import { surfaceRecordingStartFailure } from '@/lib/recording-start-errors';
import { AUTO_START_ABANDONED_EVENT } from '@/hooks/useRecordEmptyPhase';
import { toast } from 'sonner';

interface UseRecordingStartReturn {
  handleRecordingStart: () => Promise<void>;
  isAutoStarting: boolean;
}

/** Placeholder meeting the sidebar shows when nothing is being recorded (mirrors SidebarProvider). */
const PLACEHOLDER_MEETING = { id: 'intro-call', title: '+ New Call' };

/**
 * Session-storage keys the STOP path (useRecordingStop) writes from the `recording-stopped`
 * event to route a resumed session's save into the append branch. They are cleared at every
 * fresh START so a stale `last_recording_resumed='true'` from an errored/abandoned prior save
 * can never misroute the NEXT recording's stop (specs/0037 FIX #7).
 */
const RESUME_STOP_KEYS = ['last_recording_resumed', 'last_recording_prior_audio_duration'] as const;

/**
 * Custom hook for managing recording start lifecycle.
 * Handles both manual start (button click) and auto-start (from sidebar navigation).
 *
 * Features:
 * - Meeting title generation (format: Meeting DD_MM_YY_HH_MM_SS)
 * - Transcript clearing on start
 * - Recording notification display
 * - Auto-start from sidebar via sessionStorage flag
 */
export function useRecordingStart(
  isRecording: boolean,
  setIsRecording: (value: boolean) => void,
  showModal?: (name: 'modelSelector' | 'errorAlert', message?: string) => void
): UseRecordingStartReturn {
  const [isAutoStarting, setIsAutoStarting] = useState(false);

  const { clearTranscripts, setMeetingTitle } = useTranscripts();
  const {
    setIsMeetingActive,
    setCurrentMeeting,
    setActiveRecordingMeetingId,
    activeRecordingMeetingId,
    refetchMeetings,
  } = useSidebar();
  const { selectedDevices } = useConfig();
  const { setStatus } = useRecordingState();

  // Guards against creating more than one meeting row per recording session (e.g. React
  // re-renders, HMR, or two start paths racing). Reset by the stop handler / when idle.
  const meetingCreatedRef = useRef(false);
  // The id + in-flight promise of this session's create, so a start path that LOST the
  // create race but holds the pending Join & Record can adopt the calendar identity
  // instead of silently discarding it (specs/0029 WS2.1).
  const createdMeetingIdRef = useRef<string | null>(null);
  const meetingCreatePromiseRef = useRef<Promise<void> | null>(null);
  // The id of a row THIS start attempt freshly INSERTed (plain date-stamped create only —
  // never an adopted calendar row, never a resumed/existing meeting). If the backend start
  // invoke then throws (permission denied, device gone, model race), the catch deletes this
  // row so a failed start can't strand an empty, transcript-less meeting (specs/0037 FIX #6).
  // DISARMED the moment the backend start invoke RESOLVES (review-2 FIX A): once the session
  // is live the row is a real meeting, and leaving this armed for the whole recording let a
  // racing second start's "already in progress" failure delete the LIVE session's row.
  const freshlyCreatedMeetingIdRef = useRef<string | null>(null);

  // Clear the resumed-session routing state a prior STOP may have left behind — the
  // sessionStorage keys the recording-stopped listener writes AND the in-memory stop-invoke
  // result stash — so every new recording starts clean and its stop can't be misrouted
  // into the append path (FIX #7 / review-2 FIX C).
  const clearStaleResumeStopKeys = useCallback(() => {
    clearStopRecordingResult();
    if (typeof window === 'undefined') return;
    RESUME_STOP_KEYS.forEach((key) => sessionStorage.removeItem(key));
  }, []);

  // Undo a fresh meeting row created earlier in THIS start attempt when the backend start
  // then failed (specs/0037 FIX #6, hardened by review-2 FIX A). `orphanId` is the row the
  // CALLING attempt inserted (null when it created nothing — adopted calendar row, resumed
  // meeting, or another path's create), so a start attempt can only ever delete its own row.
  const deleteOrphanMeetingOnStartFailure = useCallback(
    async (orphanId: string | null, startError?: unknown) => {
      // Only a row this attempt inserted, and only while it's still armed — a successful
      // backend start disarms the ref immediately, so a live session's row never qualifies.
      if (!orphanId || freshlyCreatedMeetingIdRef.current !== orphanId) return;
      freshlyCreatedMeetingIdRef.current = null;

      // A start rejected because a recording is ALREADY in progress didn't orphan anything —
      // whatever row exists belongs to the live session. Never delete on that failure, nor
      // when the row IS the currently-active recording (stale-isRecording double start).
      const message = startError instanceof Error ? startError.message : String(startError ?? '');
      if (/already in progress/i.test(message)) return;
      if (activeRecordingMeetingId && orphanId === activeRecordingMeetingId) return;

      // Same safety gate as the stop path's abandoned cleanup (lib/discard-meeting): typed
      // notes or anything durable (transcripts/audio/calendar link) always vetoes the delete.
      if (!(await canDiscardMeeting(orphanId, null))) return;

      try {
        await invoke('api_delete_meeting', { meetingId: orphanId });
      } catch (deleteError) {
        console.warn('Failed to delete orphan meeting after failed start:', deleteError);
      }
      // The row is gone — drop every pointer to it so the sidebar/live state doesn't reference
      // a deleted meeting, and let a later retry mint a fresh one.
      meetingCreatedRef.current = false;
      createdMeetingIdRef.current = null;
      setActiveRecordingMeetingId(null);
      setCurrentMeeting(PLACEHOLDER_MEETING);
      void refetchMeetings();
    },
    [activeRecordingMeetingId, setActiveRecordingMeetingId, setCurrentMeeting, refetchMeetings],
  );

  // Generate meeting title with timestamp
  const generateMeetingTitle = useCallback(() => {
    const now = new Date();
    const day = String(now.getDate()).padStart(2, '0');
    const month = String(now.getMonth() + 1).padStart(2, '0');
    const year = String(now.getFullYear()).slice(-2);
    const hours = String(now.getHours()).padStart(2, '0');
    const minutes = String(now.getMinutes()).padStart(2, '0');
    const seconds = String(now.getSeconds()).padStart(2, '0');
    return `Meeting ${day}_${month}_${year}_${hours}_${minutes}_${seconds}`;
  }, []);

  // One consume path for all three start triggers (specs/0029 WS2.1): the pending
  // Join & Record (if armed) supplies the title — NEVER a date-stamp when a calendar
  // identity is known; otherwise generate the usual date-stamped title.
  const consumeStartIdentity = useCallback((): {
    pendingJoin: PendingJoinMeeting | null;
    title: string;
  } => {
    const pendingJoin = consumePendingJoinMeeting();
    return { pendingJoin, title: pendingJoin?.title ?? generateMeetingTitle() };
  }, [generateMeetingTitle]);

  // Make `meetingId` the recording session's meeting: point the sidebar/live state at
  // it and (idempotently) seed the attendee roster — the backend resolves the roster
  // from the row's calendar_event_id, so this fires for adopted/reconciled rows too,
  // not only the pre-created branch (specs/0029 WS2.1).
  const adoptMeetingForSession = useCallback(
    (meetingId: string, title: string) => {
      createdMeetingIdRef.current = meetingId;
      setCurrentMeeting({ id: meetingId, title });
      setActiveRecordingMeetingId(meetingId); // WS6.5: authoritative recording id
      void seedMeetingParticipants(meetingId);
      void refetchMeetings();
    },
    [setCurrentMeeting, setActiveRecordingMeetingId, refetchMeetings],
  );

  // Resolve a pending Join & Record to the id of its calendar-linked row (specs/0029
  // WS2.1). Order of preference: the reconciled pre-created id; the id of the
  // pre-create still in flight (the stash is written BEFORE the awaited create, so a
  // consume can race it — wait briefly instead of minting a duplicate); a row created
  // here with the calendar link (the backend's find_adoptable_calendar_meeting dedupe
  // keys off calendarEventId, so a lost race still converges on one row). Returns null
  // only when no calendar-linked row could be obtained.
  const resolvePendingJoinMeetingId = useCallback(
    async (pending: PendingJoinMeeting): Promise<string | null> => {
      if (pending.id) return pending.id;
      const inFlightId = await waitForPendingJoinCreate();
      if (inFlightId) return inFlightId;
      if (!pending.calendarEventId) return null;
      try {
        const result = await invoke<{ meeting_id: string }>('api_create_meeting', {
          meetingTitle: pending.title,
          origin: 'recorded',
          calendarEventId: pending.calendarEventId,
          startedAt: pending.startsAt ?? null,
          // specs/0036: carry the recurring-series key on the deferred (navigate-to-recorder)
          // path too, so this row groups into its series like the inline joinAndRecord path.
          calendarSeriesKey: pending.calendarSeriesKey ?? null,
        });
        return result?.meeting_id ?? null;
      } catch (error) {
        console.error('Failed to create calendar-linked meeting:', error);
        return null;
      }
    },
    [],
  );

  // A pending Join & Record lost the create race — another start path already minted
  // this session's row (a date-stamped one, since it consumed nothing). The calendar
  // identity must not be silently discarded (specs/0029 WS2.1): prefer the
  // calendar-linked row (it carries the event title + calendar_event_id, which both
  // attendee seeding and the backend dedupe key off), leaving the racing row behind
  // empty. If no calendar row can be resolved, re-title the racing row so at least
  // the event title sticks.
  const adoptPendingJoinIntoSession = useCallback(
    async (pending: PendingJoinMeeting): Promise<void> => {
      // Let the racing create settle so we don't interleave with its state updates
      // (it holds createdMeetingIdRef / setCurrentMeeting until it resolves).
      try {
        await meetingCreatePromiseRef.current;
      } catch {
        /* the racing path already surfaced its own error */
      }

      const calendarRowId = await resolvePendingJoinMeetingId(pending);
      if (calendarRowId) {
        setMeetingTitle(pending.title);
        adoptMeetingForSession(calendarRowId, pending.title);
        return;
      }

      const racingId = createdMeetingIdRef.current;
      if (!racingId) return; // racing create failed too; its toast already fired
      try {
        await invoke('api_save_meeting_title', { meetingId: racingId, title: pending.title });
        setMeetingTitle(pending.title);
        setCurrentMeeting({ id: racingId, title: pending.title });
        void refetchMeetings();
      } catch (error) {
        console.error('Failed to re-title meeting with calendar event title:', error);
      }
    },
    [resolvePendingJoinMeetingId, adoptMeetingForSession, setMeetingTitle, setCurrentMeeting, refetchMeetings],
  );

  // Persist-at-start: create the real meeting row the moment a recording begins, so the live
  // notepad autosaves straight to it (no draft/placeholder dance) and stop just attaches the
  // transcripts to this same id. Idempotent within a session via `meetingCreatedRef` so React
  // re-renders / HMR / racing start paths can't create duplicate empty rows. Failure is
  // surfaced as a toast and never aborts the recording — worst case stop falls back to
  // create-new behavior because currentMeeting stays the placeholder.
  //
  // Join & Record (spec 0015, Phase C): when a pending Join & Record is passed as
  // `preCreated`, we ADOPT its calendar-linked row (pre-created, in flight, or created
  // here) instead of inserting a second one — preserving its title/origin/event-id. The
  // manual "New recording" path passes no `preCreated` and behaves exactly as before
  // (create a fresh date-stamped row).
  // Returns the id of the row THIS call freshly INSERTed (null when it created nothing:
  // adopted calendar row, lost the create race, or the create failed) so the caller can
  // scope its failed-start cleanup to its own row (review-2 FIX A).
  const createMeetingForRecording = useCallback(
    async (title: string, preCreated?: PendingJoinMeeting | null): Promise<string | null> => {
      if (meetingCreatedRef.current) {
        // Another start path already claimed this session's create. If WE hold the
        // pending Join & Record, adopt its calendar identity instead of dropping it
        // (specs/0029 WS2.1 — previously this early-return silently discarded it,
        // stranding a date-stamped, attendee-less row as the recording meeting).
        if (preCreated) await adoptPendingJoinIntoSession(preCreated);
        return null;
      }
      meetingCreatedRef.current = true; // claim the slot before awaiting (prevents races)

      let freshRowId: string | null = null;

      const createPromise = (async (): Promise<void> => {
        // Join & Record (specs/0019 WS6.3 + 0029 WS2.1): adopt the calendar-linked
        // row (pre-created, in flight, or created here with the event id + start) so
        // the live recording keeps the event's title + attendee roster instead of
        // degrading to a bare date-stamped meeting.
        if (preCreated) {
          const calendarRowId = await resolvePendingJoinMeetingId(preCreated);
          if (calendarRowId) {
            adoptMeetingForSession(calendarRowId, preCreated.title);
            return;
          }
          // No calendar row obtainable — fall through to the plain create below
          // (recording must not be blocked). `title` is still the event title.
        }

        try {
          const result = await invoke<{ meeting_id: string }>('api_create_meeting', {
            meetingTitle: title,
            folderPath: null,
          });
          const realId = result?.meeting_id;
          if (!realId) {
            throw new Error('api_create_meeting returned no meeting_id');
          }
          // Show the in-progress meeting in the sidebar/dashboard immediately.
          adoptMeetingForSession(realId, title);
          // Mark this specific row as one THIS attempt created, so a subsequent start
          // failure can delete it (specs/0037 FIX #6). Adopted calendar rows and resumed
          // meetings never set this, so they're never deleted on failure.
          freshlyCreatedMeetingIdRef.current = realId;
          freshRowId = realId;
        } catch (error) {
          console.error('Failed to create meeting at recording start:', error);
          meetingCreatedRef.current = false; // allow a later path to retry
          toast.error('Could not start a new meeting record', {
            description: 'Your notes may not save until the recording is saved. Recording continues.',
            duration: 5000,
          });
        }
      })();
      meetingCreatePromiseRef.current = createPromise;
      await createPromise;
      return freshRowId;
    },
    [adoptPendingJoinIntoSession, resolvePendingJoinMeetingId, adoptMeetingForSession],
  );

  // Allow a fresh meeting to be created on the next recording once the current one ends.
  useEffect(() => {
    if (!isRecording) {
      meetingCreatedRef.current = false;
      createdMeetingIdRef.current = null;
      meetingCreatePromiseRef.current = null;
      freshlyCreatedMeetingIdRef.current = null;
    }
  }, [isRecording]);

  // Check if Parakeet transcription model is ready
  const checkParakeetReady = useCallback(async (): Promise<boolean> => {
    try {
      await invoke('parakeet_init');
      const hasModels = await invoke<boolean>('parakeet_has_available_models');
      return hasModels;
    } catch (error) {
      console.error('Failed to check Parakeet status:', error);
      return false;
    }
  }, []);

  // Check if any model is currently downloading
  const checkIfModelDownloading = useCallback(async (): Promise<boolean> => {
    try {
      const models = await invoke<any[]>('parakeet_get_available_models');
      const isDownloading = models.some(m =>
        m.status && (
          typeof m.status === 'object'
            ? 'Downloading' in m.status
            : m.status === 'Downloading'
        )
      );
      return isDownloading;
    } catch (error) {
      console.error('Failed to check model download status:', error);
      return false; // Default to not downloading (will show error + modal)
    }
  }, []);

  // Resume / continue an existing recording (specs/0037). Unlike the three fresh-start
  // paths, this REUSES an existing meeting_id + folder: it SKIPS `api_create_meeting`
  // entirely, adopts the existing id as the session's authoritative recording id, and
  // threads `{ meetingId, resumeFolderPath }` into the start invoke so the backend appends
  // to that meeting instead of minting a new row. Triggered by the `resumeRecording`
  // sessionStorage key (mirror of `autoStartRecording`), consumed on mount below.
  const startResumeSession = useCallback(
    async (descriptor: ResumeRecordingDescriptor) => {
      setIsAutoStarting(true);
      // Wipe any stale resumed-stop routing keys from a previous session (FIX #7).
      clearStaleResumeStopKeys();
      try {
        // Same model-readiness gate as every other start path.
        const parakeetReady = await checkParakeetReady();
        if (!parakeetReady) {
          const isDownloading = await checkIfModelDownloading();
          if (isDownloading) {
            toast.info('Model download in progress', {
              description: 'Please wait for the transcription model to finish downloading before recording.',
              duration: 5000,
            });
          } else {
            toast.error('Transcription model not ready', {
              description: 'Please download a transcription model before recording.',
              duration: 5000,
            });
            showModal?.('modelSelector', 'Transcription model setup required');
          }
          setStatus(RecordingStatus.IDLE);
          return;
        }

        // Resolve the folder from the meeting row BEFORE starting, ALWAYS (review-2 FIX B;
        // specs/0073: the descriptor's path may predate a recordings move, so the row wins
        // and the descriptor is only the fallback). A start without `resumeFolderPath`
        // records into a new folder and saves a DUPLICATE meeting row, so a meeting with no
        // folder at all ABORTS loudly rather than silently degrading to a fresh recording.
        let folderPath: string | null = null;
        try {
          const metadata = await invoke<{ folder_path?: string | null } | null>(
            'api_get_meeting_metadata',
            { meetingId: descriptor.meetingId },
          );
          folderPath = metadata?.folder_path || null;
        } catch (error) {
          console.error('Could not resolve the meeting folder for resume:', error);
        }
        folderPath = folderPath ?? descriptor.folderPath ?? null;
        if (!folderPath) {
          toast.error("Can't continue this recording", {
            description: 'Its recording folder is missing, so there is nothing to resume into.',
          });
          setStatus(RecordingStatus.IDLE);
          return;
        }

        const title = descriptor.meetingName ?? '';
        setStatus(RecordingStatus.STARTING, 'Resuming recording...');

        // Reuse the existing meeting + folder — no new row is created.
        await recordingService.startRecordingWithDevices(
          selectedDevices?.micDevice || null,
          selectedDevices?.systemDevice || null,
          title,
          { meetingId: descriptor.meetingId, resumeFolderPath: folderPath },
        );

        // Adopt the existing id AFTER the backend confirms the resume, mirroring how the
        // fresh paths only adopt once the create lands. `meetingCreatedRef` is claimed so
        // a racing start path can't mint a duplicate row against this session.
        meetingCreatedRef.current = true;
        adoptMeetingForSession(descriptor.meetingId, title);
        if (descriptor.meetingName) setMeetingTitle(descriptor.meetingName);

        // The crashed session's IndexedDB recovery entries are keyed by fabricated
        // `meeting-<timestamp>` ids, not this SQLite id — mark the ones for this folder
        // saved so the legacy recovery prompt stops re-offering the meeting we just
        // resumed (review-2 FIX D). Best-effort, never blocks the live session.
        void markRecoveryEntriesSavedForFolder(folderPath);

        setIsRecording(true);
        clearTranscripts();
        setIsMeetingActive(true);

        await showRecordingNotification();
      } catch (error) {
        console.error('Failed to resume recording:', error);
        setStatus(RecordingStatus.ERROR, error instanceof Error ? error.message : 'Failed to resume recording');
        setIsRecording(false);
        toast.error('Failed to resume recording', {
          description: error instanceof Error ? error.message : 'Check the console for details.',
        });
      } finally {
        setIsAutoStarting(false);
      }
    },
    [
      selectedDevices,
      adoptMeetingForSession,
      setMeetingTitle,
      setIsRecording,
      clearTranscripts,
      setIsMeetingActive,
      checkParakeetReady,
      checkIfModelDownloading,
      showModal,
      setStatus,
      clearStaleResumeStopKeys,
    ],
  );

  // Single consume+start path for an armed resume, shared by BOTH the mount effect (below)
  // and the `nixon:resume-armed` window event (FIX #8). `consumeResumeRecording` is
  // read-and-clear, so if both fire only the first sees the descriptor — the resume starts
  // exactly once and never double-starts.
  const maybeStartArmedResume = useCallback(async () => {
    if (typeof window === 'undefined') return;
    if (isRecording || isAutoStarting) return;
    const descriptor = consumeResumeRecording();
    if (!descriptor) return;
    console.log('Resuming recording from stash:', descriptor.meetingId);
    // Flag the resume as in flight SYNCHRONOUSLY with the consume (review-2 FIX D): the
    // page's IndexedDB recovery startup check runs after this hook's mount effect and
    // consults this flag, so it can never race the async start below and offer to
    // "recover" the very meeting being resumed.
    setResumeInFlight(true);
    try {
      await startResumeSession(descriptor);
    } finally {
      setResumeInFlight(false);
    }
  }, [isRecording, isAutoStarting, startResumeSession]);

  // Consume the `resumeRecording` stash on mount (mirror of the autoStartRecording effect
  // below). Only fires when a resume was armed by the relaunch prompt or the meeting-details
  // "Continue recording" action; a plain visit to /record leaves this a no-op.
  useEffect(() => {
    void maybeStartArmedResume();
  }, [maybeStartArmedResume]);

  // Arm-while-already-mounted (FIX #8): if the user is already sitting on /record, a
  // `router.push('/record')` from the resume initiator does NOT remount this hook, so the
  // mount effect above never re-runs. `armResumeRecording` dispatches `nixon:resume-armed`
  // after writing the stash; catch it here and run the same consume+start path.
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const handleResumeArmed = () => {
      void maybeStartArmedResume();
    };
    window.addEventListener(RESUME_ARMED_EVENT, handleResumeArmed);
    return () => window.removeEventListener(RESUME_ARMED_EVENT, handleResumeArmed);
  }, [maybeStartArmedResume]);

  // Handle manual recording start (from button click)
  const handleRecordingStart = useCallback(async () => {
    // Wipe any stale resumed-stop routing keys from a previous session (FIX #7).
    clearStaleResumeStopKeys();
    // The row THIS attempt freshly inserts (null when it adopts or loses the create race);
    // scopes the failed-start cleanup in the catch to this attempt's own row (FIX A).
    let freshRowId: string | null = null;
    try {
      console.log('handleRecordingStart called - checking Parakeet model status');

      // Check if Parakeet transcription model is ready before starting
      const parakeetReady = await checkParakeetReady();
      if (!parakeetReady) {
        const isDownloading = await checkIfModelDownloading();
        if (isDownloading) {
          toast.info('Model download in progress', {
            description: 'Please wait for the transcription model to finish downloading before recording.',
            duration: 5000,
          });
        } else {
          toast.error('Transcription model not ready', {
            description: 'Please download a transcription model before recording.',
            duration: 5000,
          });
          showModal?.('modelSelector', 'Transcription model setup required');
        }
        setStatus(RecordingStatus.IDLE);
        return;
      }

      console.log('Parakeet ready - setting up meeting title and state');

      // Join & Record (spec 0015): if a calendar-linked meeting is armed, adopt its
      // title (the event title) instead of a date-stamp; otherwise generate the usual title.
      const { pendingJoin, title: meetingTitle } = consumeStartIdentity();
      setMeetingTitle(meetingTitle);

      // Set STARTING status before initiating backend recording
      setStatus(RecordingStatus.STARTING, 'Initializing recording...');

      // Persist the meeting row FIRST so its real id can be handed to the backend at start
      // (crash recovery writes that id into the recording's metadata.json). For Join & Record
      // this adopts the pre-created calendar row (no duplicate INSERT).
      freshRowId = await createMeetingForRecording(meetingTitle, pendingJoin);
      const createdMeetingId = createdMeetingIdRef.current ?? undefined;

      // Start the actual backend recording, threading the created id when we have one.
      console.log('Starting backend recording with meeting:', meetingTitle);
      await recordingService.startRecordingWithDevices(
        selectedDevices?.micDevice || null,
        selectedDevices?.systemDevice || null,
        meetingTitle,
        createdMeetingId ? { meetingId: createdMeetingId } : undefined
      );
      console.log('Backend recording started successfully');
      // The backend confirmed the start — this row now belongs to a LIVE session. Disarm the
      // failed-start cleanup immediately (review-2 FIX A): leaving it armed for the whole
      // session let a racing second start's failure delete the live meeting's row.
      freshlyCreatedMeetingIdRef.current = null;

      // Update state after successful backend start
      // Note: RECORDING status will be set by RecordingStateContext event listener
      console.log('Setting isRecordingState to true');
      setIsRecording(true); // This will also update the sidebar via the useEffect
      clearTranscripts(); // Clear previous transcripts when starting new recording
      setIsMeetingActive(true);

      // Show recording notification if enabled
      await showRecordingNotification();
    } catch (error) {
      console.error('Failed to start recording:', error);
      setStatus(RecordingStatus.ERROR, error instanceof Error ? error.message : 'Failed to start recording');
      setIsRecording(false); // Reset state on error
      // Start threw AFTER this attempt created a fresh row — remove the orphan so a failed
      // start (permission denied, device gone) doesn't strand an empty meeting (FIX #6).
      // Scoped to THIS attempt's row + gated on the failure cause (FIX A).
      await deleteOrphanMeetingOnStartFailure(freshRowId, error);
      // The device-specific copy the retired RecordingControls rendered inline (specs/0057
      // Plan 2 Task 7) — the start hook is now the only thing that knows a start failed.
      surfaceRecordingStartFailure(error, showModal);
      // Re-throw so callers can still react to a failed start.
      throw error;
    }
  }, [consumeStartIdentity, setMeetingTitle, setIsRecording, clearTranscripts, setIsMeetingActive, checkParakeetReady, checkIfModelDownloading, selectedDevices, showModal, setStatus, createMeetingForRecording, clearStaleResumeStopKeys, deleteOrphanMeetingOnStartFailure]);

  // Check for autoStartRecording flag and start recording automatically
  useEffect(() => {
    const checkAutoStartRecording = async () => {
      if (typeof window !== 'undefined') {
        const shouldAutoStart = sessionStorage.getItem('autoStartRecording');
        if (shouldAutoStart === 'true' && !isRecording && !isAutoStarting) {
          console.log('Auto-starting recording from navigation...');
          setIsAutoStarting(true);
          sessionStorage.removeItem('autoStartRecording'); // Clear the flag
          clearStaleResumeStopKeys(); // Wipe stale resumed-stop routing keys (FIX #7)

          // Check if Parakeet transcription model is ready before starting
          const parakeetReady = await checkParakeetReady();
          if (!parakeetReady) {
            const isDownloading = await checkIfModelDownloading();
            if (isDownloading) {
              toast.info('Model download in progress', {
                description: 'Please wait for the transcription model to finish downloading before recording.',
                duration: 5000,
              });
            } else {
              toast.error('Transcription model not ready', {
                description: 'Please download a transcription model before recording.',
                duration: 5000,
              });
              showModal?.('modelSelector', 'Transcription model setup required');
            }
            setStatus(RecordingStatus.IDLE);
            setIsAutoStarting(false);
            // The start never reached STARTING: tell the record page to stop showing
            // "Listening…" for it (useRecordEmptyPhase).
            window.dispatchEvent(new Event(AUTO_START_ABANDONED_EVENT));
            return;
          }

          // Start the actual backend recording
          // The row THIS attempt freshly inserts (FIX A — scopes the catch's cleanup).
          let freshRowId: string | null = null;
          try {
            // Join & Record (spec 0015): adopt the armed calendar meeting's title when
            // present (this is the path Join & Record navigates into), else generate one.
            const { pendingJoin, title: generatedMeetingTitle } = consumeStartIdentity();

            // Set STARTING status before initiating backend recording
            setStatus(RecordingStatus.STARTING, 'Initializing recording...');

            // Persist the meeting row FIRST so its real id can be handed to the backend at
            // start (crash recovery writes it into metadata.json). For Join & Record this
            // adopts the pre-created row (no duplicate INSERT).
            freshRowId = await createMeetingForRecording(generatedMeetingTitle, pendingJoin);
            const createdMeetingId = createdMeetingIdRef.current ?? undefined;

            console.log('Auto-starting backend recording with meeting:', generatedMeetingTitle);
            const result = await recordingService.startRecordingWithDevices(
              selectedDevices?.micDevice || null,
              selectedDevices?.systemDevice || null,
              generatedMeetingTitle,
              createdMeetingId ? { meetingId: createdMeetingId } : undefined
            );
            console.log('Auto-start backend recording result:', result);
            // Start confirmed — the row is a live meeting now; disarm the failed-start
            // cleanup so nothing can delete it later (review-2 FIX A).
            freshlyCreatedMeetingIdRef.current = null;

            // Update UI state after successful backend start
            // Note: RECORDING status will be set by RecordingStateContext event listener
            setMeetingTitle(generatedMeetingTitle);
            setIsRecording(true);
            clearTranscripts();
            setIsMeetingActive(true);

            // Show recording notification if enabled
            await showRecordingNotification();
          } catch (error) {
            console.error('Failed to auto-start recording:', error);
            setStatus(RecordingStatus.ERROR, error instanceof Error ? error.message : 'Failed to auto-start recording');
            surfaceRecordingStartFailure(error, showModal);
            // Delete the orphan row a failed backend start left behind (FIX #6),
            // scoped to this attempt's own row + gated on the failure cause (FIX A).
            await deleteOrphanMeetingOnStartFailure(freshRowId, error);
          } finally {
            setIsAutoStarting(false);
          }
        }
      }
    };

    checkAutoStartRecording();
  }, [
    isRecording,
    isAutoStarting,
    selectedDevices,
    consumeStartIdentity,
    setMeetingTitle,
    setIsRecording,
    clearTranscripts,
    setIsMeetingActive,
    checkParakeetReady,
    checkIfModelDownloading,
    showModal,
    setStatus,
    createMeetingForRecording,
    clearStaleResumeStopKeys,
    deleteOrphanMeetingOnStartFailure,
  ]);

  // Listen for direct recording trigger from sidebar when already on home page
  useEffect(() => {
    const handleDirectStart = async () => {
      if (isRecording || isAutoStarting) {
        console.log('Recording already in progress, ignoring direct start event');
        return;
      }

      console.log('Direct start from sidebar - checking Parakeet model status');
      setIsAutoStarting(true);
      clearStaleResumeStopKeys(); // Wipe stale resumed-stop routing keys (FIX #7)

      // Check if Parakeet transcription model is ready before starting
      const parakeetReady = await checkParakeetReady();
      if (!parakeetReady) {
        const isDownloading = await checkIfModelDownloading();
        if (isDownloading) {
          toast.info('Model download in progress', {
            description: 'Please wait for the transcription model to finish downloading before recording.',
            duration: 5000,
          });
        } else {
          toast.error('Transcription model not ready', {
            description: 'Please download a transcription model before recording.',
            duration: 5000,
          });
          showModal?.('modelSelector', 'Transcription model setup required');
        }
        setStatus(RecordingStatus.IDLE);
        setIsAutoStarting(false);
        return;
      }

      // The row THIS attempt freshly inserts (FIX A — scopes the catch's cleanup).
      let freshRowId: string | null = null;
      try {
        // Join & Record (spec 0015): adopt the armed calendar meeting's title when
        // present (sidebar dispatches this event when already on /record), else generate one.
        const { pendingJoin, title: generatedMeetingTitle } = consumeStartIdentity();

        // Set STARTING status before initiating backend recording
        setStatus(RecordingStatus.STARTING, 'Initializing recording...');

        // Persist the meeting row FIRST so its real id can be handed to the backend at start
        // (crash recovery writes it into metadata.json). For Join & Record this adopts the
        // pre-created row (no duplicate INSERT).
        freshRowId = await createMeetingForRecording(generatedMeetingTitle, pendingJoin);
        const createdMeetingId = createdMeetingIdRef.current ?? undefined;

        console.log('Starting backend recording with meeting:', generatedMeetingTitle);
        const result = await recordingService.startRecordingWithDevices(
          selectedDevices?.micDevice || null,
          selectedDevices?.systemDevice || null,
          generatedMeetingTitle,
          createdMeetingId ? { meetingId: createdMeetingId } : undefined
        );
        console.log('Backend recording result:', result);
        // Start confirmed — the row is a live meeting now; disarm the failed-start
        // cleanup so nothing can delete it later (review-2 FIX A).
        freshlyCreatedMeetingIdRef.current = null;

        // Update UI state after successful backend start
        // Note: RECORDING status will be set by RecordingStateContext event listener
        setMeetingTitle(generatedMeetingTitle);
        setIsRecording(true);
        clearTranscripts();
        setIsMeetingActive(true);

        // Show recording notification if enabled
        await showRecordingNotification();
      } catch (error) {
        console.error('Failed to start recording from sidebar:', error);
        setStatus(RecordingStatus.ERROR, error instanceof Error ? error.message : 'Failed to start recording from sidebar');
        surfaceRecordingStartFailure(error, showModal);
        // Delete the orphan row a failed backend start left behind (FIX #6),
        // scoped to this attempt's own row + gated on the failure cause (FIX A).
        await deleteOrphanMeetingOnStartFailure(freshRowId, error);
      } finally {
        setIsAutoStarting(false);
      }
    };

    window.addEventListener('start-recording-from-sidebar', handleDirectStart);

    return () => {
      window.removeEventListener('start-recording-from-sidebar', handleDirectStart);
    };
  }, [
    isRecording,
    isAutoStarting,
    selectedDevices,
    consumeStartIdentity,
    setMeetingTitle,
    setIsRecording,
    clearTranscripts,
    setIsMeetingActive,
    checkParakeetReady,
    checkIfModelDownloading,
    showModal,
    setStatus,
    createMeetingForRecording,
    clearStaleResumeStopKeys,
    deleteOrphanMeetingOnStartFailure,
  ]);

  return {
    handleRecordingStart,
    isAutoStarting,
  };
}
