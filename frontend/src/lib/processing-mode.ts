/**
 * Low-power-mode session bookkeeping (spec low-power-mode, §§3,5).
 *
 * A recording session can start in "defer" mode (live transcription off, e.g.
 * on battery) and be overridden to "live" mid-meeting, or vice versa. This
 * module tracks — purely, with no Tauri dependency — whether the CURRENT
 * recording session started deferred, and decides what to do about it once
 * the session stops:
 *
 *  - `markSessionDeferred` / `sessionStartedDeferred`: a sessionStorage-backed
 *    flag set at recording start (`useProcessingMode`, on the start-time
 *    `processing-mode-changed` emit) and cleared once stop-time bookkeeping
 *    has run (`useRecordingStop`). sessionStorage (not memory) so it survives
 *    remounts within the same app session — the recording UI can unmount and
 *    remount between start and stop.
 *  - `stopAction`: pure decision of what the stop path should do, given
 *    whether the session started deferred and any mid-meeting processing-mode
 *    override recorded in the backend (`api_get_meeting_processing_mode`).
 */

export const SESSION_DEFERRED_KEY = 'recording_session_started_deferred';
export const SESSION_EVER_DEFERRED_KEY = 'recording_session_ever_deferred';

/**
 * T8 invariant on both sessionStorage flags below: they describe the CURRENT
 * recording session only. After an abnormal stop (page reload / crash mid-stop)
 * they can be left stale, but that is harmless — the next recording's start-time
 * `processing-mode-changed` event always overwrites them (a live start clears
 * both via `markSessionDeferred(false)`; a deferred start re-sets them) BEFORE
 * that session's stop path ever reads them back. So a reader in `useRecordingStop`
 * only ever sees flags written by the session it is stopping.
 */

/**
 * Record whether the in-progress recording session started deferred (live
 * transcription off). Resilient to sessionStorage being unavailable (private
 * browsing, storage quota, etc.) — a failure here must never break the
 * recording flow. Clearing (`false`) also clears the "ever deferred" flag, so a
 * fresh live-started session begins with both flags reset.
 */
export function markSessionDeferred(deferred: boolean): void {
  try {
    if (deferred) {
      sessionStorage.setItem(SESSION_DEFERRED_KEY, 'true');
    } else {
      sessionStorage.removeItem(SESSION_DEFERRED_KEY);
      sessionStorage.removeItem(SESSION_EVER_DEFERRED_KEY);
    }
  } catch {
    /* best-effort; ignore storage errors */
  }
}

/** Did the in-progress (or just-ended) recording session start deferred? */
export function sessionStartedDeferred(): boolean {
  try {
    return sessionStorage.getItem(SESSION_DEFERRED_KEY) === 'true';
  } catch {
    return false;
  }
}

/**
 * Record that the in-progress session has been in deferred mode at ANY point —
 * whether it started deferred or was flipped to defer mid-recording. This is
 * sticky for the session (a subsequent go-live does NOT clear it); only a fresh
 * session start (`markSessionDeferred(false)`) resets it. It's what lets the
 * live→defer→live square of the matrix still trigger a repair pass at stop.
 */
export function markSessionEverDeferred(): void {
  try {
    sessionStorage.setItem(SESSION_EVER_DEFERRED_KEY, 'true');
  } catch {
    /* best-effort; ignore storage errors */
  }
}

/** Was the in-progress (or just-ended) session ever in deferred mode? */
export function sessionEverDeferred(): boolean {
  try {
    return sessionStorage.getItem(SESSION_EVER_DEFERRED_KEY) === 'true';
  } catch {
    return false;
  }
}

/** What the stop path should do with this meeting's processing mode. */
export type StopAction = 'process-now' | 'mark-defer' | 'none';

/**
 * Decide the stop-time action for a meeting, given:
 *  - `startedDeferred`: did the session START with live transcription off?
 *  - `everDeferred`: was the session in deferred mode at ANY point (started
 *    deferred, or flipped to defer mid-recording)? A live→defer→live session
 *    has `startedDeferred=false` but `everDeferred=true` — its deferred span is
 *    a permanent gap and the re-attached VAD restarts timestamps near 0, so it
 *    needs the same repair pass as a session that started deferred.
 *  - `overrideMode`: the last mid-meeting override recorded via
 *    `api_set_meeting_processing_mode` ('live' | 'defer' | null — none set).
 *
 * - `'process-now'`: was deferred at some point but ends overridden live — run
 *   the full uniform pass (transcription + diarization) right away to repair the
 *   deferred span and re-order the re-attached VAD's segments.
 * - `'mark-defer'`: the session ends still deferred (was deferred at some point
 *   and not overridden live), OR the user explicitly deferred mid-meeting — mark
 *   it so the backlog picks it up and retention doesn't sweep it. Note this now
 *   also covers a live-STARTED session that was deferred mid-way and never
 *   overridden back to live: its tail is missing, so it must be repaired.
 * - `'none'`: a normal live meeting with no deferred involvement.
 */
