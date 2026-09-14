/**
 * Speaker diarization controller (specs/0010, P1-C).
 *
 * Encapsulates the "Identify speakers" flow for a single meeting:
 *  1. Ensure the two ONNX models are present (`api_diarization_models_present`);
 *     if not, kick off `api_download_diarization_models` (~35 MB, one-time) and
 *     surface progress.
 *  2. Start the background pass (`api_diarize_meeting`), which returns immediately.
 *  3. Listen for `diarization-{progress,complete,error}` events (via `safeListen`):
 *     - progress  → update the stage text shown next to the spinner
 *     - complete  → re-fetch the transcript so labels appear + success toast
 *     - error     → error toast
 *
 * The button is disabled while a pass is running (`isRunning`). Events without a
 * `meeting_id`, or for a different meeting, are ignored so two open meetings don't
 * cross-talk (model-download progress has no meeting_id, so we accept those too).
 *
 * WS3.1 (specs/0029): the backend keeps a per-meeting run registry, so this hook's
 * state is a VIEW of it, not the source of truth:
 *  - On mount it rehydrates from `api_diarization_status`, so navigating away and
 *    back never loses the "identifying…" state (or re-enables the button mid-run).
 *  - `api_diarize_meeting` returns `{ started, alreadyRunning }`; `alreadyRunning`
 *    means attach to the live pass — never toast it as a failure.
 *  - As defense-in-depth, a progress percentage that moves backwards within the
 *    same stage is ignored (a stage change legitimately resets the ramp).
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';

interface DiarizationProgressPayload {
  meeting_id?: string;
  stage: string;
  /** 0–100 progress within the current stage (currently only "diarizing"). */
  pct?: number;
}
interface DiarizationCompletePayload {
  meeting_id: string;
  speaker_count: number;
  /**
   * How the expected speaker count was decided (specs/0011 calendar-seed):
   *  - "manual"   → the user's Settings override forced an exact count
   *  - "calendar" → seeded from the linked calendar event's remote attendees
   *  - "auto"     → audio-derived cap (specs/0050): ad-hoc / distribution-list invite
   */
  speakerCountSource?: 'manual' | 'calendar' | 'auto';
  /** The count we seeded into the clusterer (manual/calendar); null for auto. */
  seededSpeakerCount?: number | null;
}
interface DiarizationErrorPayload {
  meeting_id: string;
  error: string;
}
/** `api_diarization_status` result (specs/0029 WS3.1); null when no run this session. */
interface DiarizationStatusPayload {
  running: boolean;
  stage: string;
  progressPct: number;
}
/** `api_diarize_meeting` result (specs/0029 WS3.1). */
interface DiarizeStartResult {
  started: boolean;
  alreadyRunning: boolean;
}

interface UseDiarizationOptions {
  meetingId: string | undefined;
  /** Called after a pass completes so the caller can re-fetch the transcript. */
  onComplete?: () => void | Promise<void>;
}

interface UseDiarizationReturn {
  isRunning: boolean;
  /** Human-readable current stage (e.g. "diarizing"), shown next to the spinner. */
  stage: string | null;
  /** 0–100 progress within the current stage, or null if not reported. */
  progressPct: number | null;
  identifySpeakers: () => Promise<void>;
}

