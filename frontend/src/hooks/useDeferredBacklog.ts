/**
 * Deferred-backlog controller (deferred-processing UX spec §0045).
 *
 * Mounted once at app level (its UI is the transport rail's queue), this hook drives the
 * per-meeting backlog state machine (`@/lib/deferred-backlog`). It has NO dependency on
 * recording state: deferred processing runs CONCURRENTLY with a live recording — starting
 * or stopping a recording never gates, cancels, or aborts a drain. The only things that
 * halt work are an explicit user `stop()` (via `stopRef`) and the per-step timeout ceilings.
 *
 * Behaviour:
 *  - On mount and on a return to AC power, it lists the deferred meetings, enqueues them,
 *    and — if on AC (or forced) — AUTO-STARTS a drain (no prompt). On battery it enqueues
 *    but leaves starting to the user. See `decideAutostart` (`@/lib/backlog-autostart`).
 *  - The drain is a self-refilling loop: each iteration pulls the next still-waiting item
 *    from the live state, so a meeting enqueued mid-run (a click while busy, or the
 *    `process-deferred-meeting-now` window event) is picked up rather than silently dropped.
 *  - Per meeting it runs the sequential pipeline: retranscribe (if sparse/deferred) →
 *    diarize (best-effort) → summarize → clear the meeting's processing_mode marker,
 *    dispatching a per-step status the UI surfaces read from one shared model.
 *  - `enqueueMeeting(id, { force })` adds a single meeting (per-meeting "Process now"
 *    button / mid-recording override) and starts a drain when forced or on AC.
 *
 * Every timer and listener is cancellable and cleaned up on unmount; repeated AC/battery
 * flapping never stacks duplicate timers (each schedule clears the prior one first).
 */

import { useCallback, useEffect, useReducer, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { fetchAllTranscripts } from '@/lib/fetch-all-transcripts';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useConfig } from '@/contexts/ConfigContext';
import { retranscriptionProviderFor } from '@/lib/deferred-transcription';
import { resolveSummaryLanguage } from '@/lib/resolve-summary-language';
import { DEFAULT_TEMPLATE_ID } from '@/hooks/meeting-details/useTemplates';
import type { Transcript } from '@/types';
import {
  backlogReducer,
  initialBacklogState,
  selectBacklogView,
  firstWaiting,
  enqueueItems,
  needsRetranscription,
  type BacklogView,
  type DeferredMeeting,
} from '@/lib/deferred-backlog';
import { decideAutostart, AUTOSTART_DEBOUNCE_MS } from '@/lib/backlog-autostart';
import type { HandoffOutcome } from '@/lib/stop-processing-handoff';

interface PowerState {
  onBattery: boolean;
}
interface PowerSourceChangedPayload {
  onBattery: boolean;
}

// Per-step ceilings so a backend that emits neither a complete NOR an error event (e.g.
// a diarization already-running guard, or a summary that never reaches a terminal status)
// can't wedge the sequential drain forever with the banner spinning. Generous — they're a
// safety net, not the expected path.
const RETRANSCRIPTION_TIMEOUT_MS = 30 * 60_000; // 30 min
const SUMMARY_TIMEOUT_MS = 30 * 60_000; // 30 min
const DIARIZATION_TIMEOUT_MS = 15 * 60_000; // 15 min
/** How often a wait re-checks the abort predicate (explicit user stop). */
const WAIT_ABORT_POLL_MS = 2_000;

/** Outcome of a bounded, abortable wait for a meeting-scoped completion/error event. */
type WaitResult = 'complete' | 'error' | 'timeout' | 'aborted';

export interface UseDeferredBacklogReturn {
  view: BacklogView;
  /**
   * Add a meeting to the backlog and (on AC, or forced) start draining. Resolves with
   * whether the backlog took ownership — spec 0051 WS2, so the stop path can fall back
   * instead of assuming a fire-and-forget dispatch landed.
   */
  enqueueMeeting: (meetingId: string, opts?: { force?: boolean }) => Promise<HandoffOutcome>;
  /** Signal the drain loop to stop after the current step. */
  stop: () => void;
  /** Drop completed/errored items from the list. */
  dismissDone: () => void;
  /** Start draining now (manual button), regardless of power. */
  startNow: () => void;
}

