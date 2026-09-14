/**
 * Request the FULL two-part recording stop (specs/0024 WS1.1).
 *
 * A complete stop is two steps: the backend `stop_recording` command (stops the
 * Core Audio tap, flushes the WAV) followed by post-stop processing/save
 * (`handleRecordingStop(true)`). That combined flow only lives on `/record`
 * (in `useRecordingStop`, driven by `runGlobalStop` in `record/page.tsx`).
 *
 * Callers that are NOT on `/record` (the global recording bar, the Zoom
 * auto-stop) must therefore trigger that on-page flow rather than calling
 * `window.handleRecordingStop` directly — the latter only runs post-processing
 * and leaves the audio tap running (which produced the lingering recording, the
 * spurious second meeting on manual stop, and the inflated duration).
 *
 * This sets a flag the record page consumes on mount AND dispatches a live event,
 * so it works whether `/record` is already mounted or is about to mount from this
 * navigation: the flag wins the race when the page isn't mounted yet; the event
 * fires the stop when it already is. Mirrors the `autoStartRecording` pattern.
 */
export function requestFullRecordingStop(navigate: (path: string) => void): void {
  sessionStorage.setItem('stopRecordingOnLoad', 'true');
  navigate('/record');
  window.dispatchEvent(new CustomEvent('stop-recording-from-global-bar'));
}

// ── Stop-result transport (specs/0037 review-2, FIX C) ─────────────────────────────
//
// The `stop_recording` command RETURNS the same payload it emits as `recording-stopped`
// ({ folder_path, meeting_name, resumed, prior_audio_duration_seconds }). The return
// value is the race-free primary transport: the EVENT is emitted only after
// stop_and_save (multi-segment stops run several blocking ffmpeg concats), so on slow
// stops it can arrive after the save path's bounded wait — a late event used to read
// resumed=false from sessionStorage and save a resumed session into a DUPLICATE meeting.
// The invoke's resolved value has no such race: it exists the moment the stop call
// returns, before the save path ever runs.
//
// `recordingService.stopRecording` stashes the normalized result here; the save path
// (useRecordingStop.handleRecordingStop) consumes it — read-and-clear — as its primary
// source, falling back to the event/sessionStorage keys only when no invoke result
// exists (tray-initiated stops run the command in Rust; a reload mid-stop loses memory).

/** Normalized `stop_recording` return payload (backend field names, snake_case). */
export interface StopRecordingResult {
  folder_path: string | null;
  meeting_name: string | null;
  resumed: boolean;
  prior_audio_duration_seconds: number;
  /** v1.6.1: the recording's AUTHORITATIVE meeting id (set at start, from folder metadata).
   * The save path attaches transcripts to this, immune to current-meeting drift when the
   * user opens another meeting while recording. Null on older backends → fall back. */
  meeting_id: string | null;
}

let lastStopResult: StopRecordingResult | null = null;

/**
 * Normalize + stash the `stop_recording` invoke's resolved value. Type-tolerant by
 * design: a backend that predates the return-value contract resolves with undefined/null
 * — that records nothing and the save path falls back to the event transport.
 * Returns the normalized result (or null when the payload was unusable).
 */
export function recordStopRecordingResult(raw: unknown): StopRecordingResult | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  lastStopResult = {
    folder_path: typeof r.folder_path === 'string' && r.folder_path ? r.folder_path : null,
    meeting_name: typeof r.meeting_name === 'string' && r.meeting_name ? r.meeting_name : null,
    resumed: r.resumed === true,
    prior_audio_duration_seconds:
      typeof r.prior_audio_duration_seconds === 'number' &&
      Number.isFinite(r.prior_audio_duration_seconds)
        ? r.prior_audio_duration_seconds
        : 0,
    meeting_id: typeof r.meeting_id === 'string' && r.meeting_id ? r.meeting_id : null,
  };
  return lastStopResult;
}

/**
 * Read-and-clear the stashed stop result. Consumed once, at the top of the save flow
 * (before any await), so a subsequent recording start's stale-state cleanup can never
 * wipe it out from under an in-flight save.
 */
export function consumeStopRecordingResult(): StopRecordingResult | null {
  const result = lastStopResult;
  lastStopResult = null;
  return result;
}

/** Drop any stashed stop result (called when a NEW recording starts, so a stale result
 * from an errored/abandoned prior save can never misroute the next session's stop). */
export function clearStopRecordingResult(): void {
  lastStopResult = null;
}