export function useDiarization({
  meetingId,
  onComplete,
}: UseDiarizationOptions): UseDiarizationReturn {
  const [isRunning, setIsRunning] = useState(false);
  const [stage, setStage] = useState<string | null>(null);
  const [progressPct, setProgressPct] = useState<number | null>(null);

  // Monotonic-progress guard (specs/0029 WS3.1 defense-in-depth): within one
  // stage the percentage must never move backwards. A stage change resets it.
  const lastStageRef = useRef<string | null>(null);
  const lastPctRef = useRef<number>(-1);

  // Keep the latest onComplete without re-subscribing the event listeners.
  const onCompleteRef = useRef(onComplete);
  useEffect(() => {
    onCompleteRef.current = onComplete;
  });

  const resetProgressGuard = useCallback((stageValue: string | null, pct: number) => {
    lastStageRef.current = stageValue;
    lastPctRef.current = pct;
  }, []);

  // Subscribe to diarization events for this meeting while the hook is mounted,
  // and rehydrate from the backend's per-meeting run registry (WS3.1) so a
  // remount mid-run picks the live state back up.
  useEffect(() => {
    if (!meetingId) return;

    let cancelled = false;
    invoke<DiarizationStatusPayload | null>('api_diarization_status', {
      meetingId,
    })
      .then((status) => {
        if (cancelled || !status?.running) return;
        setIsRunning(true);
        setStage(status.stage);
        setProgressPct(status.progressPct > 0 ? status.progressPct : null);
        resetProgressGuard(status.stage, status.progressPct);
      })
      .catch(() => {
        // Status is advisory; live events still drive the state below.
      });

    const matches = (eventMeetingId?: string) =>
      // Accept events for this meeting; model-download progress carries no
      // meeting_id, so accept those too (they only fire while we're running).
      eventMeetingId === undefined || eventMeetingId === meetingId;

    const disposeProgress = safeListen<DiarizationProgressPayload>(
      'diarization-progress',
      (event) => {
        if (!matches(event.payload.meeting_id)) return;
        const nextStage = event.payload.stage;
        const pct =
          typeof event.payload.pct === 'number' ? event.payload.pct : null;

        if (nextStage === lastStageRef.current && pct !== null) {
          // Same stage: ignore a percentage that would move backwards.
          if (pct < lastPctRef.current) return;
          lastPctRef.current = pct;
        } else if (nextStage !== lastStageRef.current) {
          // New stage: the ramp legitimately resets.
          resetProgressGuard(nextStage, pct ?? -1);
        }

        // A progress event explicitly for this meeting means a pass is live —
        // reflect that even if the run was started elsewhere (another surface,
        // or before a remount). Meeting-less (model-download) events don't
        // prove a run, so they only update the stage text.
        if (event.payload.meeting_id === meetingId) setIsRunning(true);
        setStage(nextStage);
        setProgressPct(pct);
      },
    );

    const disposeComplete = safeListen<DiarizationCompletePayload>(
      'diarization-complete',
      (event) => {
        if (event.payload.meeting_id !== meetingId) return;
        setIsRunning(false);
        setStage(null);
        setProgressPct(null);
        resetProgressGuard(null, -1);
        const count = event.payload.speaker_count;
        const found =
          count === 1 ? '1 speaker found.' : `${count} speakers found.`;
        // Surface the basis when we seeded an estimate from the calendar, so the
        // user understands why the count is constrained (specs/0011 calendar-seed).
        const seeded = event.payload.seededSpeakerCount;
        let basis = '';
        if (
          event.payload.speakerCountSource === 'calendar' &&
          typeof seeded === 'number'
        ) {
          basis = ` Estimated ${seeded} speakers from calendar.`;
        }
        toast.success('Speakers identified', {
          description: `${found}${basis}`,
        });
        void Promise.resolve(onCompleteRef.current?.());
      },
    );

    const disposeError = safeListen<DiarizationErrorPayload>(
      'diarization-error',
      (event) => {
        if (event.payload.meeting_id !== meetingId) return;
        setIsRunning(false);
        setStage(null);
        setProgressPct(null);
        resetProgressGuard(null, -1);
        toast.error('Could not identify speakers', {
          description: event.payload.error || 'Diarization failed.',
        });
      },
    );

    return () => {
      cancelled = true;
      disposeProgress();
      disposeComplete();
      disposeError();
    };
  }, [meetingId, resetProgressGuard]);

  const identifySpeakers = useCallback(async () => {
    if (!meetingId || isRunning) return;

    setIsRunning(true);
    setStage('preparing');
    setProgressPct(null);
    resetProgressGuard('preparing', -1);

    try {
      // 1. Ensure models are present; download on demand (~35 MB, one-time).
      const present = await invoke<boolean>('api_diarization_models_present');
      if (!present) {
        setStage('downloading models (~35 MB, one-time)');
        toast.info('Downloading speaker model', {
          description:
            'A one-time ~35 MB download is needed before the first run. This may take a moment.',
        });
        await invoke('api_download_diarization_models');
      }

      // 2. Start the background pass. Completion/errors arrive via events.
      setStage('starting');
      const start = await invoke<DiarizeStartResult>('api_diarize_meeting', {
        meetingId,
      });
      if (start?.alreadyRunning) {
        // WS3.1: a pass is already live for this meeting — attach, don't restart
        // (and don't treat it as a failure). Pull the current stage/pct so the
        // button reflects the real run instead of sitting at "starting".
        const status = await invoke<DiarizationStatusPayload | null>(
          'api_diarization_status',
          { meetingId },
        ).catch(() => null);
        if (status?.running) {
          setStage(status.stage);
          setProgressPct(status.progressPct > 0 ? status.progressPct : null);
          resetProgressGuard(status.stage, status.progressPct);
        }
      }
      // isRunning stays true until diarization-complete / diarization-error.
    } catch (error) {
      console.error('Failed to start diarization:', error);
      setIsRunning(false);
      setStage(null);
      setProgressPct(null);
      resetProgressGuard(null, -1);
      toast.error('Could not identify speakers', {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  }, [meetingId, isRunning, resetProgressGuard]);

  return { isRunning, stage, progressPct, identifySpeakers };
}