/**
 * Await a meeting-scoped completion/error event pair around a start call, bounded by a
 * timeout and cancellable via `shouldAbort` (polled). Resolves to the FIRST of: the
 * complete event ('complete'), the error event ('error'), the ceiling ('timeout'), or an
 * abort ('aborted') — so a stuck backend or an explicit stop can never wedge the wait.
 */
async function awaitMeetingCompletion(
  meetingId: string,
  completeEvent: string,
  errorEvent: string,
  start: () => Promise<unknown>,
  { timeoutMs, shouldAbort }: { timeoutMs: number; shouldAbort: () => boolean },
): Promise<WaitResult> {
  let settle: (result: WaitResult) => void = () => {};
  const done = new Promise<WaitResult>((resolve) => {
    settle = resolve;
  });
  const unlisteners: Array<() => void> = [];
  let timeoutTimer: ReturnType<typeof setTimeout> | null = null;
  let abortPoll: ReturnType<typeof setInterval> | null = null;
  try {
    // Register both listeners BEFORE starting so a fast completion can't be missed.
    unlisteners.push(
      await listen<{ meeting_id: string }>(completeEvent, (event) => {
        if (event.payload.meeting_id === meetingId) settle('complete');
      }),
    );
    unlisteners.push(
      await listen<{ meeting_id: string }>(errorEvent, (event) => {
        if (event.payload.meeting_id === meetingId) settle('error');
      }),
    );
    timeoutTimer = setTimeout(() => settle('timeout'), timeoutMs);
    abortPoll = setInterval(() => {
      if (shouldAbort()) settle('aborted');
    }, WAIT_ABORT_POLL_MS);
    await start();
    return await done;
  } catch (error) {
    console.error(`[deferred-backlog] ${completeEvent} wait failed:`, error);
    return 'error';
  } finally {
    unlisteners.forEach((unlisten) => unlisten());
    if (timeoutTimer) clearTimeout(timeoutTimer);
    if (abortPoll) clearInterval(abortPoll);
  }
}

/** Join transcripts into the summary payload (mirrors useSummaryGeneration.buildSummaryTranscriptPayload). */
function buildSummaryTranscriptPayload(allTranscripts: Transcript[]) {
  const formatTime = (seconds: number | undefined, fallbackTimestamp: string): string => {
    if (seconds === undefined) return fallbackTimestamp;
    const totalSecs = Math.floor(seconds);
    const mins = Math.floor(totalSecs / 60);
    const secs = totalSecs % 60;
    return `[${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}]`;
  };
  return {
    transcriptText: allTranscripts
      .map((t) => `${formatTime(t.audio_start_time, t.timestamp)} ${t.text}`)
      .join('\n'),
    transcriptTexts: allTranscripts.map((t) => t.text),
  };
}