export function stopAction(
  startedDeferred: boolean,
  everDeferred: boolean,
  overrideMode: string | null,
): StopAction {
  const wasDeferred = startedDeferred || everDeferred;
  if (overrideMode === 'live' && wasDeferred) return 'process-now';
  if ((wasDeferred && overrideMode !== 'live') || overrideMode === 'defer') {
    return 'mark-defer';
  }
  return 'none';
}

/**
 * Should the stop path kick off best-effort auto-diarization for this meeting?
 * Only for a normal live meeting ('none'). 'process-now' already includes
 * diarization in its full pass; 'mark-defer' meetings must run NOTHING at stop
 * (1.10 feedback) — the deferred backlog (AC return or the manual "Process now"
 * button) diarizes them exactly once as part of its pipeline.
 */
export function shouldAutoDiarizeAtStop(action: StopAction): boolean {
  return action === 'none';
}

/**
 * Effective "is live transcription on" value for display purposes, once a
 * `useProcessingMode()` consumer wants to show something before the first
 * `processing-mode-changed` event has arrived. Prefer the live event value;
 * fall back to a caller-supplied stored preference until it does.
 */
export function effectiveLiveTranscription(
  eventValue: boolean | null,
  fallbackPref: boolean,
): boolean {
  return eventValue ?? fallbackPref;
}

/** Compact mode-chip label + battery-glyph decision (RecordingHeader). */
export interface ModeChipDisplay {
  label: 'Pause transcript' | 'Resume transcript';
  showBatteryGlyph: boolean;
}

/**
 * What the chip's click will DO — not what mode the session is in.
 *
 * It used to read `Live` / `Deferred`, i.e. the current state, while clicking *left* that
 * state. The owner read the word "Live" as "make it live", pressed it mid-meeting, and the
 * transcript stopped (2026-09-21; specs/0071). That is the only reading a reasonable person
 * takes from a button in a row where its two neighbours — Participants and the template
 * picker — both do the thing written on them.
 *
 * The state the label used to carry now lives on the transport rail's status line, which
 * already owns the session's state vocabulary ("On the reel" / "On hold" / "Mic muted") and
 * is on screen on every route.
 */
export function modeChipDisplay(
  live: boolean,
  onBattery: boolean | null,
): ModeChipDisplay {
  return {
    label: live ? 'Pause transcript' : 'Resume transcript',
    // Still the chip's business: it explains WHY the transcript is paused.
    showBatteryGlyph: !live && onBattery === true,
  };
}

/**
 * Which record-only empty-state variant TranscriptPanel should show, if any.
 * `null` means the normal (live) transcript view should render instead.
 *
 * - `'low-power-battery'`: this session is deferred specifically because it's
 *   running on battery (Low Power Mode) — offer a per-meeting "go live" button.
 * - `'live-transcription-off'`: deferred for any other reason (the global
 *   live-transcription setting is off) — the existing quiet explanation, no
 *   per-meeting override offered since there's nothing battery-specific to undo.
 */
export type EmptyStateVariant = 'low-power-battery' | 'live-transcription-off';

export function transcriptEmptyStateVariant(
  liveTranscriptionEnabled: boolean,
  hasTranscripts: boolean,
  onBattery: boolean | null,
): EmptyStateVariant | null {
  if (liveTranscriptionEnabled || hasTranscripts) return null;
  return onBattery === true ? 'low-power-battery' : 'live-transcription-off';
}

/**
 * A finished recording that has audio on disk but no transcript yet — i.e. recorded in
 * low-power/deferred mode and not processed. Drives the transcript empty state so it reads
 * "not processed yet" instead of the generic welcome copy (spec 0045 WS4).
 */
export function isUnprocessedRecording(input: {
  hasTranscripts: boolean;
  isDeferred: boolean;
  audioAvailable: boolean;
}): boolean {
  return !input.hasTranscripts && input.isDeferred && input.audioAvailable;
}