export function useDeferredBacklog(): UseDeferredBacklogReturn {
  const [state, dispatch] = useReducer(backlogReducer, initialBacklogState);
  const { startSummaryPolling, stopSummaryPolling } = useSidebar();
  const { transcriptModelConfig } = useConfig();

  // Latest-value refs so timers/async loops never close over stale state.
  const transcriptProviderRef = useRef(transcriptModelConfig?.provider);
  const startSummaryPollingRef = useRef(startSummaryPolling);
  const stopSummaryPollingRef = useRef(stopSummaryPolling);
  const processingRef = useRef(false);
  const stopRef = useRef(false);
  const itemsRef = useRef(state.items);
  const checkTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    itemsRef.current = state.items;
  }, [state.items]);
  useEffect(() => {
    transcriptProviderRef.current = transcriptModelConfig?.provider;
  }, [transcriptModelConfig?.provider]);
  useEffect(() => {
    startSummaryPollingRef.current = startSummaryPolling;
    stopSummaryPollingRef.current = stopSummaryPolling;
  }, [startSummaryPolling, stopSummaryPolling]);

  // --- per-meeting pipeline steps -----------------------------------------

  const runRetranscription = useCallback(async (m: DeferredMeeting): Promise<WaitResult> => {
    return awaitMeetingCompletion(
      m.id,
      'retranscription-complete',
      'retranscription-error',
      () =>
        invoke('start_retranscription_command', {
          meetingId: m.id,
          meetingFolderPath: m.folderPath,
          language: null,
          model: null,
          provider: retranscriptionProviderFor(transcriptProviderRef.current),
        }),
      { timeoutMs: RETRANSCRIPTION_TIMEOUT_MS, shouldAbort: () => stopRef.current },
    );
  }, []);

  // Best-effort. Returns 'aborted' if the user stopped mid-wait, else 'done' (a timeout/
  // error just skips diarization — it never blocks the drain).
  const runDiarization = useCallback(async (m: DeferredMeeting): Promise<'aborted' | 'done'> => {
    // Same gating as useRecordingStop's auto-diarization: opt-in AND models already
    // present (never auto-download mid-flow).
    try {
      const [enabled, present] = await Promise.all([
        invoke<boolean>('api_get_diarization_enabled'),
        invoke<boolean>('api_diarization_models_present'),
      ]);
      if (!enabled || !present) return 'done';
      const result = await awaitMeetingCompletion(
        m.id,
        'diarization-complete',
        'diarization-error',
        () => invoke('api_diarize_meeting', { meetingId: m.id }),
        { timeoutMs: DIARIZATION_TIMEOUT_MS, shouldAbort: () => stopRef.current },
      );
      return result === 'aborted' ? 'aborted' : 'done';
    } catch (error) {
      console.warn('[deferred-backlog] diarization skipped:', error);
      return 'done';
    }
  }, []);

  // Summary outcome:
  //  - 'done'                → generated.
  //  - 'skipped-empty'       → no transcript to summarize.
  //  - 'skipped-no-provider' → no summary provider configured.
  //  - 'error'               → provider configured but generation failed/timed out.
  //  - 'aborted'             → the user stopped mid-wait.
  type SummaryOutcome = 'done' | 'skipped-empty' | 'skipped-no-provider' | 'error' | 'aborted';
  const runSummary = useCallback(async (m: DeferredMeeting): Promise<SummaryOutcome> => {
    try {
      const allTranscripts = await fetchAllTranscripts(m.id);
      if (allTranscripts.length === 0) return 'skipped-empty';

      // "No provider configured" mirrors the meeting-details auto-summary gate:
      // the DB model config has no saved model.
      const modelConfig = await invoke<{ provider?: string; model?: string } | null>(
        'api_get_model_config',
      );
      if (!modelConfig?.model || !modelConfig.provider) return 'skipped-no-provider';

      const { transcriptText, transcriptTexts } = buildSummaryTranscriptPayload(allTranscripts);
      let templateId = DEFAULT_TEMPLATE_ID;
      try {
        const persisted = await invoke<string | null>('api_get_meeting_template', {
          meetingId: m.id,
        });
        if (persisted) templateId = persisted;
      } catch (error) {
        console.warn('[deferred-backlog] template load failed, using default:', error);
      }
      const summaryLanguage = await resolveSummaryLanguage(m.id, transcriptTexts);

      // Deliberately NOT `background: true` (specs/0063 W3 Task 6b). The drain already has a
      // queue row — the dispatch above sets this item to `summarizing`, which `queue-view.ts`
      // renders — and backlog rows and LLM rows are concatenated with no de-duplication, so
      // asking for a background registration here would list the same meeting twice.
      const result = await invoke<{ process_id: string }>('api_process_transcript', {
        text: transcriptText,
        model: modelConfig.provider,
        modelName: modelConfig.model,
        meetingId: m.id,
        chunkSize: 40000,
        overlap: 1000,
        customPrompt: '',
        templateId,
        summaryLanguage,
      });

      // Await completion via the SAME startSummaryPolling context useSummaryGeneration
      // uses (one interval per meeting id; terminal statuses stop it). Bounded by a
      // ceiling and cancellable so a summary that never reaches terminal — or an explicit
      // user stop — can't wedge the drain.
      return await new Promise<SummaryOutcome>((resolve) => {
        let settled = false;
        let guard: ReturnType<typeof setInterval> | null = null;
        const finish = (outcome: SummaryOutcome) => {
          if (settled) return;
          settled = true;
          if (guard) clearInterval(guard);
          resolve(outcome);
        };
        startSummaryPollingRef.current(m.id, result.process_id, (poll: { status: string }) => {
          if (poll.status === 'completed' || poll.status === 'idle') finish('done');
          else if (
            poll.status === 'error' ||
            poll.status === 'failed' ||
            poll.status === 'cancelled'
          ) {
            finish('error');
          }
        });
        const startedAt = Date.now();
        guard = setInterval(() => {
          if (stopRef.current) {
            stopSummaryPollingRef.current(m.id);
            finish('aborted');
          } else if (Date.now() - startedAt > SUMMARY_TIMEOUT_MS) {
            stopSummaryPollingRef.current(m.id);
            finish('error');
          }
        }, WAIT_ABORT_POLL_MS);
      });
    } catch (error) {
      console.error('[deferred-backlog] summary failed:', error);
      return 'error';
    }
  }, []);

  // Per-meeting outcome, returned to the drain loop so the failure toast reads it directly
  // (never a stale itemsRef re-read):
  //  - 'done'       → transcript complete + marker cleared (summary may have been skipped).
  //  - 'error'      → summary failed for a configured provider (marker still cleared).
  //  - 'incomplete' → transcription failed/timed out; marker stays, retried on a later pass.
  //  - 'aborted'    → user stop mid-pipeline; state left as-is.
  type MeetingOutcome = 'done' | 'error' | 'incomplete' | 'aborted';
  const processMeeting = useCallback(
    async (m: DeferredMeeting): Promise<MeetingOutcome> => {
      const explicitlyDeferred = true; // backlog meetings are all defer-marked or sparse
      // 1. Retranscribe when sparse or explicitly deferred.
      if (needsRetranscription(m.transcriptCount, explicitlyDeferred)) {
        dispatch({ type: 'set-status', meetingId: m.id, status: 'transcribing' });
        const status = await runRetranscription(m);
        if (status === 'aborted') return 'aborted'; // user stop; leave it as-is
        if (status !== 'complete') {
          dispatch({ type: 'set-status', meetingId: m.id, status: 'error' });
          return 'incomplete'; // marker stays on the backend; retried on a later pass
        }
      }
      if (stopRef.current) return 'aborted';

      // 2. Diarize (best-effort — a timeout/error just skips it).
      dispatch({ type: 'set-status', meetingId: m.id, status: 'diarizing' });
      if ((await runDiarization(m)) === 'aborted') return 'aborted';
      if (stopRef.current) return 'aborted';

      // 3. Summarize.
      dispatch({ type: 'set-status', meetingId: m.id, status: 'summarizing' });
      const summary = await runSummary(m);
      if (summary === 'aborted') return 'aborted';

      // 4. Clear the pending marker.
      try {
        await invoke('api_set_meeting_processing_mode', { meetingId: m.id, mode: null });
      } catch (error) {
        console.warn('[deferred-backlog] failed to clear processing mode:', error);
      }
      dispatch({
        type: 'set-status',
        meetingId: m.id,
        status: summary === 'error' ? 'error' : 'done',
      });
      return summary === 'error' ? 'error' : 'done';
    },
    [runRetranscription, runDiarization, runSummary],
  );

  const drain = useCallback(async (): Promise<void> => {
    if (processingRef.current) return; // a drain is already running; it will pick up new items
    processingRef.current = true;
    stopRef.current = false;
    dispatch({ type: 'processing-started' });
    const failedTitles: string[] = [];
    try {
      for (;;) {
        if (stopRef.current) break;
        const next = firstWaiting({ items: itemsRef.current, processing: false });
        if (!next) break;
        const outcome = await processMeeting(next.meeting);
        if (outcome === 'error' || outcome === 'incomplete') failedTitles.push(next.meeting.title);
      }
    } finally {
      processingRef.current = false;
      dispatch({ type: 'processing-stopped' });
      // A user Stop leaves the active meeting mid-flight; return it to 'waiting' so it's
      // re-runnable (Process all / auto-start) rather than stuck in a non-terminal status.
      // No-op on normal completion (the last item is already done/error). Does NOT restart
      // the drain — Stop holds until the user or a later power/mount trigger starts it again.
      dispatch({ type: 'requeue-active' });
    }
    if (failedTitles.length > 0) {
      const who =
        failedTitles.length === 1 ? `'${failedTitles[0]}'` : `${failedTitles.length} meetings`;
      toast.warning(`Processing incomplete for ${who}`, {
        description: 'Open the meeting to retry, or it will retry on the next power connection.',
      });
    }
  }, [processMeeting]);

  // --- trigger plumbing ----------------------------------------------------

  const clearCheckTimer = useCallback(() => {
    if (checkTimerRef.current) {
      clearTimeout(checkTimerRef.current);
      checkTimerRef.current = null;
    }
  }, []);

  /** List deferred meetings, enqueue them, and auto-start if on AC (or forced). */
  const refreshAndMaybeStart = useCallback(
    async (opts?: { force?: boolean }) => {
      let onBattery = true;
      try {
        const power = await invoke<PowerState>('api_get_power_state');
        onBattery = power.onBattery;
      } catch (error) {
        console.warn('[deferred-backlog] power-state check failed:', error);
        onBattery = true;
      }
      let meetings: DeferredMeeting[] = [];
      try {
        meetings = await invoke<DeferredMeeting[]>('api_list_deferred_meetings');
      } catch (error) {
        console.warn('[deferred-backlog] deferred-meeting list failed:', error);
        return;
      }
      if (meetings.length === 0 && !opts?.force) return;
      // Seed the ref with the SAME merge the reducer will commit, so a drain kicked off on
      // the next line sees the new items before React commits the dispatch.
      itemsRef.current = enqueueItems(itemsRef.current, meetings);
      dispatch({ type: 'enqueue', meetings });
      // decideAutostart only needs "is there a backlog?" — we already returned early on 0.
      if (
        opts?.force ||
        decideAutostart({
          backlogCount: meetings.length,
          onBattery,
          isProcessing: processingRef.current,
        })
      ) {
        void drain();
      }
    },
    [drain],
  );

  const scheduleRefresh = useCallback(
    (delayMs: number, opts?: { force?: boolean }) => {
      clearCheckTimer();
      checkTimerRef.current = setTimeout(() => {
        checkTimerRef.current = null;
        void refreshAndMaybeStart(opts);
      }, delayMs);
    },
    [clearCheckTimer, refreshAndMaybeStart],
  );

  // Single-meeting enqueue (per-meeting "Process now" button / mid-recording override).
  // Fetches the meeting's folder/title/transcript-count, enqueues, then starts a drain when
  // forced or on AC.
  const enqueueMeeting = useCallback(
    async (meetingId: string, opts?: { force?: boolean }): Promise<HandoffOutcome> => {
      // `api_get_meeting_metadata`, NOT `api_get_meeting`.
      //
      // This guard used to read `folder_path` off `api_get_meeting`, whose `MeetingDetails`
      // has no folder-path field at all — so it refused EVERY meeting. The stop handoff for
      // a live->defer->live recording therefore always told the user to process it by hand,
      // and the "Process now" button always returned here before dispatching or draining,
      // which is precisely "clicking it doesn't seem to do anything".
      //
      // `MeetingMetadata` carries `folder_path: Option<String>` (snake_case, as this code
      // always expected) and costs less than `api_get_meeting`, which also serializes every
      // transcript. The refresh path never hit this because it reads
      // `api_list_deferred_meetings`, whose DTO is camelCase and matches its TS type.
      const meeting = await invoke<{ folder_path?: string; title?: string }>(
        'api_get_meeting_metadata',
        { meetingId },
      ).catch(() => null);
      const folderPath = meeting?.folder_path;
      if (!folderPath) {
        // spec 0051 WS2: this used to return silently, dropping the meeting on the
        // floor with no trace. A notes-only meeting legitimately lands here.
        console.warn('[deferred-backlog] no folder_path for', meetingId);
        return { accepted: false, reason: 'no-folder-path' };
      }
      let transcriptCount = 0;
      try {
        const page = await invoke<{ total_count: number }>('api_get_meeting_transcripts', {
          meetingId,
          limit: 1,
          offset: 0,
        });
        transcriptCount = page.total_count;
      } catch (error) {
        console.warn('[deferred-backlog] transcript count failed:', error);
      }
      const m: DeferredMeeting = {
        id: meetingId,
        title: meeting?.title ?? 'meeting',
        folderPath,
        transcriptCount,
      };
      // Seed the ref synchronously (same merge the reducer commits) so an immediate drain
      // sees this meeting before React commits the dispatch.
      itemsRef.current = enqueueItems(itemsRef.current, [m]);
      dispatch({ type: 'enqueue', meetings: [m] });
      if (opts?.force) {
        void drain();
        return { accepted: true };
      }
      let onBattery = true;
      try {
        onBattery = (await invoke<PowerState>('api_get_power_state')).onBattery;
      } catch {
        /* default to not auto-starting */
      }
      if (decideAutostart({ backlogCount: 1, onBattery, isProcessing: processingRef.current })) {
        void drain();
      }
      // Enqueued either way — a drain that has not started yet still owns the meeting.
      return { accepted: true };
    },
    [drain],
  );

  const enqueueMeetingRef = useRef(enqueueMeeting);
  useEffect(() => {
    enqueueMeetingRef.current = enqueueMeeting;
  }, [enqueueMeeting]);

  // On mount: check for a backlog and auto-start if on AC (short debounce).
  useEffect(() => {
    scheduleRefresh(AUTOSTART_DEBOUNCE_MS);
    return clearCheckTimer;
  }, [scheduleRefresh, clearCheckTimer]);

  // Power→AC: re-check + auto-start (debounced). Power→battery: cancel a pending check.
  useEffect(() => {
    return safeListen<PowerSourceChangedPayload>('power-source-changed', (event) => {
      if (event.payload.onBattery) {
        clearCheckTimer();
      } else {
        scheduleRefresh(AUTOSTART_DEBOUNCE_MS);
      }
    });
  }, [scheduleRefresh, clearCheckTimer]);

  // Immediate single-meeting request (per-meeting "Process now" button / mid-recording override).
  // Enqueue + start now, regardless of power — the user asked for it explicitly.
  useEffect(() => {
    const handler = (event: Event) => {
      const meetingId = (event as CustomEvent<{ meetingId?: string }>).detail?.meetingId;
      if (meetingId) void enqueueMeetingRef.current(meetingId, { force: true });
    };
    window.addEventListener('process-deferred-meeting-now', handler);
    return () => window.removeEventListener('process-deferred-meeting-now', handler);
  }, []);

  const stop = useCallback(() => {
    stopRef.current = true;
  }, []);
  const dismissDone = useCallback(() => {
    dispatch({ type: 'clear-done' });
  }, []);
  const startNow = useCallback(() => {
    void drain();
  }, [drain]);

  return {
    view: selectBacklogView(state),
    enqueueMeeting: (id, opts) => enqueueMeeting(id, opts),
    stop,
    dismissDone,
    startNow,
  };
}
